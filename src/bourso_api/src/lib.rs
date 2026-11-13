pub mod account;
#[path = "client/bank_money.rs"]
pub(crate) mod bank_money;
pub mod client;
pub mod constants;

pub fn get_client() -> client::BoursoWebClient {
    client::BoursoWebClient::new()
}

#[path = "client/outcome.rs"]
pub mod outcome;
pub use outcome::{OutcomeOperation, OutcomeStage, OutcomeUnknown};
