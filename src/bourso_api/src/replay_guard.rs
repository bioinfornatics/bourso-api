//! Opt-in, conservative local replay barrier for orders and immediate transfers.
//!
//! All cooperating callers must explicitly choose the same absolute directory,
//! with trusted ancestors on a local Unix filesystem. Raw bank APIs bypass this
//! barrier; the CLI and DCA journals are separate. There is no global storage.
//!
//! The lock is held for this handle's lifetime. A reservation is written and
//! flushed before the closure is called, and is NEVER automatically cleared,
//! including after success, failure, panic, cancellation or process death. A
//! reservation may therefore represent an operation that was never emitted.
//! Review requires independent operator reconciliation; it does not contact the
//! bank, assert a bank outcome or submit a new operation.
//!
//! This is not provider idempotency, exactly-once execution, cancellation,
//! reconciliation, multi-machine coordination or network-filesystem support.
//! Writes are not fsynced: power-loss durability is not promised. Malicious
//! same-user path replacement is outside the trusted-directory model. Non-Unix
//! storage is refused (Windows ACLs are not established).
//!
//! ```no_run
//! use bourso_api::{replay_guard::{ReplayGuard, ReviewDecision}, OutcomeOperation};
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let mut guard = ReplayGuard::open("/trusted/private/bourso-library")?;
//! let original_result = guard.invoke(OutcomeOperation::Order, || async {
//!     // Invoke order/order_detailed here, or construct AND fully consume a
//!     // transfer_funds/transfer_money stream inside this async block.
//!     Ok::<_, anyhow::Error>("original result")
//! }).await?;
//! // The inner result and its downcastable errors are untouched.
//! let _ = original_result;
//! let attempt = guard.unresolved()?.expect("reservation retained");
//! // ONLY after independent operator review; never automate this step:
//! guard.review(attempt.id(), ReviewDecision::OperatorReviewed)?;
//! # Ok(()) }
//! ```
use crate::OutcomeOperation;
use serde::{Deserialize, Serialize};
use std::{
    fmt,
    fs::{self, File, OpenOptions},
    future::Future,
    io::{Read, Write},
    path::{Path, PathBuf},
};

#[cfg(unix)]
use std::path::Component;

const JOURNAL: &str = "replay-unresolved.json";
const LOCK: &str = "replay.lock";

/// Fixed local diagnostics. No OS error, path, bank response or caller data is
/// retained, even in Debug or the error source chain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuardError {
    InvalidPath,
    UnsupportedPlatform,
    StorageUnavailable,
    Locked,
    InvalidJournal,
    Unresolved,
    ReviewMismatch,
}
impl fmt::Display for GuardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidPath => "Replay guard requires an explicit absolute trusted directory",
            Self::UnsupportedPlatform => "Replay guard storage is supported only on Unix",
            Self::StorageUnavailable => "Replay guard storage unavailable; no invocation started",
            Self::Locked => "Replay guard domain is locked or cannot be locked",
            Self::InvalidJournal => "Replay journal invalid or unreadable; manual investigation required",
            Self::Unresolved => "Unresolved replay attempt; possibly not emitted; do not retry automatically; independent review required",
            Self::ReviewMismatch => "Replay review requires the exact current attempt identifier",
        })
    }
}
impl std::error::Error for GuardError {}

/// Operator assertion only, NOT a verified bank result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReviewDecision {
    NotApplied,
    OperatorReviewed,
}

/// Minimal local reservation. The generated ID is not a provider reference or
/// idempotency key. No account, amount, token, URL or response is recorded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attempt {
    id: String,
    operation: OutcomeOperation,
}
impl Attempt {
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn operation(&self) -> OutcomeOperation {
        self.operation
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum Operation {
    Order,
    Transfer,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    version: u8,
    attempt: String,
    operation: Operation,
}

/// Stable exclusive local lock. Never delete/replace `replay.lock` while any
/// cooperating process may use this domain. Drop releases ONLY the OS lock.
/// Methods perform small blocking filesystem operations on the calling thread.
pub struct ReplayGuard {
    directory: PathBuf,
    lock: File,
    #[cfg(all(test, unix))]
    fault: Option<Fault>,
}
impl Drop for ReplayGuard {
    fn drop(&mut self) {
        // Explicitly unlock: another thread's process spawn may transiently
        // inherit the file description before close-on-exec. Closing only our
        // descriptor could otherwise keep this domain locked after owner drop.
        // Never remove the stable lock file or the unresolved journal.
        let _ = self.lock.unlock();
    }
}
impl fmt::Debug for ReplayGuard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReplayGuard").finish_non_exhaustive()
    }
}
impl ReplayGuard {
    /// Create/tighten a private directory and stable lock (0700/0600). Parent
    /// directories must already exist. Existing reservations remain inspectable;
    /// invoking with any existing or ambiguous journal is forbidden.
    #[cfg(unix)]
    pub fn open(directory: impl AsRef<Path>) -> Result<Self, GuardError> {
        let directory = directory.as_ref();
        if !directory.is_absolute()
            || directory
                .components()
                .any(|c| matches!(c, Component::ParentDir))
        {
            return Err(GuardError::InvalidPath);
        }
        private_directory(directory).map_err(|_| GuardError::StorageUnavailable)?;
        let path = directory.join(LOCK);
        let lock = match existing_file(&path) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // A concurrent creator may win. Open its stable inode, never
                // truncate or replace it, then contend on the same OS lock.
                match create_file(&path) {
                    Ok(file) => file,
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                        existing_file(&path).map_err(|_| GuardError::StorageUnavailable)?
                    }
                    Err(_) => return Err(GuardError::StorageUnavailable),
                }
            }
            Err(_) => return Err(GuardError::StorageUnavailable),
        };
        lock.try_lock().map_err(|_| GuardError::Locked)?;
        Ok(Self {
            directory: directory.to_owned(),
            lock,
            #[cfg(all(test, unix))]
            fault: None,
        })
    }

    /// Refuse unsupported platforms before inspecting the path or touching storage.
    #[cfg(not(unix))]
    pub fn open(_directory: impl AsRef<Path>) -> Result<Self, GuardError> {
        Err(GuardError::UnsupportedPlatform)
    }

    fn verify_lock(&self) -> Result<(), GuardError> {
        verify_file(&self.directory.join(LOCK), &self.lock)
            .map_err(|_| GuardError::StorageUnavailable)
    }

    /// Inspect under the same lock used for invocation and review. Corrupt,
    /// partial, unreadable and unsafe entries fail closed and are left intact.
    pub fn unresolved(&self) -> Result<Option<Attempt>, GuardError> {
        self.verify_lock()?;
        let file = match existing_file(&self.directory.join(JOURNAL)) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(GuardError::InvalidJournal),
        };
        let mut bytes = Vec::new();
        file.take(257)
            .read_to_end(&mut bytes)
            .map_err(|_| GuardError::InvalidJournal)?;
        if bytes.len() > 256 {
            return Err(GuardError::InvalidJournal);
        }
        let entry: Entry =
            serde_json::from_slice(&bytes).map_err(|_| GuardError::InvalidJournal)?;
        if entry.version != 1
            || entry.attempt.len() != 32
            || !entry
                .attempt
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(GuardError::InvalidJournal);
        }
        Ok(Some(Attempt {
            id: entry.attempt,
            operation: match entry.operation {
                Operation::Order => OutcomeOperation::Order,
                Operation::Transfer => OutcomeOperation::Transfer,
            },
        }))
    }

    fn reserve(&mut self, operation: OutcomeOperation) -> Result<(), GuardError> {
        if self.unresolved()?.is_some() {
            return Err(GuardError::Unresolved);
        }
        let entry = Entry {
            version: 1,
            attempt: format!("{:032x}", rand::random::<u128>()),
            operation: match operation {
                OutcomeOperation::Order => Operation::Order,
                OutcomeOperation::Transfer => Operation::Transfer,
            },
        };
        let bytes = serde_json::to_vec(&entry).map_err(|_| GuardError::StorageUnavailable)?;
        let mut file = create_file(&self.directory.join(JOURNAL))
            .map_err(|_| GuardError::StorageUnavailable)?;
        // Failed writes/flushes leave the created journal in place. No cleanup,
        // no retry, and no claim of power-loss durability.
        #[cfg(all(test, unix))]
        if matches!(self.fault, Some(Fault::Write)) {
            file.write_all(&bytes[..1])
                .map_err(|_| GuardError::StorageUnavailable)?;
            return Err(GuardError::StorageUnavailable);
        }
        file.write_all(&bytes)
            .map_err(|_| GuardError::StorageUnavailable)?;
        #[cfg(all(test, unix))]
        if matches!(self.fault, Some(Fault::Flush)) {
            return Err(GuardError::StorageUnavailable);
        }
        file.flush().map_err(|_| GuardError::StorageUnavailable)
    }

    /// Reserve BEFORE calling `call`, then preserve its output without wrapping,
    /// classifying or logging it. For `Future<Output = Result<T, E>>`, the return
    /// is `Result<Result<T, E>, GuardError>`; only the outer error is local.
    /// Construct AND fully consume streams inside the closure's future. Returning
    /// an unpolled stream transfers responsibility to the caller. The mutable
    /// borrow prevents review through this handle while the future is alive.
    pub async fn invoke<F, Fut>(
        &mut self,
        operation: OutcomeOperation,
        call: F,
    ) -> Result<Fut::Output, GuardError>
    where
        F: FnOnce() -> Fut,
        Fut: Future,
    {
        self.reserve(operation)?;
        Ok(call().await)
    }

    /// Release ONLY the exact valid reservation after an explicit operator
    /// decision. Performs no bank call and never invokes or retries an operation.
    /// Invalid journals cannot be cleared by this method.
    pub fn review(&mut self, attempt_id: &str, decision: ReviewDecision) -> Result<(), GuardError> {
        let current = self.unresolved()?.ok_or(GuardError::ReviewMismatch)?;
        if current.id != attempt_id {
            return Err(GuardError::ReviewMismatch);
        }
        match decision {
            ReviewDecision::NotApplied | ReviewDecision::OperatorReviewed => {}
        }
        fs::remove_file(self.directory.join(JOURNAL)).map_err(|_| GuardError::StorageUnavailable)
    }
}

fn unsafe_entry() -> std::io::Error {
    std::io::Error::other("Unsafe replay storage")
}
#[cfg(unix)]
fn private_directory(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    // Refuse existing symlink ancestors, not just the final directory.
    for ancestor in path.ancestors().skip(1) {
        if !fs::symlink_metadata(ancestor)?.is_dir() {
            return Err(unsafe_entry());
        }
    }
    match fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => (),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
        Err(e) => return Err(e),
    }
    if !fs::symlink_metadata(path)?.is_dir() {
        return Err(unsafe_entry());
    }
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
}
fn regular_metadata(path: &Path) -> std::io::Result<fs::Metadata> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() {
        return Err(unsafe_entry());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err(unsafe_entry());
        }
    }
    Ok(metadata)
}
fn verify_file(path: &Path, file: &File) -> std::io::Result<()> {
    let metadata = regular_metadata(path)?;
    let opened = file.metadata()?;
    if !opened.is_file() {
        return Err(unsafe_entry());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.dev() != opened.dev() || metadata.ino() != opened.ino() || opened.nlink() != 1 {
            return Err(unsafe_entry());
        }
    }
    #[cfg(not(unix))]
    let _ = metadata;
    Ok(())
}
fn private_mode(file: &File) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    let _ = file;
    Ok(())
}
fn create_file(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path)?;
    verify_file(path, &file)?;
    private_mode(&file)?;
    Ok(file)
}
fn existing_file(path: &Path) -> std::io::Result<File> {
    regular_metadata(path)?;
    let file = OpenOptions::new().read(true).write(true).open(path)?;
    verify_file(path, &file)?;
    private_mode(&file)?;
    Ok(file)
}
#[cfg(all(test, unix))]
enum Fault {
    Write,
    Flush,
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn owner_drop_releases_lock_even_with_inherited_descriptor() {
        // Resolve only the existing OS temp root, never the guard directory.
        let root = std::env::temp_dir().canonicalize().unwrap();
        let path = root.join(format!(
            "bourso-api-replay-inherited-{:032x}",
            rand::random::<u128>()
        ));
        let guard = ReplayGuard::open(&path).unwrap();
        // try_clone duplicates the open file description, just like a transient
        // descriptor inherited by another thread's fork before close-on-exec.
        // The guard itself does not expose or clone its descriptor publicly.
        let inherited = guard.lock.try_clone().unwrap();
        drop(guard);
        let reopened = ReplayGuard::open(&path)
            .expect("owner drop releases domain even before inherited descriptor closes");
        drop(inherited);
        drop(reopened);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn injected_write_and_flush_failures_never_call_and_never_clear() {
        for fault in [Fault::Write, Fault::Flush] {
            // Resolve only the existing OS temp root, never the guard directory.
            let root = std::env::temp_dir().canonicalize().unwrap();
            let path = root.join(format!(
                "bourso-api-replay-fault-{:032x}",
                rand::random::<u128>()
            ));
            let mut guard = ReplayGuard::open(&path).unwrap();
            guard.fault = Some(fault);
            crate::outcome::tests::run(async {
                assert_eq!(
                    guard
                        .invoke(OutcomeOperation::Order, || {
                            panic!("persistence failure must not construct the future");
                            #[allow(unreachable_code)]
                            async {}
                        })
                        .await,
                    Err(GuardError::StorageUnavailable)
                );
            });
            let bytes = fs::read(path.join(JOURNAL)).unwrap();
            assert!(!bytes.is_empty());
            drop(guard);
            let mut reopened = ReplayGuard::open(&path).unwrap();
            crate::outcome::tests::run(async {
                assert!(reopened
                    .invoke(OutcomeOperation::Transfer, || {
                        panic!("failed persistence remains unresolved");
                        #[allow(unreachable_code)]
                        async {}
                    })
                    .await
                    .is_err());
            });
            assert_eq!(fs::read(path.join(JOURNAL)).unwrap(), bytes);
            drop(reopened);
            fs::remove_dir_all(path).unwrap();
        }
    }
}
