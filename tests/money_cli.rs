use bourso_cli::money::{self, Money};
use clap::{Arg, Command};

fn command() -> Command {
    money::command(Arg::new("account").long("account").required(true))
}

#[test]
fn invalid_amount_is_rejected_by_public_command_parser() {
    for input in [
        "abc",
        "NaN",
        "inf",
        "-10",
        "0",
        "9.99",
        "10.001",
        "10,000",
        "184467440737095516.16",
        "1e2",
    ] {
        let parsed = command().try_get_matches_from([
            "transfer",
            "--account",
            "source",
            "--to",
            "destination",
            &format!("--amount={input}"),
        ]);
        assert!(parsed.is_err(), "invalid money accepted by Clap: {input}");
        assert!(input.parse::<Money>().is_err());
    }
}

#[test]
fn cli_and_api_share_decimal_policy_without_float_conversion() {
    for (input, cents) in [
        ("10", 1000),
        ("10,50", 1050),
        ("10.01", 1001),
        ("184467440737095516.15", u64::MAX),
    ] {
        let matches = command()
            .try_get_matches_from([
                "transfer",
                "--account",
                "source",
                "--to",
                "destination",
                "--amount",
                input,
            ])
            .unwrap();
        let amount = matches
            .get_one::<String>("amount")
            .unwrap()
            .parse::<Money>()
            .unwrap();
        // This assignment also checks that the root reexports the API type rather
        // than maintaining a second monetary implementation.
        let api_amount: bourso_api::client::transfer::money::Money = amount;
        assert_eq!(api_amount.cents(), cents);
    }
}

#[tokio::test]
async fn caller_built_matches_are_validated_before_credentials_are_loaded() {
    // Deliberately bypass the production Clap validator, exercising parse_matches'
    // own boundary. An unusable credentials path must never mask the amount error.
    for input in [
        "abc",
        "NaN",
        "inf",
        "0",
        "9.99",
        "10.001",
        "184467440737095516.16",
    ] {
        let matches = Command::new("bourso")
            .arg(Arg::new("credentials").long("credentials"))
            .subcommand(Command::new("transfer").arg(Arg::new("amount").long("amount")))
            .try_get_matches_from([
                "bourso",
                "--credentials",
                "/dev/null/not-a-file",
                "transfer",
                "--amount",
                input,
            ])
            .unwrap();
        let error = bourso_cli::parse_matches(matches).await.unwrap_err();
        assert!(
            error.downcast_ref::<money::MoneyError>().is_some(),
            "wrong early error: {error}"
        );
    }
}

#[test]
fn binary_rejects_invalid_money_before_authentication() {
    let home = std::env::temp_dir().join(format!(
        "bourso-money-cli-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&home).unwrap();
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(home.clone());
    for input in [
        "abc",
        "NaN",
        "inf",
        "9.99",
        "10.001",
        "184467440737095516.16",
    ] {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_bourso-cli"))
            .env("BOURSO_DATA_DIR", home.join(".bourso"))
            .env("HOME", &home)
            .env("XDG_CONFIG_HOME", &home)
            .env("RUST_LOG", "trace")
            .args([
                "--credentials",
                "/dev/null/not-a-file",
                "transfer",
                "--account",
                "0123456789abcdef0123456789abcdef",
                "--to",
                "destination",
                "--amount",
                input,
            ])
            .stdin(std::process::Stdio::null())
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        assert!(diagnostic.contains("--amount"));
        assert!(!diagnostic.contains("panicked"));
        assert!(!diagnostic.contains("Authenticating"));
        assert!(!diagnostic.contains("password"));
    }
}
