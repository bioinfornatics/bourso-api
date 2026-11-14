//! External callers exercise only the public session; all rejects occur locally.
#![cfg(unix)]
use bourso_api::{
    account::Account,
    client::trade::order::OrderSide,
    replay_guard::{GuardError, ReplayGuard, ReviewDecision},
    FinancialSession,
};
use std::{fs, path::PathBuf};
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        Self(
            std::env::temp_dir()
                .canonicalize()
                .unwrap()
                .join(format!("bourso-session-{:032x}", rand::random::<u128>())),
        )
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn run(future: impl std::future::Future<Output = ()>) {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future);
}
#[test]
fn public_session_cross_operation_review_and_shared_domain() {
    run(async {
        let temp = Temp::new();
        let mut session = FinancialSession::open(&temp.0).unwrap();
        assert!(matches!(
            ReplayGuard::open(&temp.0),
            Err(GuardError::Locked)
        ));
        assert!(matches!(
            FinancialSession::open(&temp.0),
            Err(GuardError::Locked)
        ));
        let client = bourso_api::get_client();
        // Quantity zero fails before network, but AFTER mandatory reservation.
        assert!(session
            .order(
                &client,
                OrderSide::Buy,
                &Account::default(),
                "SECRET",
                0,
                None
            )
            .await
            .unwrap()
            .is_err());
        let attempt = session.unresolved().unwrap().unwrap();
        let bytes = fs::read(temp.0.join("replay-unresolved.json")).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("SECRET"));
        assert!(matches!(
            session
                .transfer_money(
                    &client,
                    "12.00".parse().unwrap(),
                    Account::default(),
                    Account::default(),
                    None
                )
                .await,
            Err(GuardError::Unresolved)
        ));
        assert_eq!(
            session.review("wrong", ReviewDecision::NotApplied),
            Err(GuardError::ReviewMismatch)
        );
        assert_eq!(
            fs::read(temp.0.join("replay-unresolved.json")).unwrap(),
            bytes
        );
        drop(session);
        let mut session = FinancialSession::open(&temp.0).unwrap();
        assert_eq!(session.unresolved().unwrap().unwrap(), attempt);
        session
            .review(attempt.id(), ReviewDecision::OperatorReviewed)
            .unwrap();
        // Invalid local reason exercises the other public API without bank work.
        assert!(session
            .transfer_money(
                &client,
                "12.00".parse().unwrap(),
                Account::default(),
                Account::default(),
                Some("x".repeat(51))
            )
            .await
            .unwrap()
            .is_err());
        assert_ne!(session.unresolved().unwrap().unwrap().id(), attempt.id());
    });
}
#[test]
fn public_session_storage_failure_blocks_before_chain() {
    run(async {
        let temp = Temp::new();
        let mut session = FinancialSession::open(&temp.0).unwrap();
        fs::create_dir(temp.0.join("replay-unresolved.json")).unwrap();
        assert!(matches!(
            session
                .order(
                    &bourso_api::get_client(),
                    OrderSide::Buy,
                    &Account::default(),
                    "SECRET",
                    2,
                    None
                )
                .await,
            Err(GuardError::InvalidJournal)
        ));
        assert!(session.review("wrong", ReviewDecision::NotApplied).is_err());
        assert!(temp.0.join("replay-unresolved.json").is_dir());
    });
}
