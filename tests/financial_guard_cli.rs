use std::{fs, path::PathBuf, process::Command};

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        Self::new_at(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        )
    }
    fn new_at(nanos: u128) -> Self {
        // The clock can repeat, even on one thread; reserve a process-local suffix.
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let sequence = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        // Resolve only the existing system temp root (e.g. macOS /var -> /private/var).
        // Production guard paths must still reject symlink ancestors.
        let root = fs::canonicalize(std::env::temp_dir()).unwrap();
        let path = root.join(format!(
            "bourso-financial-{}-{}-{}",
            std::process::id(),
            nanos,
            sequence
        ));
        let builder = fs::DirBuilder::new();
        #[cfg(unix)]
        let builder = {
            use std::os::unix::fs::DirBuilderExt;
            let mut builder = builder;
            builder.mode(0o700);
            builder
        };
        builder.create(&path).unwrap();
        Self(path)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn financial_preflight_failure_is_redacted_and_visible_with_logs_off() {
    let temp = Temp::new();
    let secret = "SECRET-bank-id-url-body";
    let output = Command::new(env!("CARGO_BIN_EXE_bourso-cli"))
        .env("BOURSO_DATA_DIR", temp.0.join("data"))
        .env("RUST_LOG", "off")
        .args([
            "--credentials",
            temp.0.join(secret).to_str().unwrap(),
            "trade",
            "order",
            "new",
            "--side",
            "buy",
            "--account",
            "0123456789abcdef0123456789abcdef",
            "--symbol",
            secret,
            "--quantity",
            "1",
        ])
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.status.code(), Some(2), "{text}");
    assert!(text.contains("not-emitted"), "{text}");
    assert!(!text.contains(secret), "{text}");
    assert!(!temp.0.join("data/replay-unresolved.json").exists());
}

fn cli(temp: &Temp, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_bourso-cli"))
        .env("BOURSO_DATA_DIR", temp.0.join("data"))
        .env("RUST_LOG", "trace")
        .args(args)
        .output()
        .unwrap()
}
fn text(output: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}
#[test]
#[cfg(unix)]
fn explicit_review_requires_matching_attempt_and_decision_without_bank_work() {
    let temp = Temp::new();
    fs::create_dir(temp.0.join("data")).unwrap();
    let journal = temp.0.join("data/replay-unresolved.json");
    fs::write(
        &journal,
        r#"{"version":1,"attempt":"0123456789abcdef0123456789abcdef","operation":"order"}"#,
    )
    .unwrap();
    let output = cli(&temp, &["financial-review"]);
    assert!(output.status.success(), "{}", text(&output));
    assert!(text(&output).contains("unresolved attempt=0123456789abcdef0123456789abcdef"));
    for args in [
        vec![
            "financial-review",
            "--attempt",
            "bad",
            "--decision",
            "not-applied",
        ],
        vec![
            "financial-review",
            "--attempt",
            "0123456789abcdef0123456789abcdef",
        ],
        vec![
            "financial-review",
            "--attempt",
            "0123456789abcdef0123456789abcdef",
            "--decision",
            "SECRET-unknown",
        ],
    ] {
        let output = cli(&temp, &args);
        assert_eq!(output.status.code(), Some(2));
        assert!(journal.exists());
        assert!(!text(&output).contains("SECRET"));
    }
    for decision in ["not-applied", "operator-reviewed"] {
        fs::write(
            &journal,
            r#"{"version":1,"attempt":"0123456789abcdef0123456789abcdef","operation":"order"}"#,
        )
        .unwrap();
        let output = cli(
            &temp,
            &[
                "financial-review",
                "--attempt",
                "0123456789abcdef0123456789abcdef",
                "--decision",
                decision,
            ],
        );
        assert!(output.status.success(), "{}", text(&output));
        assert!(!journal.exists());
        assert!(text(&output).contains("no bank outcome asserted"));
    }
}

#[test]
fn corrupt_journal_cannot_disclose_arbitrary_content_or_be_automatically_cleared() {
    let temp = Temp::new();
    fs::create_dir(temp.0.join("data")).unwrap();
    let journal = temp.0.join("data/replay-unresolved.json");
    let secret = "SECRET-url-bank-account";
    fs::write(
        &journal,
        format!(r#"{{"version":1,"attempt":"{secret}","operation":"order"}}"#),
    )
    .unwrap();
    let output = cli(&temp, &["financial-review"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(!text(&output).contains(secret));
    assert!(journal.exists());
}

#[test]
fn financial_parser_errors_do_not_echo_values_even_with_trace_enabled() {
    let temp = Temp::new();
    let output = cli(
        &temp,
        &[
            "transfer",
            "--account",
            "SECRET-invalid-account",
            "--to",
            "SECRET-bank-destination",
            "--amount",
            "SECRET-invalid-amount",
        ],
    );
    assert_eq!(output.status.code(), Some(2));
    assert!(text(&output).contains("not-emitted"));
    assert!(!text(&output).contains("SECRET"));
    assert!(!temp.0.join("data/bourso.log").exists());
}

#[test]
#[cfg(unix)]
fn fake_results_are_visible_with_logging_off_and_remain_blocked() {
    for outcome in ["response", "error", "cancel"] {
        let temp = Temp::new();
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "fake_financial_child",
                "--ignored",
                "--nocapture",
            ])
            .env("BOURSO_DATA_DIR", temp.0.join("data"))
            .env("BOURSO_FAKE_OUTCOME", outcome)
            .env("RUST_LOG", "off")
            .output()
            .unwrap();
        let expected = if outcome == "response" {
            3
        } else if outcome == "error" {
            4
        } else {
            23
        };
        assert_eq!(output.status.code(), Some(expected), "{}", text(&output));
        if outcome != "cancel" {
            assert!(text(&output).contains(if outcome == "response" {
                "response-unverified"
            } else {
                "outcome-unknown"
            }));
        }
        assert!(!text(&output).contains("SECRET"));
        let journal = temp.0.join("data/replay-unresolved.json");
        assert!(journal.exists());
        // Real CLI sees the fake API's journal and refuses before authentication.
        let credentials = temp.0.join("credentials.json");
        fs::write(&credentials, r#"{"clientId":"fake","password":"fake"}"#).unwrap();
        let output = cli(
            &temp,
            &[
                "--credentials",
                credentials.to_str().unwrap(),
                "transfer",
                "--account",
                "0123456789abcdef0123456789abcdef",
                "--to",
                "fake",
                "--amount",
                "10",
            ],
        );
        assert_eq!(output.status.code(), Some(2), "{}", text(&output));
        assert!(text(&output).contains("unresolved prior attempt"));
        assert!(journal.exists());
    }
}

#[test]
#[ignore = "only spawned by fake_results_are_visible_with_logging_off_and_remain_blocked"]
#[cfg(unix)]
fn fake_financial_child() {
    use bourso_api::{replay_guard::ReplayGuard, OutcomeOperation as Operation};
    use bourso_cli::financial_guard::Report;
    let outcome = std::env::var("BOURSO_FAKE_OUTCOME").unwrap();
    let mut session = ReplayGuard::open(std::path::PathBuf::from(
        std::env::var_os("BOURSO_DATA_DIR").unwrap(),
    ))
    .unwrap();
    let report = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(session.invoke(Operation::Transfer, || async {
            match outcome.as_str() {
                "response" => Ok(()),
                "error" => anyhow::bail!("SECRET-bank-id https://bank.invalid/SECRET body"),
                "cancel" => std::process::exit(23),
                _ => panic!("invalid fixture"),
            }
        }));
    let report = Report::from_result(
        report,
        session.unresolved().unwrap().as_ref().map(|a| a.id()),
    );
    report.print();
    std::process::exit(report.exit_code().into());
}

#[test]
fn fixed_clock_directories_are_unique_across_repeated_and_concurrent_calls() {
    // Retain every directory until all workers finish, including repeated calls
    // on each worker: a thread identifier alone cannot satisfy this regression.
    let mut directories = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..8)
            .map(|_| scope.spawn(|| (0..8).map(|_| Temp::new_at(0)).collect::<Vec<_>>()))
            .collect();
        workers
            .into_iter()
            .flat_map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>()
    });
    let paths: std::collections::HashSet<_> = directories.iter().map(|dir| &dir.0).collect();
    assert_eq!(paths.len(), 64);
    for (index, dir) in directories.iter().enumerate() {
        assert!(dir.0.is_dir());
        // Exclusive file creation also detects accidental directory reuse.
        let mut marker = fs::File::create_new(dir.0.join("owner")).unwrap();
        std::io::Write::write_all(&mut marker, index.to_string().as_bytes()).unwrap();
    }
    let removed = directories.pop().unwrap();
    let removed_path = removed.0.clone();
    drop(removed);
    assert!(!removed_path.exists());
    for (index, dir) in directories.iter().enumerate() {
        assert_eq!(
            fs::read_to_string(dir.0.join("owner")).unwrap(),
            index.to_string()
        );
    }
}

#[test]
#[cfg(unix)]
fn unexpected_legacy_journal_blocks_review_and_submission_without_deletion() {
    let temp = Temp::new();
    fs::create_dir(temp.0.join("data")).unwrap();
    let journal = temp.0.join("data/financial-unresolved.json");
    fs::write(&journal, b"SECRET legacy journal").unwrap();
    for args in [
        vec!["financial-review"],
        vec![
            "financial-review",
            "--attempt",
            "0123456789abcdef0123456789abcdef",
            "--decision",
            "not-applied",
        ],
    ] {
        let output = cli(&temp, &args);
        assert_eq!(output.status.code(), Some(2));
        assert!(text(&output).contains("Unexpected legacy financial journal"));
        assert!(!text(&output).contains("SECRET"));
        assert_eq!(fs::read(&journal).unwrap(), b"SECRET legacy journal");
        assert!(!temp.0.join("data/replay-unresolved.json").exists());
    }
}

#[test]
#[cfg(unix)]
fn cli_and_api_contend_on_the_same_stable_lock_before_authentication() {
    let temp = Temp::new();
    let session = bourso_api::FinancialSession::open(temp.0.join("data")).unwrap();
    let output = cli(&temp, &["financial-review"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(!temp.0.join("data/financial.lock").exists());
    drop(session);
    assert!(cli(&temp, &["financial-review"]).status.success());
}

#[test]
#[cfg(not(unix))]
fn unsupported_financial_review_refuses_without_creating_storage() {
    let temp = Temp::new();
    let output = cli(&temp, &["financial-review"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(text(&output).contains("not-emitted"));
    assert!(!temp.0.join("data").exists());
}
