//! Public dispatcher preflight. Successful HTTP quote requests are covered by
//! API loopback tests; root unit tests inject quote data and reject bank access.
use clap::{Arg, Command};

#[tokio::test]
async fn quote_preflight_ignores_missing_or_corrupt_credentials_without_network() {
    let path = std::env::temp_dir().join(format!(
        "bourso-quote-credentials-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let _cleanup = Cleanup(path.clone());
    for corrupt in [false, true] {
        if corrupt {
            std::fs::write(&path, b"{truncated").unwrap();
        }
        let matches = Command::new("bourso")
            .arg(Arg::new("credentials").long("credentials"))
            .subcommand(Command::new("transfer"))
            .subcommand(
                Command::new("quote")
                    .arg(Arg::new("symbol").long("symbol"))
                    .arg(Arg::new("length").long("length"))
                    .arg(Arg::new("interval").long("interval")),
            )
            .try_get_matches_from([
                "bourso",
                "--credentials",
                path.to_str().unwrap(),
                "quote",
                "--symbol",
                "fixture",
                "--length",
                "invalid",
                "--interval",
                "0",
            ])
            .unwrap();
        // Deliberately caller-built invalid length: parsing must fail before the
        // production quote closure can construct a client or perform HTTP.
        let error = bourso_cli::parse_matches(matches).await.unwrap_err();
        assert!(
            error.downcast_ref::<std::num::ParseIntError>().is_some(),
            "wrong preflight error: {error}"
        );
        if corrupt {
            assert_eq!(std::fs::read(&path).unwrap(), b"{truncated");
        } else {
            assert!(!path.exists());
        }
    }
}

#[test]
fn binary_quote_parser_uses_isolated_storage_without_loading_credentials() {
    let home = std::env::temp_dir().join(format!(
        "bourso-quote-cli-{}-{}",
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
    let directory = home.join(".bourso");
    std::fs::create_dir(&directory).unwrap();
    let settings = directory.join("settings.json");
    std::fs::write(&settings, b"{truncated").unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_bourso-cli"))
        .env("HOME", &home)
        .env("BOURSO_DATA_DIR", &directory)
        .env("RUST_LOG", "trace")
        .args(["quote", "--symbol", "fixture", "--length", "invalid"])
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let diagnostic = String::from_utf8_lossy(&output.stderr);
    assert!(diagnostic.contains("--length"));
    assert!(!diagnostic.contains("Failed to deserialize settings"));
    assert!(!diagnostic.contains("Authenticating"));
    assert!(!diagnostic.contains("panicked"));
    assert_eq!(std::fs::read(settings).unwrap(), b"{truncated");
    assert!(directory.join("bourso.log").is_file());
}
