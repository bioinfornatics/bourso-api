//! DOCS-1: real-binary README syntax, isolated local config, no live quotes.
//! Successful quote dispatch is separately covered by lib.rs's private fake seam.
use std::{
    fs,
    path::PathBuf,
    process::{Command, Output, Stdio},
};

const README: &str = include_str!("../README.md");
const CONFIG: &str = "./target/release/bourso-cli config --username 123456789";
const QUOTES: [&str; 2] = [
    "./target/release/bourso-cli quote --symbol 1rTCW8 --length 30 --interval 0 average",
    "./target/release/bourso-cli quote --symbol 1rTCW8 --length 1 last",
];

struct Sandbox(PathBuf);
impl Sandbox {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "bourso-docs-{}-{}",
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
            .env_clear()
            .env("HOME", &self.0)
            .env("USERPROFILE", &self.0)
            .env("XDG_CONFIG_HOME", &self.0)
            .env("BOURSO_DATA_DIR", self.0.join(".bourso"))
            .env("RUST_LOG", "off")
            .current_dir(&self.0)
            .args(args)
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }

    fn settings(&self) -> PathBuf {
        self.0.join(".bourso/settings.json")
    }
}
impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn documented_args(command: &str) -> Vec<&str> {
    assert!(
        README.lines().any(|line| line == command),
        "README example changed: {command}"
    );
    command.split_whitespace().skip(1).collect()
}

#[test]
fn readme_config_really_saves_synthetic_id_and_null_password() {
    let home = Sandbox::new();
    let args = documented_args(CONFIG);
    let output = home.run(&args);
    assert!(output.status.success(), "{:?}", output);
    let saved: serde_json::Value =
        serde_json::from_slice(&fs::read(home.settings()).unwrap()).unwrap();
    assert_eq!(
        saved,
        serde_json::json!({"clientId": "123456789", "password": null})
    );
    assert!(home.0.join(".bourso/bourso.log").is_file());
    // Missing required username must not replace a valid configuration.
    let original = fs::read(home.settings()).unwrap();
    let missing = home.run(&["config"]);
    assert_eq!(missing.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&missing.stderr).contains("--username"));
    assert_eq!(fs::read(home.settings()).unwrap(), original);
}

#[test]
fn exact_readme_quote_options_reach_real_binary_operation_help() {
    let home = Sandbox::new();
    for command in QUOTES {
        let mut args = documented_args(command);
        let operation = *args.last().unwrap();
        args.push("--help");
        let output = home.run(&args);
        assert!(output.status.success(), "{:?}", output);
        let help = String::from_utf8_lossy(&output.stdout);
        assert!(
            help.contains("Usage:") && help.contains(&format!(" {operation}\n")),
            "{help}"
        );
    }
    assert!(!home.settings().exists());
    // Help may precede required-argument/value validation. The next test uses
    // NO help flag and deliberately omits symbol, keeping execution preflight-only.
}

#[test]
fn documented_quote_values_validate_before_missing_symbol_without_help() {
    let home = Sandbox::new();
    for length in ["1", "5", "30", "90", "180", "365", "1825", "3650"] {
        for operation in ["highest", "lowest", "average", "volume", "last"] {
            let output = home.run(&["quote", "--length", length, "--interval", "0", operation]);
            let error = String::from_utf8_lossy(&output.stderr);
            assert_eq!(output.status.code(), Some(2), "{error}");
            assert!(
                error.contains("required arguments were not provided"),
                "{error}"
            );
            assert!(error.contains("--symbol"), "{error}");
            assert!(!error.contains("invalid value"), "{error}");
            assert!(!error.contains("unrecognized"), "{error}");
        }
    }
    // Negative controls ensure this is value validation, not just help precedence.
    for (option, value) in [("--length", "1M"), ("--interval", "1")] {
        let output = home.run(&["quote", option, value, "average"]);
        let error = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(2));
        assert!(error.contains("invalid value"), "{error}");
        assert!(error.contains(option), "{error}");
    }
    assert!(!home.settings().exists());
}

#[test]
fn quote_help_lists_readme_contract_without_reading_credentials() {
    let home = Sandbox::new();
    let output = home.run(&["--credentials", "missing.json", "quote", "--help"]);
    assert!(output.status.success(), "{:?}", output);
    let help = String::from_utf8_lossy(&output.stdout);
    for token in [
        "--symbol",
        "--length",
        "--interval",
        "highest",
        "lowest",
        "average",
        "volume",
        "last",
    ] {
        assert!(help.contains(token), "missing {token}: {help}");
    }
    assert!(!home.settings().exists());
    assert!(!home.0.join("missing.json").exists());
}

#[test]
fn published_guides_do_not_restore_known_unsafe_permission_or_scheduler_examples() {
    // Static regression guard, not an exhaustive security review or OS execution.
    for (path, text) in [
        ("README.md", README),
        (
            "src/bourso_api/README.md",
            include_str!("../src/bourso_api/README.md"),
        ),
        ("docs/DCA.md", include_str!("../docs/DCA.md")),
        (
            "docs/DEVELOPMENT.md",
            include_str!("../docs/DEVELOPMENT.md"),
        ),
    ] {
        let mut fenced = false;
        for line in text.lines() {
            if line.trim_start().starts_with("```") {
                fenced = !fenced;
                continue;
            }
            if fenced {
                let lower = line.to_ascii_lowercase();
                assert!(
                    !lower.contains("-noexit"),
                    "persistent shell example in {path}"
                );
                assert!(
                    !((lower.contains("chmod") || lower.contains("chown"))
                        && (lower.contains("777") || lower.contains("a+rwx"))),
                    "unsafe permission example in {path}"
                );
            }
        }
    }
}

#[test]
fn transfer_help_names_local_byte_policy_without_claiming_provider_contract() {
    let home = Sandbox::new();
    let output = home.run(&["transfer", "--help"]);
    assert!(output.status.success());
    let help = String::from_utf8_lossy(&output.stdout);
    assert!(help.contains("50 UTF-8 bytes"), "{help}");
    assert!(help.contains("provider contract unverified"), "{help}");
    assert!(!help.contains("max 50 characters"));
}
