//! Conservative CLI-only replay barrier; not provider idempotency or reconciliation.
use crate::settings;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fmt,
    fs::{self, File},
    future::Future,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const JOURNAL: &str = "financial-unresolved.json";

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Operation {
    Order,
    Transfer,
}

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

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    version: u8,
    attempt: String,
    operation: Operation,
}

pub fn is_financial(matches: &clap::ArgMatches) -> bool {
    matches!(
        matches.subcommand_name(),
        Some("trade" | "transfer" | "financial-review")
    )
}

/// The stable lock file is never deleted; dropping the handle releases only its
/// OS lock. The unresolved journal deliberately survives drop, panic and crash.
struct Guard {
    directory: PathBuf,
    _lock: File,
}
impl Drop for Guard {
    fn drop(&mut self) {
        // A concurrent spawn may inherit the file description before exec.
        // Explicitly unlock rather than relying on closing only our descriptor;
        // preserve both the stable lock inode and the unresolved journal.
        let _ = self._lock.unlock();
    }
}
impl Guard {
    fn open(directory: &Path) -> Result<Self> {
        settings::private_directory(directory)?;
        let lock = settings::open_private_file(&directory.join("financial.lock"))?;
        lock.try_lock()
            .map_err(|_| anyhow::anyhow!("Financial storage unavailable"))?;
        Ok(Self {
            directory: directory.to_owned(),
            _lock: lock,
        })
    }
    fn entry(&self) -> Result<Option<Entry>> {
        let file = match settings::open_existing_regular(&self.directory.join(JOURNAL), true) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        anyhow::ensure!(file.metadata()?.len() <= 256, "Invalid financial journal");
        let mut content = String::new();
        file.take(257).read_to_string(&mut content)?;
        let entry: Entry = serde_json::from_str(&content)?;
        anyhow::ensure!(
            entry.version == 1 && valid_id(&entry.attempt),
            "Invalid financial journal"
        );
        Ok(Some(entry))
    }
    fn ensure_clear(&self) -> Result<()> {
        if let Some(entry) = self.entry()? {
            return Err(Report {
                state: State::NotEmitted,
                attempt: Some(entry.attempt),
                detail: "This invocation blocked by unresolved prior attempt (possibly-not-emitted). Reconcile with the bank, then use financial-review; never retry automatically.",
            }.into());
        }
        Ok(())
    }
    fn begin(&self, operation: Operation) -> Result<String> {
        self.ensure_clear()?;
        let attempt = format!(
            "{:x}-{:x}",
            SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos(),
            std::process::id()
        );
        let entry = Entry {
            version: 1,
            attempt: attempt.clone(),
            operation,
        };
        let bytes = serde_json::to_vec(&entry)?;
        let mut file = settings::create_private_file(&self.directory.join(JOURNAL))?;
        // No fsync guarantee. Partial writes remain blocked; never unlink on error.
        file.write_all(&bytes)?;
        file.flush()?;
        Ok(attempt)
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
}

/// Acquired before authentication; persistence occurs only at invocation. Keeping
/// this handle across the await also excludes another process or review command.
pub struct Session(Guard);
impl Session {
    pub fn open() -> Result<Self> {
        Self::at(&settings::storage_directory()?)
    }
    fn at(directory: &Path) -> Result<Self> {
        let guard = Guard::open(directory)?;
        guard.ensure_clear()?;
        Ok(Self(guard))
    }
    /// Invoke the closure only after the unresolved entry is completely written.
    /// Ordinary errors are conservatively unknown: there is no final-POST hook.
    pub async fn invoke<F, Fut>(&self, operation: Operation, call: F) -> Report
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<()>>,
    {
        let attempt = match self.0.begin(operation) {
            Ok(attempt) => attempt,
            Err(_) => return Report::not_emitted(),
        };
        let state = match call().await {
            Ok(()) => State::ResponseUnverified,
            Err(_) => State::OutcomeUnknown,
        };
        Report {
            state,
            attempt: Some(attempt),
            detail: "Bank acceptance/execution not established; possibly-not-emitted if the API stopped before submission. Replay blocked. Reconcile in the bank before explicit financial-review.",
        }
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
    let directory = settings::storage_directory()?;
    let guard = Guard::open(&directory)?;
    let entry = guard.entry()?;
    match (
        matches.get_one::<String>("attempt"),
        matches.get_one::<String>("decision"),
    ) {
        (None, None) => {
            if let Some(entry) = entry {
                println!("financial-journal: unresolved attempt={}; possibly-not-emitted; reconcile before review", entry.attempt);
            } else {
                println!("financial-journal: clear; no bank outcome asserted");
            }
        }
        (Some(attempt), Some(decision)) => {
            let entry = entry.context("No unresolved attempt")?;
            anyhow::ensure!(
                valid_id(attempt) && *attempt == entry.attempt,
                "Attempt does not match"
            );
            anyhow::ensure!(
                matches!(decision.as_str(), "not-applied" | "operator-reviewed"),
                "Invalid review decision"
            );
            fs::remove_file(directory.join(JOURNAL))?;
            println!("financial-review: local barrier released; decision={decision}; no bank outcome asserted");
        }
        _ => anyhow::bail!("Review requires attempt and decision"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::FutureExt;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "bourso-guard-{}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
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
            let session = Session::at(&temp.0).unwrap();
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
        let session = Session::at(&temp.0).unwrap();
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
        assert!(temp.0.join("financial.lock").exists());
    }

    #[cfg(unix)]
    #[test]
    fn financial_owner_drop_releases_with_duplicate_alive() {
        use std::os::unix::fs::MetadataExt;
        let temp = Temp::new();
        let guard = Guard::open(&temp.0).unwrap();
        guard.begin(Operation::Order).unwrap();
        let journal = fs::read(temp.0.join(JOURNAL)).unwrap();
        let duplicate = guard._lock.try_clone().unwrap();
        let inode = duplicate.metadata().unwrap().ino();
        assert!(Guard::open(&temp.0).is_err());
        drop(guard);
        let reopened = Guard::open(&temp.0).expect("owner must unlock despite duplicate");
        assert_eq!(
            fs::metadata(temp.0.join("financial.lock")).unwrap().ino(),
            inode
        );
        assert_eq!(fs::read(temp.0.join(JOURNAL)).unwrap(), journal);
        assert!(reopened.ensure_clear().is_err());
        drop(duplicate);
        assert!(Guard::open(&temp.0).is_err());
        drop(reopened);
        assert!(Guard::open(&temp.0).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn financial_preparation_error_releases_with_duplicate_alive() {
        use std::os::unix::fs::MetadataExt;
        let temp = Temp::new();
        fs::write(temp.0.join(JOURNAL), b"invalid journal: preserve").unwrap();
        let mut duplicate = None;
        let result = (|| -> Result<Session> {
            let guard = Guard::open(&temp.0)?;
            duplicate = Some(guard._lock.try_clone()?);
            // Same post-acquisition validation and error exit as Session::at.
            guard.ensure_clear()?;
            Ok(Session(guard))
        })();
        assert!(result.is_err());
        let duplicate = duplicate.unwrap();
        let inode = duplicate.metadata().unwrap().ino();
        let reopened =
            Guard::open(&temp.0).expect("validation error must unlock despite duplicate");
        assert_eq!(
            fs::metadata(temp.0.join("financial.lock")).unwrap().ino(),
            inode
        );
        assert_eq!(
            fs::read(temp.0.join(JOURNAL)).unwrap(),
            b"invalid journal: preserve"
        );
        drop(duplicate);
        assert!(Guard::open(&temp.0).is_err());
        drop(reopened);
        assert!(
            Session::at(&temp.0).is_err(),
            "invalid journal still blocks replay"
        );
    }

    #[tokio::test]
    async fn journal_creation_failure_does_not_invoke_fake_api() {
        let temp = Temp::new();
        let session = Session::at(&temp.0).unwrap();
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
        let session = Session::at(&temp.0).unwrap();
        session.invoke(Operation::Order, || async { Ok(()) }).await;
        for (path, mode) in [
            (temp.0.clone(), 0o700),
            (temp.0.join(JOURNAL), 0o600),
            (temp.0.join("financial.lock"), 0o600),
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
        let session = Session::at(&temp.0).unwrap();
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

    #[test]
    #[ignore = "only spawned by abrupt_process_exit_preserves_unresolved_journal"]
    fn crash_child() {
        let path = std::env::var_os("BOURSO_GUARD_CRASH_FIXTURE").unwrap();
        let session = Session::at(Path::new(&path)).unwrap();
        session.0.begin(Operation::Order).unwrap();
        // process::exit skips destructors, modelling a process lost after journaling.
        std::process::exit(23);
    }
}
