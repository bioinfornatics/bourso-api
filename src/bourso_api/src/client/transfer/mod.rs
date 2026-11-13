use crate::account::{Account, AccountKind};
use crate::outcome::{OutcomeOperation, OutcomeStage, OutcomeUnknown};
use crate::{client::transfer::error::TransferError, client::BoursoWebClient, constants::BASE_URL};
use anyhow::{bail, Context, Result};
use futures_util::stream::Stream;
use tracing::debug;

mod error;
pub mod money;

use money::{Money, MoneyError};

/// Existing local policy: at most 50 UTF-8 bytes; provider contract unverified.
/// Preserve the input verbatim (including empty or decomposed text); do not
/// normalize, trim or count Unicode scalar values/graphemes instead of bytes.
fn validate_transfer_reason_local_policy(reason: &str) -> std::result::Result<(), TransferError> {
    const MAX_REASON_UTF8_BYTES_LOCAL_POLICY: usize = 50;
    if reason.len() > MAX_REASON_UTF8_BYTES_LOCAL_POLICY {
        Err(TransferError::ReasonIsTooLong)
    } else {
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub enum TransferProgress {
    Validating,
    InitializingTransfer,
    ExtractingFlowInstance,
    SettingDebitAccount,
    SettingCreditAccount,
    SettingAmount,
    SubmittingStep5,
    SettingReason,
    ConfirmingTransfer,
    /// The final submission response was received, not a verified execution receipt.
    /// Check the bank before retrying: the transfer may already have been applied.
    SubmittedUnverified,
    /// Compatibility only; no longer emitted by the transfer streams.
    #[deprecated(note = "Use SubmittedUnverified; transfer execution is not verified")]
    Completed,
}

#[allow(deprecated)]
impl TransferProgress {
    pub fn step_number(&self) -> u8 {
        match self {
            TransferProgress::Validating => 1,
            TransferProgress::InitializingTransfer => 2,
            TransferProgress::ExtractingFlowInstance => 3,
            TransferProgress::SettingDebitAccount => 4,
            TransferProgress::SettingCreditAccount => 5,
            TransferProgress::SettingAmount => 6,
            TransferProgress::SubmittingStep5 => 7,
            TransferProgress::SettingReason => 8,
            TransferProgress::ConfirmingTransfer => 9,
            TransferProgress::SubmittedUnverified | TransferProgress::Completed => 10,
        }
    }

    pub fn total_steps() -> u8 {
        10
    }

    /// Operator notice for a finished submission, never proof of bank execution.
    /// The CLI uses this same text so normal stream exhaustion cannot imply success.
    pub fn submission_warning(&self) -> Option<&'static str> {
        match self {
            Self::SubmittedUnverified | Self::Completed => Some(
                "Transfer submitted-unverified: submission response received; execution unverified. Verify in the bank before retrying; do not resubmit automatically.",
            ),
            _ => None,
        }
    }

    pub fn description(&self) -> &str {
        match self {
            TransferProgress::Validating => "Validating transfer parameters",
            TransferProgress::InitializingTransfer => "Initializing transfer",
            TransferProgress::ExtractingFlowInstance => "Extracting flow instance",
            TransferProgress::SettingDebitAccount => "Setting debit account",
            TransferProgress::SettingCreditAccount => "Setting credit account",
            TransferProgress::SettingAmount => "Setting transfer amount",
            TransferProgress::SubmittingStep5 => "Submitting intermediate step",
            TransferProgress::SettingReason => "Setting transfer reason",
            TransferProgress::ConfirmingTransfer => "Submitting transfer request",
            TransferProgress::SubmittedUnverified | TransferProgress::Completed => {
                "Submission response received; execution unverified"
            }
        }
    }
}

impl BoursoWebClient {
    /// Initialize the transfer and extract the transfer ID
    async fn init_transfer(&self, from_account: &str) -> Result<String> {
        let init_transfer_url = format!(
            "{}/compte/cav/{}/virements/immediat/nouveau",
            BASE_URL, from_account
        );

        let res = self
            .client
            .get(&init_transfer_url)
            .send()
            .await
            .map_err(reqwest::Error::without_url)?;

        if res.status() != 302 {
            debug!("Init transfer response status: {}", res.status());
            bail!(TransferError::TransferInitiationFailed);
        }

        let location = res
            .headers()
            .get("location")
            .context("Missing Location header")?
            .to_str()?;

        // /compte/cav/XXXXXXX/virements/immediat/nouveau/YYYYY/1
        // get YYYYY
        let transfer_id = location
            .split('/')
            .nth(7)
            .filter(|id| !id.is_empty())
            .context("Failed to extract transfer id")?
            .to_string();

        Ok(transfer_id)
    }

    /// Extract the flow instance from the HTML response
    async fn extract_flow_instance(&self, url: &str) -> Result<String> {
        let res = self
            .client
            .get(url)
            .send()
            .await
            .map_err(reqwest::Error::without_url)?;

        if res.status() != 200 {
            debug!("First transfer step response status: {}", res.status());
            bail!(TransferError::TransferInitiationFailed);
        }

        let res_text = res.text().await.map_err(reqwest::Error::without_url)?;
        let re = regex::Regex::new(r#"name="flow_ImmediateCashTransfer_instance" value="([^"]+)""#)
            .unwrap();
        let flow_instance = re
            .captures(&res_text)
            .and_then(|cap| cap.get(1))
            .map(|m| m.as_str())
            .context("Failed to extract flow instance")?
            .to_string();

        Ok(flow_instance)
    }

    /// Set the debit account (step 2)
    async fn set_debit_account(
        &self,
        from_account: &str,
        transfer_id: &str,
        flow_instance: &str,
    ) -> Result<()> {
        let data = reqwest::multipart::Form::new()
            .text(
                "flow_ImmediateCashTransfer_instance",
                flow_instance.to_string(),
            )
            .text("flow_ImmediateCashTransfer_step", "1".to_string())
            .text("DebitAccount[debit]", from_account.to_string());

        let url = format!(
            "{}/compte/cav/{}/virements/immediat/nouveau/{}/2",
            BASE_URL, from_account, transfer_id
        );

        let res = self
            .client
            .post(&url)
            .multipart(data)
            .send()
            .await
            .map_err(reqwest::Error::without_url)?;

        if res.status() != 200 {
            debug!("Set debit account response status: {}", res.status());
            bail!(TransferError::SetDebitAccountFailed);
        }

        Ok(())
    }

    /// Set the credit account (step 3)
    async fn set_credit_account(
        &self,
        from_account: &str,
        to_account: &str,
        transfer_id: &str,
        flow_instance: &str,
        transfer_from_banking: bool,
    ) -> Result<()> {
        let form = if transfer_from_banking {
            reqwest::multipart::Form::new().text("CreditAccount[newBeneficiary]", "0".to_string())
        } else {
            reqwest::multipart::Form::new()
        };

        let data = form
            .text(
                "flow_ImmediateCashTransfer_instance",
                flow_instance.to_string(),
            )
            .text("flow_ImmediateCashTransfer_step", "2".to_string())
            .text("CreditAccount[credit]", to_account.to_string());

        let url = format!(
            "{}/compte/cav/{}/virements/immediat/nouveau/{}/3",
            BASE_URL, from_account, transfer_id
        );

        let res = self
            .client
            .post(&url)
            .multipart(data)
            .send()
            .await
            .map_err(reqwest::Error::without_url)?;

        if res.status() != 200 {
            debug!("Set credit account response status: {}", res.status());
            bail!(TransferError::SetCreditAccountFailed);
        }

        Ok(())
    }

    /// Set the transfer amount (step 4)
    async fn set_transfer_amount(
        &self,
        from_account: &str,
        transfer_id: &str,
        flow_instance: &str,
        amount: Money,
    ) -> Result<()> {
        let data = reqwest::multipart::Form::new()
            .text(
                "flow_ImmediateCashTransfer_instance",
                flow_instance.to_string(),
            )
            .text("flow_ImmediateCashTransfer_step", "3".to_string())
            .text("Amount[amount]", amount.bank_amount())
            .text("flow_ImmediateCashTransfer_transition", "".to_string())
            .text("submit", "".to_string());

        let url = format!(
            "{}/compte/cav/{}/virements/immediat/nouveau/{}/4",
            BASE_URL, from_account, transfer_id
        );

        let res = self
            .client
            .post(&url)
            .multipart(data)
            .send()
            .await
            .map_err(reqwest::Error::without_url)?;

        if res.status() != 200 {
            debug!("Set amount response status: {}", res.status());
            bail!(TransferError::SetAmountFailed);
        }

        Ok(())
    }

    /// Submit step 5
    async fn submit_step_5(
        &self,
        from_account: &str,
        transfer_id: &str,
        flow_instance: &str,
    ) -> Result<()> {
        let data = reqwest::multipart::Form::new()
            .text("flow_ImmediateCashTransfer_transition", "".to_string())
            .text("flow_ImmediateCashTransfer_transition", "".to_string())
            .text(
                "flow_ImmediateCashTransfer_instance",
                flow_instance.to_string(),
            )
            .text("flow_ImmediateCashTransfer_step", "4".to_string())
            .text("submit", "".to_string());

        let res = self
            .client
            .post(format!(
                "{}/compte/cav/{}/virements/immediat/nouveau/{}/5",
                BASE_URL, from_account, transfer_id
            ))
            .multipart(data)
            .send()
            .await
            .map_err(reqwest::Error::without_url)?;

        if res.status() != 200 {
            debug!("Submit transfer response status: {}", res.status());
            bail!(TransferError::Step5Failed);
        }

        Ok(())
    }

    /// Set the transfer reason (step 8)
    async fn set_transfer_reason(
        &self,
        from_account: &str,
        transfer_id: &str,
        flow_instance: &str,
        transfer_reason: &str,
    ) -> Result<()> {
        let data = reqwest::multipart::Form::new()
            .text(
                "flow_ImmediateCashTransfer_instance",
                flow_instance.to_string(),
            )
            .text("flow_ImmediateCashTransfer_step", "7".to_string())
            .text("Characteristics[label]", transfer_reason.to_string())
            .text("Characteristics[schedulingType]", "1".to_string()) // 1 = unique
            .text("flow_ImmediateCashTransfer_transition", "".to_string())
            .text("flow_ImmediateCashTransfer_transition", "".to_string())
            .text("submit", "".to_string());

        let url = format!(
            "{}/compte/cav/{}/virements/immediat/nouveau/{}/8",
            BASE_URL, from_account, transfer_id
        );

        let res = self
            .client
            .post(&url)
            .multipart(data)
            .send()
            .await
            .map_err(reqwest::Error::without_url)?;

        if res.status() != 200 {
            debug!("Set reason response status: {}", res.status());
            bail!(TransferError::SetReasonFailed);
        }

        Ok(())
    }

    /// Send the final transfer request (step 10), without asserting execution.
    async fn confirm_transfer(
        &self,
        from_account: &str,
        transfer_id: &str,
        flow_instance: &str,
    ) -> Result<()> {
        let data = reqwest::multipart::Form::new()
            .text(
                "flow_ImmediateCashTransfer_instance",
                flow_instance.to_string(),
            )
            .text("flow_ImmediateCashTransfer_step", "9".to_string())
            .text("flow_ImmediateCashTransfer_transition", "".to_string())
            .text("flow_ImmediateCashTransfer_transition", "".to_string())
            .text("submit", "".to_string());

        let unknown = OutcomeUnknown::new(OutcomeOperation::Transfer, transfer_id);
        let res = self
            .client
            .post(format!(
                "{}/compte/cav/{}/virements/immediat/nouveau/{}/10",
                BASE_URL, from_account, transfer_id
            ))
            .multipart(data)
            .send()
            .await
            .map_err(|_| unknown.at_stage(OutcomeStage::Sending))?;

        if res.status() != 200 {
            return Err(unknown.at_stage(OutcomeStage::HttpStatus).into());
        }

        let _body = res
            .text()
            .await
            .map_err(|_| unknown.at_stage(OutcomeStage::ReadingResponse))?;

        // No trusted receipt contract is available. HTTP 200, a "Confirmation"
        // heading, an error page, or a login form cannot establish execution.
        // Consume the response to retain read-failure classification, but never
        // parse, log or expose its potentially sensitive contents as proof.
        Ok(())
    }

    /// Transfer funds from one account to another, yielding progress updates.
    ///
    /// A final submission error may downcast to [`OutcomeUnknown`]. Do not
    /// reissue automatically. Dropping the stream cannot return this error;
    /// callers need durable external state for cancellation/crash recovery.
    /// Normal exhaustion ends with [`TransferProgress::SubmittedUnverified`], not
    /// verified execution. HTTP 200 is only a submission response. Verify with the
    /// bank before retrying; no automatic resubmission is safe. Progress reaching
    /// 100% describes this local submission process, not bank execution.
    ///
    /// ## Arguments
    /// - `amount`: Amount to transfer (>= 10 EUR, finite, at most two decimal
    ///   places in its shortest round-trip decimal spelling; never rounded).
    /// - `from_account`: Source account
    /// - `to_account`: Destination account
    /// - `reason`: Optional reason (max 50 UTF-8 bytes, local policy; provider
    ///   contract unverified). No Unicode normalization or trimming is performed.
    ///
    /// ## Returns
    /// A stream of progress updates for the transfer.
    pub fn transfer_funds(
        &self,
        amount: f64,
        from_account: Account,
        to_account: Account,
        reason: Option<String>,
    ) -> impl Stream<Item = Result<TransferProgress>> + '_ {
        self.transfer_validated(Money::try_from(amount), from_account, to_account, reason)
    }

    /// Transfer an exact decimal amount without passing through floating point.
    ///
    /// Like [`Self::transfer_funds`], normal completion is submitted-unverified,
    /// never a verified bank execution receipt. Check the bank before retrying.
    ///
    /// `reason` uses the same max 50 UTF-8 bytes local policy as `transfer_funds`;
    /// provider contract unverified. The supplied text is preserved verbatim.
    pub fn transfer_money(
        &self,
        amount: Money,
        from_account: Account,
        to_account: Account,
        reason: Option<String>,
    ) -> impl Stream<Item = Result<TransferProgress>> + '_ {
        self.transfer_validated(Ok(amount), from_account, to_account, reason)
    }

    fn transfer_validated(
        &self,
        amount: std::result::Result<Money, MoneyError>,
        from_account: Account,
        to_account: Account,
        reason: Option<String>,
    ) -> impl Stream<Item = Result<TransferProgress>> + '_ {
        async_stream::stream! {
            yield Ok(TransferProgress::Validating);
            let amount = match amount {
                Ok(amount) => amount,
                Err(MoneyError::TooLow) => {
                    yield Err(TransferError::AmountTooLow.into());
                    return;
                }
                Err(error) => {
                    yield Err(error.into());
                    return;
                }
            };

            debug!(
                "Initiating transfer of {} EUR",
                amount
            );

            let transfer_from_banking = from_account.kind == AccountKind::Banking;
            let from_account_id = from_account.id.clone();
            let to_account_id = to_account.id.clone();

            let transfer_reason = reason.unwrap_or_else(|| "Virement depuis BoursoBank".to_string());
            // Enforce the existing local byte policy before any transfer request.
            if let Err(error) = validate_transfer_reason_local_policy(&transfer_reason) {
                yield Err(error.into());
                return;
            }

            // Step 1: Initialize transfer and get transfer ID
            yield Ok(TransferProgress::InitializingTransfer);
            let transfer_id = match self.init_transfer(&from_account_id).await {
                Ok(id) => id,
                Err(e) => {
                    yield Err(e);
                    return;
                }
            };

            // Extract flow instance
            yield Ok(TransferProgress::ExtractingFlowInstance);
            let flow_instance = match self
                .extract_flow_instance(&format!(
                    "{}/compte/cav/{}/virements/immediat/nouveau/{}/1",
                    BASE_URL, &from_account_id, transfer_id
                ))
                .await {
                Ok(flow) => flow,
                Err(e) => {
                    yield Err(e);
                    return;
                }
            };

            // Step 2: Set debit account
            yield Ok(TransferProgress::SettingDebitAccount);
            if let Err(e) = self.set_debit_account(&from_account_id, &transfer_id, &flow_instance)
                .await {
                yield Err(e);
                return;
            }

            // Step 3: Set credit account
            yield Ok(TransferProgress::SettingCreditAccount);
            if let Err(e) = self.set_credit_account(
                &from_account_id,
                &to_account_id,
                &transfer_id,
                &flow_instance,
                transfer_from_banking,
            )
            .await {
                yield Err(e);
                return;
            }

            // Step 6: Set amount
            yield Ok(TransferProgress::SettingAmount);
            if let Err(e) = self.set_transfer_amount(&from_account_id, &transfer_id, &flow_instance, amount)
                .await {
                yield Err(e);
                return;
            }

            // Step 5: Submit
            yield Ok(TransferProgress::SubmittingStep5);
            if let Err(e) = self.submit_step_5(&from_account_id, &transfer_id, &flow_instance)
                .await {
                yield Err(e);
                return;
            }

            // Step 10: Set reason
            yield Ok(TransferProgress::SettingReason);
            if let Err(e) = self.set_transfer_reason(
                &from_account_id,
                &transfer_id,
                &flow_instance,
                &transfer_reason,
            )
            .await {
                yield Err(e);
                return;
            }

            // Step 12: Confirm transfer
            yield Ok(TransferProgress::ConfirmingTransfer);
            if let Err(e) = self.confirm_transfer(&from_account_id, &transfer_id, &flow_instance)
                .await {
                yield Err(e);
                return;
            }

            yield Ok(TransferProgress::SubmittedUnverified);
        }
    }
}

#[cfg(test)]
mod money_tests;

#[cfg(test)]
mod outcome_tests {
    use crate::outcome::{tests::*, OutcomeOperation, OutcomeStage};
    use std::sync::atomic::Ordering;

    #[test]
    fn final_transfer_post_errors_are_unknown_without_retry() {
        run(async {
            for (reply, stage) in [
                ("", OutcomeStage::Sending),
                ("HTTP/1.1 200 OK\r\nContent-Length: 999\r\nConnection: close\r\n\r\n<", OutcomeStage::ReadingResponse),
                ("HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n", OutcomeStage::HttpStatus),
            ] {
                let (client, applied, worker) = server(reply);
                let error = client.confirm_transfer(SECRET, SECRET, SECRET).await.unwrap_err();
                worker.join().unwrap();
                assert_eq!(applied.load(Ordering::SeqCst), 1);
                assert_unknown(error, OutcomeOperation::Transfer, stage);
            }
        });
    }

    #[test]
    fn confirmation_response_is_received_but_not_a_validated_receipt() {
        run(async {
            let (client, applied, worker) = server(
                "HTTP/1.1 200 OK\r\nContent-Length: 12\r\nConnection: close\r\n\r\nConfirmation",
            );
            assert!(client
                .confirm_transfer(SECRET, SECRET, SECRET)
                .await
                .is_ok());
            worker.join().unwrap();
            assert_eq!(applied.load(Ordering::SeqCst), 1);
        });
    }
}

#[cfg(test)]
mod reason_tests {
    use super::*;
    use futures_util::{pin_mut, StreamExt};
    use std::{net::TcpListener, time::Duration};

    fn cases() -> Vec<(Option<String>, bool)> {
        vec![
            (None, true),
            (Some(String::new()), true),
            (Some("a".repeat(50)), true),
            (Some("a".repeat(51)), false),
            (Some("é".repeat(25)), true),
            (Some("é".repeat(26)), false),
            (Some("😀".repeat(12)), true),
            (Some("😀".repeat(13)), false),
            // Each decomposed e + combining acute is three UTF-8 bytes.
            (Some("e\u{301}".repeat(16)), true),
            (Some(format!("{}ab", "e\u{301}".repeat(16))), true),
            (Some("e\u{301}".repeat(17)), false),
        ]
    }

    #[test]
    fn existing_reason_byte_boundaries_gate_both_public_streams_before_requests() {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let listener = TcpListener::bind("127.0.0.1:0").unwrap();
                listener.set_nonblocking(true).unwrap();
                let client = BoursoWebClient::for_test(
                    listener.local_addr().unwrap(),
                    Duration::from_secs(3),
                )
                .unwrap();
                for legacy in [false, true] {
                    for (reason, accepted) in cases() {
                        let stream: std::pin::Pin<
                            Box<dyn Stream<Item = Result<TransferProgress>> + '_>,
                        > = if legacy {
                            Box::pin(client.transfer_funds(
                                10.0,
                                Account::default(),
                                Account::default(),
                                reason.clone(),
                            ))
                        } else {
                            Box::pin(client.transfer_money(
                                "10".parse().unwrap(),
                                Account::default(),
                                Account::default(),
                                reason.clone(),
                            ))
                        };
                        pin_mut!(stream);
                        assert!(matches!(
                            stream.next().await.unwrap().unwrap(),
                            TransferProgress::Validating
                        ));
                        let next = stream.next().await.unwrap();
                        if accepted {
                            // Local acceptance only: stop before polling the first request.
                            assert!(matches!(
                                next.unwrap(),
                                TransferProgress::InitializingTransfer
                            ));
                        } else {
                            let error = next.unwrap_err();
                            assert!(matches!(
                                error.downcast_ref::<TransferError>(),
                                Some(TransferError::ReasonIsTooLong)
                            ));
                            let reason = reason.as_ref().unwrap();
                            assert!(!error.to_string().contains(reason));
                            assert!(!format!("{error:?}").contains(reason));
                            assert!(stream.next().await.is_none());
                        }
                        assert_eq!(
                            listener.accept().unwrap_err().kind(),
                            std::io::ErrorKind::WouldBlock
                        );
                    }
                }
            });
    }

    #[test]
    fn pure_local_policy_preserves_existing_byte_boundaries() {
        for (reason, accepted) in cases() {
            let reason = reason.unwrap_or_else(|| "Virement depuis BoursoBank".to_string());
            assert_eq!(
                validate_transfer_reason_local_policy(&reason).is_ok(),
                accepted
            );
        }
    }

    #[test]
    fn reason_error_labels_the_local_byte_policy_not_a_provider_contract() {
        assert_eq!(
            TransferError::ReasonIsTooLong.to_string(),
            "Transfer reason is too long, max 50 UTF-8 bytes, local policy; provider contract unverified"
        );
    }
}

#[cfg(test)]
mod submission_tests;

#[cfg(test)]
mod init_tests;
