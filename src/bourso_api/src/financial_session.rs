//! Mandatory local replay barrier for public financial submissions.
//! Choose the same explicit trusted absolute directory as cooperating CLI/API
//! callers. No automatic clearing, retry, bank reconciliation or fsync guarantee.
use crate::{
    account::Account,
    client::{
        trade::order::{OrderData, OrderSide, OrderSubmission},
        transfer::{money::Money, TransferProgress},
        BoursoWebClient,
    },
    replay_guard::{Attempt, GuardError, ReplayGuard, ReviewDecision},
    OutcomeOperation,
};
use futures_util::{Stream, StreamExt};
use std::path::Path;

/// Owns the domain lock; no client, guard extraction or unguarded financial path.
/// The mutable borrow lasts through the entire operation, including transfer
/// stream consumption. Dropping a polled future retains its reservation.
#[derive(Debug)]
pub struct FinancialSession {
    guard: ReplayGuard,
}
impl FinancialSession {
    /// On non-Unix, refuses before inspecting the supplied path or storage.
    /// Unix directories require trusted ancestors; see [`ReplayGuard::open`].
    pub fn open(directory: impl AsRef<Path>) -> Result<Self, GuardError> {
        Ok(Self {
            guard: ReplayGuard::open(directory)?,
        })
    }
    pub fn unresolved(&self) -> Result<Option<Attempt>, GuardError> {
        self.guard.unresolved()
    }
    /// Explicit exact-ID local review only: no bank request or retry.
    pub fn review(&mut self, id: &str, decision: ReviewDecision) -> Result<(), GuardError> {
        self.guard.review(id, decision)
    }
    /// Reserve before the entire order chain; preserve the original unverified
    /// submission or downcastable bank error inside the outer local result.
    pub async fn order(
        &mut self,
        client: &BoursoWebClient,
        side: OrderSide,
        account: &Account,
        symbol: &str,
        quantity: usize,
        order_data: Option<OrderData>,
    ) -> Result<anyhow::Result<OrderSubmission>, GuardError> {
        self.guard
            .invoke(OutcomeOperation::Order, || {
                client.order_detailed(side, account, symbol, quantity, order_data)
            })
            .await
    }
    /// Reserve before any transfer step, consume the complete stream internally,
    /// and require a terminal submitted-unverified response. Never returns a stream.
    pub async fn transfer_money(
        &mut self,
        client: &BoursoWebClient,
        amount: Money,
        from_account: Account,
        to_account: Account,
        reason: Option<String>,
    ) -> Result<anyhow::Result<TransferProgress>, GuardError> {
        self.guard
            .invoke(OutcomeOperation::Transfer, || async {
                consume_transfer(client.transfer_money(amount, from_account, to_account, reason))
                    .await
            })
            .await
    }
}
async fn consume_transfer(
    stream: impl Stream<Item = anyhow::Result<TransferProgress>>,
) -> anyhow::Result<TransferProgress> {
    futures_util::pin_mut!(stream);
    let mut terminal = false;
    while let Some(progress) = stream.next().await {
        terminal = matches!(progress?, TransferProgress::SubmittedUnverified);
    }
    anyhow::ensure!(terminal, "No terminal transfer response");
    Ok(TransferProgress::SubmittedUnverified)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exhaustion_requires_terminal_and_errors_remain_original() {
        crate::outcome::tests::run(async {
            for items in [
                vec![],
                vec![Ok(TransferProgress::Validating)],
                vec![
                    Ok(TransferProgress::SubmittedUnverified),
                    Ok(TransferProgress::Validating),
                ],
            ] {
                assert!(consume_transfer(futures_util::stream::iter(items))
                    .await
                    .is_err());
            }
            assert!(matches!(
                consume_transfer(futures_util::stream::iter([Ok(
                    TransferProgress::SubmittedUnverified
                )]))
                .await
                .unwrap(),
                TransferProgress::SubmittedUnverified
            ));
            let err = consume_transfer(futures_util::stream::iter([Err(anyhow::Error::new(
                std::io::Error::other("sentinel"),
            ))]))
            .await
            .unwrap_err();
            assert!(err.downcast_ref::<std::io::Error>().is_some());
        });
    }
}
