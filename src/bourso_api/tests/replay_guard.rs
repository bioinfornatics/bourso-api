#![cfg(unix)]
use bourso_api::{replay_guard::ReplayGuard, OutcomeOperation};
use std::{fs, path::PathBuf};

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        // Resolve only the existing OS temp root, never the guard directory.
        let root = std::env::temp_dir().canonicalize().unwrap();
        let path = root.join(format!(
            "bourso-api-guard-{}-{:032x}",
            std::process::id(),
            rand::random::<u128>()
        ));
        Self(path)
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
fn reservation_precedes_closure_and_original_output_is_preserved() {
    run(async {
        let temp = Temp::new();
        let mut guard = ReplayGuard::open(&temp.0).unwrap();
        let value = guard
            .invoke(OutcomeOperation::Order, || {
                let bytes = fs::read(temp.0.join("replay-unresolved.json"))
                    .expect("complete reservation BEFORE closure construction");
                let entry: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(entry["version"], 1);
                assert_eq!(entry["operation"], "order");
                assert_eq!(entry["attempt"].as_str().unwrap().len(), 32);
                async { String::from("original-private-result") }
            })
            .await
            .unwrap();
        assert_eq!(value, "original-private-result");
        drop(guard);
        let mut reopened = ReplayGuard::open(&temp.0).unwrap();
        assert!(reopened
            .invoke(OutcomeOperation::Transfer, || async {
                panic!("must not reissue")
            })
            .await
            .is_err());
    });
}

use bourso_api::replay_guard::{GuardError, ReviewDecision};
use futures_util::FutureExt;
use std::{
    os::unix::fs::{symlink, MetadataExt, PermissionsExt},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Barrier,
    },
    time::{Duration, Instant},
};
const JOURNAL: &str = "replay-unresolved.json";
const LOCK: &str = "replay.lock";
const SECRET: &str = "SENTINEL_BANK_TOKEN_ACCOUNT_URL_BODY";

fn blocked(guard: &mut ReplayGuard) {
    run(async {
        assert!(guard
            .invoke(OutcomeOperation::Order, || {
                panic!("blocked invocation constructed its future");
                #[allow(unreachable_code)]
                async {}
            })
            .await
            .is_err());
    });
}

#[test]
fn original_error_identity_survives_and_review_is_exact_and_separate() {
    let temp = Temp::new();
    let mut guard = ReplayGuard::open(&temp.0).unwrap();
    let original = anyhow::Error::new(std::io::Error::other(SECRET)).context("caller context");
    let pointer = original.downcast_ref::<std::io::Error>().unwrap() as *const _;
    run(async {
        let result = guard
            .invoke(OutcomeOperation::Transfer, || async {
                Err::<(), _>(original)
            })
            .await
            .unwrap();
        let error = result.unwrap_err();
        assert_eq!(
            pointer,
            error.downcast_ref::<std::io::Error>().unwrap() as *const _
        );
        assert_eq!(
            error.downcast_ref::<std::io::Error>().unwrap().to_string(),
            SECRET
        );
    });
    let attempt = guard.unresolved().unwrap().unwrap();
    assert_eq!(attempt.operation(), OutcomeOperation::Transfer);
    let bytes = fs::read(temp.0.join(JOURNAL)).unwrap();
    assert_eq!(
        guard.review(SECRET, ReviewDecision::NotApplied),
        Err(GuardError::ReviewMismatch)
    );
    assert_eq!(fs::read(temp.0.join(JOURNAL)).unwrap(), bytes);
    blocked(&mut guard);
    let lock_inode = fs::metadata(temp.0.join(LOCK)).unwrap().ino();
    guard
        .review(attempt.id(), ReviewDecision::OperatorReviewed)
        .unwrap();
    assert!(guard.unresolved().unwrap().is_none());
    assert_eq!(
        guard.review(attempt.id(), ReviewDecision::NotApplied),
        Err(GuardError::ReviewMismatch)
    );
    assert_eq!(fs::metadata(temp.0.join(LOCK)).unwrap().ino(), lock_inode);
    drop(guard);
    let mut guard = ReplayGuard::open(&temp.0).unwrap();
    run(async {
        assert_eq!(
            guard
                .invoke(OutcomeOperation::Order, || async { 42 })
                .await
                .unwrap(),
            42
        );
    });
    let next = guard.unresolved().unwrap().unwrap();
    assert_ne!(attempt.id(), next.id());
    guard.review(next.id(), ReviewDecision::NotApplied).unwrap();
}

#[test]
fn cancellation_and_panic_keep_reservation_but_unpolled_future_does_not_reserve() {
    for panic_in_closure in [false, true] {
        let temp = Temp::new();
        let mut guard = ReplayGuard::open(&temp.0).unwrap();
        drop(guard.invoke(OutcomeOperation::Order, || async {}));
        assert!(guard.unresolved().unwrap().is_none());
        if panic_in_closure {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run(async {
                    let _ = guard
                        .invoke(OutcomeOperation::Order, || {
                            panic!("synthetic caller panic");
                            #[allow(unreachable_code)]
                            async {}
                        })
                        .await;
                })
            }));
            assert!(result.is_err());
        } else {
            let mut pending =
                Box::pin(guard.invoke(OutcomeOperation::Transfer, std::future::pending::<()>));
            assert!(pending.as_mut().now_or_never().is_none());
            drop(pending);
        }
        let before = fs::read(temp.0.join(JOURNAL)).unwrap();
        drop(guard);
        let mut guard = ReplayGuard::open(&temp.0).unwrap();
        blocked(&mut guard);
        assert_eq!(fs::read(temp.0.join(JOURNAL)).unwrap(), before);
    }
}

#[test]
fn concurrent_instances_share_stable_lock_and_one_invocation() {
    let temp = Temp::new();
    let first = ReplayGuard::open(&temp.0).unwrap();
    assert!(matches!(
        ReplayGuard::open(&temp.0),
        Err(GuardError::Locked)
    ));
    let inode = fs::metadata(temp.0.join(LOCK)).unwrap().ino();
    drop(first);
    let barrier = Arc::new(Barrier::new(12));
    let calls = Arc::new(AtomicUsize::new(0));
    let threads: Vec<_> = (0..12)
        .map(|_| {
            let path = temp.0.clone();
            let barrier = barrier.clone();
            let calls = calls.clone();
            std::thread::spawn(move || {
                barrier.wait();
                if let Ok(mut guard) = ReplayGuard::open(path) {
                    run(async {
                        let _ = guard
                            .invoke(OutcomeOperation::Order, || {
                                calls.fetch_add(1, Ordering::SeqCst);
                                async {}
                            })
                            .await;
                    });
                }
            })
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(fs::metadata(temp.0.join(LOCK)).unwrap().ino(), inode);
}

#[test]
fn invalid_and_ambiguous_storage_never_calls_or_clears() {
    for body in ["", "{", "null", "[]", "SENTINEL_BANK_TOKEN_ACCOUNT_URL_BODY", "{\"version\":2,\"operation\":\"order\",\"attempt\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"}", "{\"version\":1,\"operation\":\"order\",\"attempt\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",\"secret\":1}", "{\"version\":1,\"operation\":\"cancel\",\"attempt\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"}", "{\"version\":1,\"operation\":\"order\",\"attempt\":\"SENTINEL\"}"] {
        let temp = Temp::new();
        let mut guard = ReplayGuard::open(&temp.0).unwrap();
        fs::write(temp.0.join(JOURNAL), body).unwrap();
        blocked(&mut guard);
        assert_eq!(guard.unresolved(), Err(GuardError::InvalidJournal));
        assert!(guard.review("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", ReviewDecision::NotApplied).is_err());
        drop(guard);
        assert_eq!(fs::read(temp.0.join(JOURNAL)).unwrap(), body.as_bytes());
    }
    for bytes in [vec![b' '; 257], vec![255u8, 254u8]] {
        let temp = Temp::new();
        let mut guard = ReplayGuard::open(&temp.0).unwrap();
        fs::write(temp.0.join(JOURNAL), &bytes).unwrap();
        blocked(&mut guard);
        assert_eq!(fs::read(temp.0.join(JOURNAL)).unwrap(), bytes);
    }
    let temp = Temp::new();
    let mut guard = ReplayGuard::open(&temp.0).unwrap();
    fs::create_dir(temp.0.join(JOURNAL)).unwrap();
    blocked(&mut guard);
    assert!(temp.0.join(JOURNAL).is_dir());
}

#[test]
fn absolute_private_storage_and_unsafe_links_are_checked() {
    for path in ["", ".", "relative", "/tmp/../SENTINEL"] {
        assert!(matches!(
            ReplayGuard::open(path),
            Err(GuardError::InvalidPath)
        ));
    }
    let temp = Temp::new();
    fs::create_dir(&temp.0).unwrap();
    fs::set_permissions(&temp.0, fs::Permissions::from_mode(0o777)).unwrap();
    let mut guard = ReplayGuard::open(&temp.0).unwrap();
    run(async {
        guard
            .invoke(OutcomeOperation::Order, || async {})
            .await
            .unwrap();
    });
    for (path, mode) in [
        (&temp.0, 0o700),
        (&temp.0.join(LOCK), 0o600),
        (&temp.0.join(JOURNAL), 0o600),
    ] {
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            mode
        );
    }
    drop(guard);
    let alias = Temp::new();
    symlink(&temp.0, &alias.0).unwrap();
    assert!(ReplayGuard::open(&alias.0).is_err());
    assert!(ReplayGuard::open(alias.0.join("child")).is_err());
    fs::remove_file(&alias.0).unwrap();
    for name in [LOCK, JOURNAL] {
        for hard in [false, true] {
            let temp = Temp::new();
            let guard = ReplayGuard::open(&temp.0).unwrap();
            drop(guard);
            let target = temp.0.join("SENTINEL_PRIVATE_TARGET");
            fs::write(&target, SECRET).unwrap();
            let path = temp.0.join(name);
            if path.exists() {
                fs::remove_file(&path).unwrap();
            }
            if hard {
                fs::hard_link(&target, &path).unwrap();
            } else {
                symlink(&target, &path).unwrap();
            }
            match ReplayGuard::open(&temp.0) {
                Ok(mut guard) => {
                    assert_eq!(name, JOURNAL);
                    blocked(&mut guard);
                }
                Err(_) => assert_eq!(name, LOCK),
            }
            assert_eq!(fs::read_to_string(target).unwrap(), SECRET);
            assert!(fs::symlink_metadata(path).is_ok());
        }
    }
}

#[test]
fn diagnostics_and_journal_never_capture_sensitive_values() {
    let temp = Temp::new();
    let path = temp.0.join(SECRET);
    fs::create_dir(&temp.0).unwrap();
    let mut guard = ReplayGuard::open(&path).unwrap();
    run(async {
        assert_eq!(
            guard
                .invoke(OutcomeOperation::Order, || async { SECRET })
                .await
                .unwrap(),
            SECRET
        );
    });
    let attempt = guard.unresolved().unwrap().unwrap();
    let mut sink = format!("{guard:?} {guard:#?} {attempt:?}");
    sink.push_str(&fs::read_to_string(path.join(JOURNAL)).unwrap());
    for error in [
        GuardError::InvalidPath,
        GuardError::UnsupportedPlatform,
        GuardError::StorageUnavailable,
        GuardError::Locked,
        GuardError::InvalidJournal,
        GuardError::Unresolved,
        GuardError::ReviewMismatch,
    ] {
        sink.push_str(&format!("{error} {error:#} {error:?} {error:#?}"));
        assert!(std::error::Error::source(&error).is_none());
    }
    assert!(!sink.contains(SECRET));
    assert!(!sink.contains(path.to_str().unwrap()));
    let json: serde_json::Value =
        serde_json::from_slice(&fs::read(path.join(JOURNAL)).unwrap()).unwrap();
    let mut keys: Vec<_> = json
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort();
    assert_eq!(keys, ["attempt", "operation", "version"]);
}

// Executed only as an isolated subprocess. The parent kills this owner before
// it can construct any financial API, after the reservation has been flushed.
#[test]
#[ignore]
fn crash_child_before_send() {
    let path = PathBuf::from(std::env::var_os("BOURSO_REPLAY_TEST_CHILD").unwrap());
    let mut guard = ReplayGuard::open(&path).unwrap();
    run(async {
        let _ = guard
            .invoke(OutcomeOperation::Transfer, || {
                fs::write(path.join("ready"), b"ready").unwrap();
                std::future::pending::<()>()
            })
            .await;
    });
}

struct Child(std::process::Child);
impl Drop for Child {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[test]
fn process_crash_before_send_retains_barrier_and_blocks_concurrent_review() {
    let temp = Temp::new();
    let mut child = Child(
        std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "crash_child_before_send", "--ignored"])
            .env("BOURSO_REPLAY_TEST_CHILD", &temp.0)
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while !temp.0.join("ready").exists() {
        assert!(child.0.try_wait().unwrap().is_none(), "child exited early");
        assert!(Instant::now() < deadline, "child readiness timeout");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(matches!(
        ReplayGuard::open(&temp.0),
        Err(GuardError::Locked)
    ));
    let bytes = fs::read(temp.0.join(JOURNAL)).unwrap();
    child.0.kill().unwrap();
    assert!(!child.0.wait().unwrap().success());
    let mut reopened = ReplayGuard::open(&temp.0).unwrap();
    blocked(&mut reopened);
    assert_eq!(fs::read(temp.0.join(JOURNAL)).unwrap(), bytes);
    let attempt = reopened.unresolved().unwrap().unwrap();
    reopened
        .review(attempt.id(), ReviewDecision::OperatorReviewed)
        .unwrap();
}

#[test]
fn unreadable_journal_and_creation_failure_do_not_invoke() {
    let temp = Temp::new();
    let mut guard = ReplayGuard::open(&temp.0).unwrap();
    let journal = temp.0.join(JOURNAL);
    fs::write(&journal, b"private unreadable data").unwrap();
    fs::set_permissions(&journal, fs::Permissions::from_mode(0o000)).unwrap();
    // This Linux test requires an unprivileged runner, not root/CAP_DAC_OVERRIDE.
    assert!(
        fs::read(&journal).is_err(),
        "unreadable fixture requires unprivileged runner"
    );
    assert_eq!(guard.unresolved(), Err(GuardError::InvalidJournal));
    blocked(&mut guard);
    fs::set_permissions(&journal, fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(fs::read(&journal).unwrap(), b"private unreadable data");
    fs::remove_file(&journal).unwrap();
    fs::set_permissions(&temp.0, fs::Permissions::from_mode(0o500)).unwrap();
    run(async {
        assert_eq!(
            guard
                .invoke(OutcomeOperation::Order, || {
                    panic!("failed journal creation must not invoke");
                    #[allow(unreachable_code)]
                    async {}
                })
                .await,
            Err(GuardError::StorageUnavailable)
        );
    });
    assert!(!journal.exists());
    fs::set_permissions(&temp.0, fs::Permissions::from_mode(0o700)).unwrap();
}
