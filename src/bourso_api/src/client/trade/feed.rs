use crate::client::{config::Config, BoursoWebClient};
use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Market session status. Unknown must not be interpreted as open or closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarketStatus {
    /// No authoritative session information is available; the reason is safe to log.
    Unknown(&'static str),
}

const MARKET_STATUS_UNAVAILABLE: &str = "Market status unavailable: authoritative exchange timezone and trading-session calendar/status are not provided by the quote feed";

impl BoursoWebClient {
    /// Returns an explicit unknown without consulting the clock or the network.
    ///
    /// Quote opening/closing strings alone do not establish an exchange timezone,
    /// session date, holidays, exceptional closures or overnight-session rules.
    /// Consequently this currently returns Unknown for every symbol and instant,
    /// independently of the host timezone. It does not validate the symbol.
    pub async fn market_status(&self, _symbol: &str) -> MarketStatus {
        MarketStatus::Unknown(MARKET_STATUS_UNAVAILABLE)
    }

    /// Compatibility API: unavailable session information is an error, not false.
    /// Use `market_status` to handle Unknown explicitly. Neither API makes a request.
    pub async fn is_market_open(&self, symbol: &str) -> Result<bool> {
        match self.market_status(symbol).await {
            MarketStatus::Unknown(reason) => Err(anyhow::anyhow!(reason)),
        }
    }

    pub async fn instrument_quote(&self, symbol: &str) -> Result<InstrumentQuoteResponse> {
        let url = get_instrument_quote_url(&self.config, symbol)?;
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
                "Failed to get instrument quote response, status code: {}",
                status_code
            ));
        }

        let response: InstrumentQuoteResponse = serde_json::from_str(&response).map_err(|e| {
            crate::client::safe_json_error("Failed to parse instrument quote response", e)
        })?;

        Ok(response)
    }
}

fn get_feed_base_url(config: &Config) -> Result<String> {
    Ok(format!("{}/_public_/feed", config.api_url))
}

fn get_instrument_quote_url(config: &Config, symbol: &str) -> Result<String> {
    Ok(format!(
        "{}/instrument/quote/{}?_host=tradingboard.boursobank.com",
        get_feed_base_url(config)?,
        symbol
    ))
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstrumentQuoteResponse {
    pub symbol: String,
    pub label: String,
    pub isin: String,
    pub last: f64,
    pub currency: String,
    pub previous_close: f64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub total_volume: i64,
    pub exchange_id: i64,
    pub exchange_code: String,
    pub exchange_label: String,
    pub opening_time: String,
    pub closing_time: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::ErrorKind, net::TcpListener, time::Duration};

    #[test]
    fn market_unavailable_without_requesting_quote() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let mut client =
            BoursoWebClient::for_test(listener.local_addr().unwrap(), Duration::from_millis(100))
                .unwrap();
        client.config.api_url = format!("http://{}", listener.local_addr().unwrap());
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let result = runtime.block_on(client.is_market_open("1rPTEST"));
        assert!(
            matches!(listener.accept(), Err(e) if e.kind() == ErrorKind::WouldBlock),
            "insufficient session data must not cause a network request"
        );
        let error = result.expect_err("quote hours cannot establish market status");
        assert!(error.to_string().contains("Market status unavailable"));
    }

    #[test]
    fn market_status_is_explicit_unknown_for_any_symbol_without_io() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let mut client =
            BoursoWebClient::for_test(listener.local_addr().unwrap(), Duration::from_millis(100))
                .unwrap();
        client.config.api_url = format!("http://{}", listener.local_addr().unwrap());
        // Polling must finish immediately, without even requiring a Tokio runtime.
        use futures_util::FutureExt;
        for symbol in ["1rPTEST", "", "unknown-exchange", "private-symbol\nsecret"] {
            assert_eq!(
                client.market_status(symbol).now_or_never(),
                Some(MarketStatus::Unknown(MARKET_STATUS_UNAVAILABLE))
            );
            let error = client
                .is_market_open(symbol)
                .now_or_never()
                .expect("unavailability needs no asynchronous I/O")
                .expect_err("unknown must never become an open/closed boolean");
            assert_eq!(error.to_string(), MARKET_STATUS_UNAVAILABLE);
        }
        assert!(matches!(listener.accept(), Err(e) if e.kind() == ErrorKind::WouldBlock));
    }
}
