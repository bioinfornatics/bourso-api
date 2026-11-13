use crate::outcome::{OutcomeOperation, OutcomeStage, OutcomeUnknown};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::{debug, info};

use crate::{
    account::{Account, AccountKind},
    client::config::Config,
};

use super::{get_trading_base_url, BoursoWebClient};

#[cfg(test)]
#[path = "order_tests.rs"]
mod order_tests;

impl BoursoWebClient {
    /// Place an order
    ///
    /// # Arguments
    ///
    /// * `side` - Order side (buy or sell); overrides any value in `order_data` or API prefill
    /// * `account` - Account to use. Must be a trading account
    /// * `symbol` - Symbol to trade
    /// * `quantity` - Nonzero quantity to trade; overrides `order_data` and API prefill
    /// * `order_data` - Other order options. If absent, use the API prefill. Explicit side and quantity always take precedence.
    ///
    /// A selected expiration date is preserved. If absent, the API prefill's
    /// `order_validity` is used (and remains absent if no default is supplied).
    /// Present expiration dates must be calendar-valid `YYYY-MM-DD` values;
    /// malformed values fail after preparation, before check or confirmation.
    /// No current-date, market-timezone, or maximum-validity policy is imposed here.
    ///
    /// Errors after final submission may downcast to [`OutcomeUnknown`]. Do not
    /// reissue automatically; dropping this future cannot return such an error.
    /// A returned tuple acknowledges only a submission response, not execution.
    /// Use [`Self::order_detailed`] to inspect the unverified provider state.
    ///
    /// # Returns
    /// Order ID and order price limit
    pub async fn order(
        &self,
        side: OrderSide,
        account: &Account,
        symbol: &str,
        quantity: usize,
        order_data: Option<OrderData>,
    ) -> Result<(String, Option<f64>)> {
        let result = self
            .order_detailed(side, account, symbol, quantity, order_data)
            .await?;
        Ok((result.order_id, result.order_price_limit))
    }

    /// Submit an order and retain the provider state without asserting execution.
    /// Nonempty acceptability messages require review; no warning classification
    /// or positive provider-state contract is assumed. Post-send failures remain
    /// [`OutcomeUnknown`]; callers must not automatically retry.
    pub async fn order_detailed(
        &self,
        side: OrderSide,
        account: &Account,
        symbol: &str,
        quantity: usize,
        order_data: Option<OrderData>,
    ) -> Result<OrderSubmission> {
        if account.kind != AccountKind::Trading {
            return Err(anyhow::anyhow!("Account is not a trading account"));
        }

        anyhow::ensure!(quantity > 0, "Order quantity must be greater than zero");

        let response = self.prepare(account, symbol).await?;

        debug!("Order preparation received");
        anyhow::ensure!(
            response.acceptability_messages.is_empty(),
            "Order preparation requires review of provider acceptability messages"
        );

        // Either the order data set by the user
        // or a prefilled data object fetched from Bourso API
        let mut order_data = match order_data {
            Some(data) => data,
            None => response.prefill_order_data.clone(),
        };

        let last_price = response.symbol.last_price;

        // Explicit intent is authoritative, even when the selected defaults disagree.
        order_data.order_quantity = Some(quantity);
        order_data.order_side = Some(side);

        if order_data.order_price_limit.is_none() && order_data.order_type == OrderKind::Limit {
            if order_data.order_amount.is_some() {
                // Use quoted market price or given user price
                order_data.order_price_limit = order_data.order_amount;
            } else {
                // Use the last price fetched
                order_data.order_price_limit = Some(last_price);
            }
        } // else TODO: other types of orders data definition

        if order_data.order_expiration_date.is_none() {
            // Only default an absent expiration; never replace the selected date with today.
            order_data.order_expiration_date = response.prefill_order_data.order_validity;
        }
        if let Some(date) = order_data.order_expiration_date.as_deref() {
            // Validate a date, not an instant: no clock/timezone or provider maximum is implied.
            let parsed = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").map_err(|_| {
                anyhow::anyhow!("Invalid order expiration date: expected YYYY-MM-DD")
            })?;
            anyhow::ensure!(
                date.len() == 10 && parsed.format("%Y-%m-%d").to_string() == date,
                "Invalid order expiration date: expected YYYY-MM-DD"
            );
        }

        order_data.resource_id = Some(response.resource_id);

        debug!("Order data prepared");

        let checked = self.check(&order_data).await?;
        anyhow::ensure!(
            checked
                .acceptability_messages
                .as_ref()
                .is_none_or(Vec::is_empty),
            "Order check requires review of provider acceptability messages"
        );
        let echo = &checked.check_order_data;
        anyhow::ensure!(
            echo.order_side == order_data.order_side
                && echo.order_quantity == order_data.order_quantity
                && echo.order_price_limit == order_data.order_price_limit
                && echo.order_type == order_data.order_type
                && echo.order_expiration_date == order_data.order_expiration_date
                && echo
                    .resource_id
                    .as_ref()
                    .is_none_or(|id| Some(id) == order_data.resource_id.as_ref()),
            "Order check intent differs from submitted intent; confirmation not sent"
        );

        let response = self
            .confirm(order_data.resource_id.as_ref().unwrap())
            .await?;

        info!(
            side = ?order_data.order_side,
            quantity = order_data.order_quantity,
            symbol,
            order_price_limit = order_data.order_price_limit,
            "Order submission response received; execution unverified"
        );

        Ok(OrderSubmission {
            order_id: response.order_id.clone(),
            order_price_limit: order_data.order_price_limit,
            status: OrderSubmissionStatus::SubmittedUnverified,
            provider_state: RedactedOrderProviderState(response),
        })
    }

    /// Prepare an order
    ///
    /// This will fetch trading data for the given symbol
    ///
    /// # Arguments
    ///
    /// * `account` - Account to use. Must be a trading account
    /// * `symbol` - Symbol to trade
    ///
    /// # Returns
    ///
    /// An order prepare response
    async fn prepare(&self, account: &Account, symbol: &str) -> Result<OrderPrepareResponse> {
        let url = get_order_prepare_url(&self.config, account, symbol)?;
        let response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(reqwest::Error::without_url)?;

        let status_code = response.status();

        let response = response.text().await.map_err(reqwest::Error::without_url)?;

        if status_code != 200 {
            return Err(anyhow::anyhow!(
                "Failed to get order prepare response, status code: {}",
                status_code
            ));
        }

        let response: OrderPrepareResponse = serde_json::from_str(&response).map_err(|e| {
            crate::client::safe_json_error("Failed to parse order prepare response", e)
        })?;

        Ok(response)
    }

    /// Check if an order is valid
    ///
    /// # Arguments
    ///
    /// * `data` - Order data to check
    ///
    /// # Returns
    ///
    /// An order check response
    async fn check(&self, data: &OrderData) -> Result<OrderCheckResponse> {
        let url = get_order_check_url(&self.config)?;
        let response = self
            .client
            .post(url)
            .header("Content-Type", "application/json")
            .body(serde_json::to_string(data)?)
            .send()
            .await
            .map_err(reqwest::Error::without_url)?;

        let status_code = response.status();

        let response = response.text().await.map_err(reqwest::Error::without_url)?;

        if status_code != 200 {
            return Err(anyhow::anyhow!(
                "Failed to get order check response, status code: {}",
                status_code
            ));
        }

        let response: OrderCheckResponse = serde_json::from_str(&response).map_err(|e| {
            crate::client::safe_json_error("Failed to parse order check response", e)
        })?;

        Ok(response)
    }

    /// Confirm an order
    ///
    /// # Arguments
    ///
    /// * `resource_id` - Resource ID of the order to confirm
    ///
    /// # Returns
    ///
    /// An order confirm response
    async fn confirm(&self, resource_id: &str) -> Result<OrderConfirmResponse> {
        let url = get_order_confirm_url(&self.config)?;
        // Local construction errors remain ordinary errors: nothing was attempted.
        let body = serde_json::to_string(&serde_json::json!({ "resourceId": resource_id }))?;
        let request = self
            .client
            .post(url)
            .header("Content-Type", "application/json")
            .body(body);
        let unknown = OutcomeUnknown::new(OutcomeOperation::Order, resource_id);
        // Once send is attempted, even a transport error cannot establish whether
        // the bank applied the operation. Deliberately do not retain raw causes.
        let response = request
            .send()
            .await
            .map_err(|_| unknown.at_stage(OutcomeStage::Sending))?;
        if response.status() != 201 {
            return Err(unknown.at_stage(OutcomeStage::HttpStatus).into());
        }
        let response = response
            .text()
            .await
            .map_err(|_| unknown.at_stage(OutcomeStage::ReadingResponse))?;
        let response: OrderConfirmResponse = serde_json::from_str(&response)
            .map_err(|_| unknown.at_stage(OutcomeStage::ParsingResponse))?;
        // A parsed 201 is a submission response, not a validated execution contract.
        // Preserve raw state for explicit inspection, without inventing a mapping.

        Ok(response)
    }

    /// Cancel an order that has not been executed yet
    ///
    /// # Arguments
    ///
    /// * `account` - Account to use. Must be a trading account
    /// * `order_id` - ID of the order to cancel
    pub async fn cancel_order(&self, account: &Account, order_id: &str) -> Result<()> {
        let url = get_cancel_order_url(&self.config)?;
        let response = self
            .client
            .post(url)
            .header("Content-Type", "application/json")
            .body(serde_json::to_string(&serde_json::json!({
                "accountKey": &account.id,
                "reference": order_id
            }))?)
            .send()
            .await
            .map_err(reqwest::Error::without_url)?;

        let status_code = response.status();

        let _response = response.text().await.map_err(reqwest::Error::without_url)?;

        if status_code != 200 {
            return Err(anyhow::anyhow!(
                "Failed to get order prepare response, status code: {}",
                status_code
            ));
        }

        info!("Order successfully cancelled");

        Ok(())
    }
}

fn get_order_url(config: &Config) -> Result<String> {
    let trading_url = get_trading_base_url(config)?;

    Ok(format!("{}/order", trading_url))
}

fn get_order_prepare_url(config: &Config, account: &Account, symbol: &str) -> Result<String> {
    Ok(
        format!(
            "{}/prepare?_host=tradingboard.boursobank.com&searchExtendedHours=false&selectedAccount={}&symbol={}",
            get_order_url(config)?,
            account.id,
            symbol
        )
    )
}

fn get_order_check_url(config: &Config) -> Result<String> {
    Ok(format!(
        "{}/ordersimple/check",
        get_trading_base_url(config)?
    ))
}

fn get_order_confirm_url(config: &Config) -> Result<String> {
    Ok(format!(
        "{}/ordersimple/confirm",
        get_trading_base_url(config)?
    ))
}

fn get_cancel_order_url(config: &Config) -> Result<String> {
    Ok(format!(
        "{}/orderdetail/cancel",
        get_trading_base_url(config)?
    ))
}

/// A response received after submission; this does not establish acceptance or execution.
#[derive(Clone, PartialEq)]
pub struct OrderSubmission {
    pub order_id: String,
    pub order_price_limit: Option<f64>,
    pub status: OrderSubmissionStatus,
    pub provider_state: RedactedOrderProviderState,
}

impl std::fmt::Debug for OrderSubmission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OrderSubmission")
            .field("status", &self.status)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderSubmissionStatus {
    /// HTTP 201 with a parsed response; provider business semantics are unverified.
    SubmittedUnverified,
}

/// Raw provider response, deliberately redacted in diagnostics.
#[derive(Clone, PartialEq)]
pub struct RedactedOrderProviderState(OrderConfirmResponse);

impl RedactedOrderProviderState {
    /// Sensitive provider data: inspect explicitly, never log without redaction.
    pub fn expose(&self) -> &OrderConfirmResponse {
        &self.0
    }
}

impl std::fmt::Debug for RedactedOrderProviderState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RedactedOrderProviderState([REDACTED])")
    }
}

/// Data fetched from the `/order/prepare` endpoint
#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderPrepareResponse {
    /// ID of the order, will be used to confirm the order
    pub resource_id: String,
    pub is_pcc: bool,
    pub pcc_rights: PccRights,
    pub has_right_to_assign: bool,
    pub has_right_to_force: bool,
    /// Current position
    pub position: Position,
    /// Account used to place the order informations
    pub account: PrepareOrderAccount,
    pub account_fiscality: AccountFiscality,
    pub account_fees_profile: String,
    pub pending_executed_orders: PendingExecutedOrders,
    pub acceptability_messages: Vec<Value>,
    pub symbol: Symbol,
    pub prepare_order_data: PrepareOrderData,
    pub prefill_order_data: OrderData,
    pub opcvm_message: String,
    pub dici_message: String,
    pub performance_url: String,
    pub execution_policy_url: String,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PccRights {
    pub allocate: bool,
    pub force: bool,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Position {
    pub cash: f64,
    pub srd_coverage: f64,
    pub quantity: i64,
    pub srd_quantity: i64,
}

/// Data fetched from the `/order/prepare` endpoint
#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrepareOrderAccount {
    pub has_pfm: bool,
    /// The account RIB (Relevé d'Identité Bancaire)
    pub rib: String,
    /// The account IBAN (International Bank Account Number)
    pub iban: String,
    /// The account BIC (Bank Identifier Code)
    pub bic: String,
    /// The account number
    pub account_number: String,
    /// The account name
    pub name: String,
    /// The account balance in euros. More like the instant value of the account
    /// depending on the current market value of the assets and the cash balance
    pub balance: f64,
    pub internal: bool,
    /// The account currency
    pub currency: String,
    /// The account type (e.g PEA, PEA-PME, CTO, etc.)
    #[serde(rename = "type")]
    pub type_field: String,
    /// Is the account a professional account
    pub professional: bool,
    /// The account subtype (e.g ISA - Individual Savings Account, etc.)
    pub subtype: String,
    /// The account role (e.g titular)
    pub role: String,
    /// The account bank ID (e.g 1 for BoursoBank)
    pub bank_id: String,
    /// The account bank name (e.g BoursoBank)
    pub bank_name: String,
    pub cash_out: i64,
    pub cash_in: i64,
    pub account_key: String,
    pub pfm_account_key: Value,
    /// The account type category (e.g TRADING, SAVINGS, etc.)
    pub type_category: String,
    pub has_unregular_operations: bool,
    /// The account shortname
    pub short_name: String,
    /// Owned by a minor
    pub minor: bool,
    pub contact_id_owner: Value,
    /// KADOR is a special account for minors by BoursoBank
    #[serde(rename = "isKADOR")]
    pub is_kador: bool,
    pub profile_type: Value,
    pub details: Details,
    pub has_incident: bool,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Details {
    /// The first time a cash transfer was made to the account
    pub first_cash_transfer_date: String,
    /// Gain/Losses in as a float value
    pub gain_losses_percent: f64,
    pub done_gain_losses_percent: f64,
    /// Current cash balance
    pub cash: f64,
    /// Current gain/losses in euros
    pub gain_losses: f64,
    pub done_gain_losses: f64,
    pub clearance_balance: f64,
    /// The account stocks value in euros
    pub stocks: f64,
    /// Today's date in format "2022-11-01"
    pub date: String,
    pub next_liquidation_date: String,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountFiscality {
    #[serde(rename = "latGL")]
    pub lat_gl: f64,
    #[serde(rename = "realGL")]
    pub real_gl: i64,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingExecutedOrders {
    pub pending: i64,
    pub executed: i64,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Symbol {
    /// Exchange Label on which the symbol is traded (e.g Euronext Paris)
    pub exchange_label: String,
    /// Symbol ID (e.g 1rTPE500 for AMUNDI PEA S&P 500 ESG UCITS ETF)
    pub symbol: String,
    pub nb_decimals: i64,
    /// Symbol currency (e.g EUR)
    pub currency: String,
    pub label: String,
    /// ISIN (International Securities Identification Numbers) of the symbol (e.g FR0013412285)
    pub isin: String,
    /// Last price of the symbol
    pub last_price: f64,
    /// Morning Star key information document URL (e.g https://doc.morningstar.com/LatestDoc.aspx?clientid=boursorama&key=507703e53b7dec23&language=454&investmentid=F000013MGI&documenttype=299&market=1443&investmenttype=1&frame=0)
    pub fund_morning_star_pdf_url: String,
    pub direct_issuer_kid_url: Value,
    pub priips_kid_url: Value,
    pub allow_tactical_orders: bool,
    pub details: Details2,
    pub extended_hours: ExtendedHours,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Details2 {
    pub opcvm: bool,
    pub affiliated: bool,
    pub direct_issuer: bool,
    pub tracker: bool,
    pub turbo: bool,
    pub warrant: bool,
    pub euronext: bool,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtendedHours {
    pub associated_symbol: String,
    pub lox_symbol: String,
    pub is_eligible: bool,
    pub lox_exchange_id: String,
    pub is_ost: bool,
    pub is_open: bool,
}

/// Data fetched from the `/order/prepare` endpoint and used to fill the default order data
#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrepareOrderData {
    /// Minimum expiration date in format "2022-11-01"
    pub min_expire_tm: String,
    /// Maximum expiration date in format "2022-11-01"
    pub max_expire_tm: String,
    /// Invalid dates list in format "2022-11-01"
    pub invalid_dates_list: Vec<String>,
    /// List of order types per side (buy or sell)
    pub list_ord_type: ListOrdType,
    pub list_risk_md: Vec<String>,
    /// List of possible sides (buy or sell)
    pub side_list: Vec<OrderSide>,
    // pub config_ord_type: ConfigOrdType,
}

/// Possible order types per side (buy or sell)
#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListOrdType {
    /// Buy order types
    pub b: Vec<OrderKind>,
    /// Sell order types
    pub s: Vec<OrderKind>,
}

/// Type of order
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub enum OrderKind {
    #[default]
    #[serde(rename = "LIM")]
    Limit,
    #[serde(rename = "ATP")]
    Market,
    /// Seuil de déclenchement
    #[serde(rename = "STP")]
    StopLoss,
    /// Plage de déclenchement
    #[serde(rename = "SLM")]
    StopLossMargin,
    #[serde(rename = "TSO")]
    TrailingStopOrder,
    /// One Cancels the Other order
    #[serde(rename = "OCO")]
    OneCancelsOther,
    #[serde(rename = "TAL")]
    TradeAtLast,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default, clap::ValueEnum)]
pub enum OrderSide {
    #[default]
    #[serde(rename = "B")]
    Buy,
    #[serde(rename = "S")]
    Sell,
}

/// Order data submitted to the `/ordersimple/check` endpoint
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct OrderData {
    #[serde(rename = "orderType")]
    order_type: OrderKind,
    #[serde(rename = "orderSide")]
    order_side: Option<OrderSide>,
    #[serde(rename = "orderQuantity")]
    order_quantity: Option<usize>,
    /// Expiration date in format "2022-11-01"
    #[serde(rename = "orderExpirationDate")]
    order_expiration_date: Option<String>,
    #[serde(rename = "orderRiskMode")]
    order_risk_mode: String,
    /// To use at the `/ordersimple/check` endpoint
    #[serde(rename = "orderPriceLimit")]
    order_price_limit: Option<f64>,
    /// Received at the `/order/prepare` endpoint
    #[serde(rename = "orderAmount")]
    order_amount: Option<f64>,
    #[serde(rename = "resourceId")]
    resource_id: Option<String>,
    /// Received at the `/order/prepare` endpoint
    /// Validity date in format "2022-11-01"
    #[serde(rename = "orderValidity")]
    order_validity: Option<String>,

    /// Received at the `/ordersimple/check` endpoint
    #[serde(rename = "buyingPower")]
    pub buying_power: Option<f64>,
    /// Received at the `/ordersimple/check` endpoint
    #[serde(rename = "stopPx")]
    pub stop_px: Option<Value>,
    /// Received at the `/ordersimple/check` endpoint
    #[serde(rename = "trailPct")]
    pub trail_pct: Option<Value>,
    /// Received at the `/ordersimple/check` endpoint
    #[serde(rename = "estimatedFees")]
    pub estimated_fees: Option<Vec<EstimatedFee>>,
    /// Received at the `/ordersimple/check` endpoint
    #[serde(rename = "exchangeLabel")]
    pub exchange_label: Option<String>,
    /// Received at the `/ordersimple/check` endpoint
    #[serde(rename = "feesExplanation")]
    pub fees_explanation: Option<FeesExplanation>,
    /// Received at the `/ordersimple/check` endpoint
    #[serde(rename = "estimatedBalance")]
    pub estimated_balance: Option<f64>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderCheckResponse {
    pub acceptability_messages: Option<Vec<AcceptabilityMessage>>,
    pub check_order_data: OrderData,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcceptabilityMessage {
    #[serde(rename = "type")]
    pub type_field: String,
    pub content: String,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EstimatedFee {
    #[serde(rename = "type")]
    pub type_field: String,
    pub label: String,
    pub amount: f64,
    pub percentage: f64,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeesExplanation {
    pub start_amount: String,
    pub product_fee: String,
    pub service_fee: String,
    pub scenarios: Vec<ScenarioMessage>,
    // pub translations: Translations,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScenarioMessage {
    pub title: String,
    pub content: Vec<Vec<String>>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderConfirmResponse {
    pub order_id: String,
    pub order_state_label: String,
    pub ord_stat: String,
    pub action_message: ActionMessage,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionMessage {
    pub id: String,
    #[serde(rename = "type")]
    pub type_field: String,
    pub detail: Value,
    pub title: Value,
    pub body: Value,
    pub params: Value,
    pub category: Value,
    pub actions: Vec<Action>,
    pub flags: Vec<Value>,
    pub targets: Vec<Value>,
    pub visual_id: Value,
    pub visual_theme: Value,
    pub medias: Vec<Value>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Action {
    pub label: Value,
    pub feature_id: Value,
    pub web: Value,
    pub api: ActionApi,
    pub disabled: bool,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionApi {
    pub href: Value,
    pub method: Value,
    pub params: ActionApiParams,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionApiParams {
    pub account_type: String,
    pub account_key: String,
}

#[cfg(test)]
mod outcome_tests {
    use super::*;
    use crate::outcome::{tests::*, OutcomeOperation, OutcomeStage, OutcomeUnknown};
    use std::sync::atomic::Ordering;

    #[test]
    fn final_order_post_errors_are_unknown_without_retry() {
        run(async {
            for (reply, stage) in [
                ("", OutcomeStage::Sending),
                ("HTTP/1.1 201 Created\r\nContent-Length: 999\r\nConnection: close\r\n\r\n{", OutcomeStage::ReadingResponse),
                ("HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n", OutcomeStage::HttpStatus),
                ("HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n", OutcomeStage::HttpStatus),
                ("HTTP/1.1 201 Created\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}", OutcomeStage::ParsingResponse),
                ("HTTP/1.1 201 Created\r\nContent-Length: 32\r\nConnection: close\r\n\r\nsynthetic-private-resource-token", OutcomeStage::ParsingResponse),
            ] {
                let (client, applied, worker) = server(reply);
                let error = client.confirm(SECRET).await.unwrap_err();
                worker.join().unwrap();
                assert_eq!(applied.load(Ordering::SeqCst), 1);
                assert_unknown(error, OutcomeOperation::Order, stage);
            }
        });
    }

    #[test]
    fn before_order_submission_error_is_not_unknown() {
        run(async {
            let client = BoursoWebClient::new();
            let error = client.confirm(SECRET).await.unwrap_err();
            assert!(error.downcast_ref::<OutcomeUnknown>().is_none());
        });
    }
}
