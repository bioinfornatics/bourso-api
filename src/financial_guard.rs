//! CLI reporting over the same mandatory API session/domain; no second journal.
use crate::settings;
use anyhow::{Context, Result};
use bourso_api::{
    account::Account,
    client::{trade::order::OrderSide, transfer::money::Money, BoursoWebClient},
    replay_guard::{GuardError, ReviewDecision},
    FinancialSession,
};
use std::{fmt, path::Path};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    NotEmitted,
    ResponseUnverified,
    OutcomeUnknown,
}

/// Contains only local, generated identifiers and fixed diagnostics, never bank data.
#[derive(Debug)]
pub struct Report {
    pub state: State,
    attempt: Option<String>,
    detail: &'static str,
}
impl Report {
    pub fn not_emitted() -> Self {
        Self {
            state: State::NotEmitted,
            attempt: None,
            detail: "Pre-invocation failure; no financial API invoked by this command. Check local configuration (config --username), credentials and private storage.",
        }
    }
    pub fn print(&self) {
        eprintln!("{self}");
    }
    pub fn exit_code(&self) -> u8 {
        match self.state {
            State::NotEmitted => 2,
            State::ResponseUnverified => 3,
            State::OutcomeUnknown => 4,
        }
    }
}
impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = match self.state {
            State::NotEmitted => "not-emitted",
            State::ResponseUnverified => "response-unverified",
            State::OutcomeUnknown => "outcome-unknown",
        };
        write!(f, "financial-result: {state}")?;
        if let Some(attempt) = &self.attempt {
            write!(f, " attempt={attempt}")?;
        }
        write!(f, "; {}", self.detail)
    }
}
impl std::error::Error for Report {}

pub fn is_financial(matches: &clap::ArgMatches) -> bool {
    matches!(
        matches.subcommand_name(),
        Some("trade" | "transfer" | "financial-review")
    )
}

// Never hide an unexpected old reservation. Do not read, migrate or delete it.
fn open_at(directory: &Path) -> Result<FinancialSession> {
    let session = FinancialSession::open(directory)?;
    match std::fs::symlink_metadata(directory.join("financial-unresolved.json")) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(session),
        _ => Err(Report { state: State::NotEmitted, attempt: None,
            detail: "Unexpected legacy financial journal; manual investigation required; no financial API invoked." }.into()),
    }
}
/// Acquired before authentication, retained through the complete API operation.
#[derive(Debug)]
pub struct Session(FinancialSession);
impl Session {
    pub fn open() -> Result<Self> {
        Self::at(&settings::storage_directory()?)
    }
    fn at(directory: &Path) -> Result<Self> {
        let session = open_at(directory)?;
        if let Some(attempt) = session.unresolved()? {
            return Err(Report { state: State::NotEmitted, attempt: Some(attempt.id().to_owned()),
                detail: "This invocation blocked by unresolved prior attempt (possibly-not-emitted). Reconcile with the bank, then use financial-review; never retry automatically." }.into());
        }
        Ok(Self(session))
    }
    pub async fn order(
        &mut self,
        client: &BoursoWebClient,
        side: OrderSide,
        account: &Account,
        symbol: &str,
        quantity: usize,
    ) -> Report {
        let result = self
            .0
            .order(client, side, account, symbol, quantity, None)
            .await;
        self.report(result)
    }
    pub async fn transfer_money(
        &mut self,
        client: &BoursoWebClient,
        amount: Money,
        from: Account,
        to: Account,
        reason: Option<String>,
    ) -> Report {
        let result = self
            .0
            .transfer_money(client, amount, from, to, reason)
            .await;
        self.report(result)
    }
    fn report<T>(&self, result: std::result::Result<Result<T>, GuardError>) -> Report {
        Report::from_result(
            result,
            self.0.unresolved().ok().flatten().as_ref().map(|a| a.id()),
        )
    }
}
impl Report {
    /// Redact all bank and storage errors, retaining only a valid local attempt ID.
    pub fn from_result<T>(
        result: std::result::Result<Result<T>, GuardError>,
        attempt: Option<&str>,
    ) -> Self {
        let state = match result {
            Err(_) => return Self::not_emitted(),
            Ok(Ok(_)) => State::ResponseUnverified,
            Ok(Err(_)) => State::OutcomeUnknown,
        };
        Self { state, attempt: attempt.filter(|id| id.len() == 32 && id.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))).map(str::to_owned),
            detail: "Bank acceptance/execution not established; possibly-not-emitted if the API stopped before submission. Replay blocked. Reconcile in the bank before explicit financial-review." }
    }
}
pub fn review_command() -> clap::Command {
    use clap::{Arg, Command};
    Command::new("financial-review")
        .about("Inspect or explicitly release a local replay barrier AFTER bank reconciliation; performs no bank request")
        .arg(Arg::new("attempt").long("attempt").requires("decision"))
        .arg(Arg::new("decision").long("decision").requires("attempt").value_parser(["not-applied", "operator-reviewed"]))
}

pub fn review(matches: &clap::ArgMatches) -> Result<()> {
    let mut session = open_at(&settings::storage_directory()?)?;
    match (
        matches.get_one::<String>("attempt"),
        matches.get_one::<String>("decision"),
    ) {
        (None, None) => {
            if let Some(entry) = session.unresolved()? {
                println!("financial-journal: unresolved attempt={}; possibly-not-emitted; reconcile before review", entry.id());
            } else {
                println!("financial-journal: clear; no bank outcome asserted");
            }
        }
        (Some(attempt), Some(decision)) => {
            let decision_value = match decision.as_str() {
                "not-applied" => ReviewDecision::NotApplied,
                "operator-reviewed" => ReviewDecision::OperatorReviewed,
                _ => anyhow::bail!("Invalid review decision"),
            };
            session
                .review(attempt, decision_value)
                .context("Local review refused")?;
            println!("financial-review: local barrier released; decision={decision}; no bank outcome asserted");
        }
        _ => anyhow::bail!("Review requires attempt and decision"),
    }
    Ok(())
}
#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use bourso_api::{replay_guard::ReplayGuard, OutcomeOperation as Operation};
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };
    const JOURNAL: &str = "replay-unresolved.json";
    struct TestSession(ReplayGuard);
    impl TestSession {
        fn at(path: &Path) -> Result<Self> {
            Ok(Self(ReplayGuard::open(path)?))
        }
        async fn invoke<F, Fut>(&mut self, op: Operation, call: F) -> Report
        where
            F: FnOnce() -> Fut,
            Fut: std::future::Future<Output = Result<()>>,
        {
            let result = self.0.invoke(op, call).await;
            Report::from_result(
                result,
                self.0.unresolved().ok().flatten().as_ref().map(|a| a.id()),
            )
        }
    }
    use futures_util::FutureExt;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            // Resolve only the existing system temp root (e.g. macOS /var -> /private/var).
            // Production guard paths must still reject symlink ancestors.
            let root = fs::canonicalize(std::env::temp_dir()).unwrap();
            let path = root.join(format!(
                "bourso-guard-{}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let mut builder = fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            builder.create(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[tokio::test]
    async fn responses_and_errors_both_preserve_replay_barrier_without_bank_data() {
        for success in [true, false] {
            let temp = Temp::new();
            let mut session = TestSession::at(&temp.0).unwrap();
            let report = session
                .invoke(Operation::Order, || async {
                    // Independent observable: journal exists BEFORE the fake API runs.
                    assert!(temp.0.join(JOURNAL).exists());
                    if success {
                        Ok(())
                    } else {
                        anyhow::bail!("SECRET-bank-id https://bank.invalid/raw?token=SECRET body")
                    }
                })
                .await;
            assert_eq!(
                report.state,
                if success {
                    State::ResponseUnverified
                } else {
                    State::OutcomeUnknown
                }
            );
            assert_eq!(report.exit_code(), if success { 3 } else { 4 });
            assert!(!format!("{report} {report:?}").contains("SECRET"));
            assert!(!fs::read_to_string(temp.0.join(JOURNAL))
                .unwrap()
                .contains("SECRET"));
            drop(session);
            assert!(
                Session::at(&temp.0).is_err(),
                "response/error must not clear journal"
            );
        }
    }

    #[tokio::test]
    async fn cancelled_future_keeps_journal_and_blocks_replay_after_lock_drop() {
        let temp = Temp::new();
        let mut session = TestSession::at(&temp.0).unwrap();
        let calls = AtomicUsize::new(0);
        let mut future = Box::pin(session.invoke(Operation::Transfer, || async {
            calls.fetch_add(1, Ordering::SeqCst);
            std::future::pending::<Result<()>>().await
        }));
        assert!(future.as_mut().now_or_never().is_none());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        drop(future);
        drop(session);
        assert!(Session::at(&temp.0).is_err());
    }

    #[test]
    fn concurrent_sessions_allow_only_one_holder_and_no_stale_lock_cleanup() {
        let temp = Temp::new();
        let first = Session::at(&temp.0).unwrap();
        let path = Arc::new(temp.0.clone());
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let path = path.clone();
                std::thread::spawn(move || assert!(Session::at(&path).is_err()))
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        drop(first);
        assert!(Session::at(&temp.0).is_ok());
        assert!(temp.0.join("replay.lock").exists());
    }

    #[tokio::test]
    async fn journal_creation_failure_does_not_invoke_fake_api() {
        let temp = Temp::new();
        let mut session = TestSession::at(&temp.0).unwrap();
        fs::create_dir(temp.0.join(JOURNAL)).unwrap();
        let report = session
            .invoke(Operation::Order, || async { panic!("must not invoke") })
            .await;
        assert_eq!(report.state, State::NotEmitted);
        assert!(temp.0.join(JOURNAL).is_dir());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unix_private_modes_and_readonly_journal_fail_closed() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let temp = Temp::new();
        let mut session = TestSession::at(&temp.0).unwrap();
        session.invoke(Operation::Order, || async { Ok(()) }).await;
        for (path, mode) in [
            (temp.0.clone(), 0o700),
            (temp.0.join(JOURNAL), 0o600),
            (temp.0.join("replay.lock"), 0o600),
        ] {
            assert_eq!(fs::metadata(path).unwrap().mode() & 0o777, mode);
        }
        drop(session);
        fs::set_permissions(temp.0.join(JOURNAL), fs::Permissions::from_mode(0o400)).unwrap();
        assert!(Session::at(&temp.0).is_err());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn readonly_directory_before_journal_write_prevents_invocation() {
        use std::os::unix::fs::PermissionsExt;
        let temp = Temp::new();
        let mut session = TestSession::at(&temp.0).unwrap();
        fs::set_permissions(&temp.0, fs::Permissions::from_mode(0o500)).unwrap();
        // A privileged user may bypass Unix mode checks. Verify the fixture,
        // rather than silently declaring a writable directory read-only.
        let probe = settings::create_private_file(&temp.0.join("permission-probe"));
        if probe.is_ok() {
            fs::set_permissions(&temp.0, fs::Permissions::from_mode(0o700)).unwrap();
            eprintln!("read-only fixture unavailable to privileged user; other journal failures still tested");
            return;
        }
        let report = session
            .invoke(Operation::Order, || async { panic!("must not invoke") })
            .await;
        fs::set_permissions(&temp.0, fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(report.state, State::NotEmitted);
        assert!(!temp.0.join(JOURNAL).exists());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_and_hardlink_journals_are_not_followed() {
        let temp = Temp::new();
        let outside = temp.0.join("outside");
        fs::write(&outside, "keep").unwrap();
        for hardlink in [false, true] {
            let directory = temp.0.join(if hardlink { "hard" } else { "sym" });
            fs::create_dir(&directory).unwrap();
            if hardlink {
                fs::hard_link(&outside, directory.join(JOURNAL)).unwrap();
            } else {
                std::os::unix::fs::symlink(&outside, directory.join(JOURNAL)).unwrap();
            }
            assert!(Session::at(&directory).is_err());
            assert_eq!(fs::read_to_string(&outside).unwrap(), "keep");
        }
    }

    #[test]
    fn abrupt_process_exit_preserves_unresolved_journal() {
        let temp = Temp::new();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "financial_guard::tests::crash_child",
                "--ignored",
            ])
            .env("BOURSO_GUARD_CRASH_FIXTURE", &temp.0)
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(23));
        assert!(Session::at(&temp.0).is_err());
    }

    fn run_reserve(guard: &mut ReplayGuard) {
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(guard.invoke(Operation::Order, || async {}))
            .unwrap();
    }
    #[test]
    #[ignore = "only spawned by abrupt_process_exit_preserves_unresolved_journal"]
    fn crash_child() {
        let path = std::env::var_os("BOURSO_GUARD_CRASH_FIXTURE").unwrap();
        let mut session = TestSession::at(Path::new(&path)).unwrap();
        crate::financial_guard::tests::run_reserve(&mut session.0);
        // process::exit skips destructors, modelling a process lost after journaling.
        std::process::exit(23);
    }
}
