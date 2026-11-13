use std::{fs, path::PathBuf, process::Command};

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "bourso-financial-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
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
    assert!(!temp.0.join("data/financial-unresolved.json").exists());
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
fn explicit_review_requires_matching_attempt_and_decision_without_bank_work() {
    let temp = Temp::new();
    fs::create_dir(temp.0.join("data")).unwrap();
    let journal = temp.0.join("data/financial-unresolved.json");
    fs::write(
        &journal,
        r#"{"version":1,"attempt":"1234-abcd","operation":"order"}"#,
    )
    .unwrap();
    let output = cli(&temp, &["financial-review"]);
    assert!(output.status.success(), "{}", text(&output));
    assert!(text(&output).contains("unresolved attempt=1234-abcd"));
    for args in [
        vec![
            "financial-review",
            "--attempt",
            "bad",
            "--decision",
            "not-applied",
        ],
        vec!["financial-review", "--attempt", "1234-abcd"],
        vec![
            "financial-review",
            "--attempt",
            "1234-abcd",
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
            r#"{"version":1,"attempt":"1234-abcd","operation":"order"}"#,
        )
        .unwrap();
        let output = cli(
            &temp,
            &[
                "financial-review",
                "--attempt",
                "1234-abcd",
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
    let journal = temp.0.join("data/financial-unresolved.json");
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
        let journal = temp.0.join("data/financial-unresolved.json");
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
fn fake_financial_child() {
    use bourso_cli::financial_guard::{Operation, Session};
    let outcome = std::env::var("BOURSO_FAKE_OUTCOME").unwrap();
    let session = Session::open().unwrap();
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
    report.print();
    std::process::exit(report.exit_code().into());
}
