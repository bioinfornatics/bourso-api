//! CLI configuration regressions: isolated HOME, no real credentials or bank calls.
use std::{
    fs,
    path::PathBuf,
    process::{Command, Output, Stdio},
};

struct Home(PathBuf);
impl Home {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "bourso-config-cli-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_bourso-cli"))
            .env("BOURSO_DATA_DIR", self.0.join(".bourso"))
            .env("HOME", &self.0)
            .env("XDG_CONFIG_HOME", &self.0)
            .env("RUST_LOG", "trace")
            .args(args)
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }
    fn settings(&self) -> PathBuf {
        self.0.join(".bourso/settings.json")
    }
}
impl Drop for Home {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn bank_commands_without_customer_id_fail_before_authentication() {
    let home = Home::new();
    let account = "0123456789abcdef0123456789abcdef";
    for args in [
        vec!["accounts"],
        vec![
            "trade",
            "order",
            "new",
            "--side",
            "buy",
            "--account",
            account,
            "--symbol",
            "fixture",
            "--quantity",
            "1",
        ],
        vec![
            "transfer",
            "--account",
            account,
            "--to",
            "destination",
            "--amount",
            "10",
        ],
        vec![
            "export",
            "transactions",
            "--account",
            account,
            "--start-date",
            "01/01/2026",
            "--end-date",
            "02/01/2026",
        ],
    ] {
        let output = home.run(&args);
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        assert!(
            !output.status.success(),
            "{args:?} incorrectly succeeded: {diagnostic}"
        );
        assert!(
            diagnostic.contains("config --username"),
            "{args:?}: {diagnostic}"
        );
        assert!(!diagnostic.contains("panicked"));
        assert!(!diagnostic.contains("Authenticating"));
        assert!(!diagnostic.contains("Enter your password"));
        assert!(!diagnostic.contains("successful"));
    }
}

#[test]
fn explicit_config_repairs_corrupt_json_and_ignores_credentials_override() {
    let home = Home::new();
    fs::create_dir(home.0.join(".bourso")).unwrap();
    for args in [
        vec!["config", "--username", "fixture-customer"],
        vec![
            "--credentials",
            "/missing/credentials.json",
            "config",
            "--username",
            "fixture-customer",
        ],
    ] {
        fs::write(home.settings(), b"{truncated").unwrap();
        let output = home.run(&args);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let saved: serde_json::Value =
            serde_json::from_slice(&fs::read(home.settings()).unwrap()).unwrap();
        assert_eq!(saved["clientId"], "fixture-customer");
        assert!(saved["password"].is_null());
    }
}

#[test]
fn bank_command_preserves_corrupt_config_and_reports_error() {
    let home = Home::new();
    fs::create_dir(home.0.join(".bourso")).unwrap();
    fs::write(home.settings(), b"{truncated").unwrap();
    let output = home.run(&["accounts"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Failed to deserialize settings"));
    assert_eq!(fs::read(home.settings()).unwrap(), b"{truncated");
}

#[test]
fn explicit_storage_override_takes_precedence_over_home() {
    let home = Home::new();
    let directory = home.0.join("managed-storage");
    let output = Command::new(env!("CARGO_BIN_EXE_bourso-cli"))
        .env("HOME", &home.0)
        .env("BOURSO_DATA_DIR", &directory)
        .env("RUST_LOG", "off")
        .args(["config", "--username", "override-customer"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let saved: serde_json::Value =
        serde_json::from_slice(&fs::read(directory.join("settings.json")).unwrap()).unwrap();
    assert_eq!(saved["clientId"], "override-customer");
    assert!(saved["password"].is_null());
    assert!(directory.join("bourso.log").is_file());
    assert!(!home.0.join(".bourso").exists());
}

#[test]
fn invalid_storage_override_fails_without_falling_back_to_home() {
    let home = Home::new();
    for invalid in ["", "relative", "~/private"] {
        let output = Command::new(env!("CARGO_BIN_EXE_bourso-cli"))
            .env("HOME", &home.0)
            .env("BOURSO_DATA_DIR", invalid)
            .args(["config", "--username", "must-not-save"])
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr)
            .contains("BOURSO_DATA_DIR must be a nonempty absolute directory path"));
        assert!(!home.0.join(".bourso").exists());
    }
}
