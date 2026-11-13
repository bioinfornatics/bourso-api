//! Public API probes run in children; neither HOME nor umask changes in the test runner.
use bourso_cli::settings::{get_settings, init_logger, save_settings, Settings};
use std::{
    fs,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

struct Home(PathBuf);
impl Home {
    fn new() -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("bourso-private-{}-{unique}", std::process::id()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Home {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn run_probe(home: &Home, case: &str, mut command: Command) {
    let output = command
        .env("HOME", &home.0)
        .env("BOURSO_DATA_DIR", home.0.join(".bourso"))
        .env("BOURSO_PRIVATE_CASE", case)
        .env("RUST_LOG", "off")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "child failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
fn probe(home: &Home, case: &str) {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.args(["--exact", "private_files_child", "--nocapture"]);
    run_probe(home, case, command);
}
#[cfg(unix)]
fn probe_umask(home: &Home, case: &str, mask: &str) {
    let mut command = Command::new("sh");
    command
        .args([
            "-c",
            "umask \"$1\"; exec \"$2\" --exact private_files_child --nocapture",
            "probe",
            mask,
        ])
        .arg(std::env::current_exe().unwrap());
    run_probe(home, case, command);
}
#[test]
fn private_files_child() {
    let Ok(case) = std::env::var("BOURSO_PRIVATE_CASE") else {
        return;
    };
    match case.as_str() {
        "create" => {
            save_settings(&Settings {
                customer_id: Some("test-customer".into()),
                password: None,
            })
            .unwrap();
            init_logger().unwrap();
            tracing::info!(target: "bourso_cli::private_probe", "private-log-probe");
        }
        "load" => {
            assert_eq!(
                get_settings().unwrap().customer_id.as_deref(),
                Some("preserved")
            );
        }
        "load-error" => {
            assert!(get_settings().is_err());
        }
        "save-error" => {
            assert!(save_settings(&Settings {
                customer_id: None,
                password: None
            })
            .is_err());
        }
        "logger-error" => {
            assert!(init_logger().is_err());
        }
        "rotate" => {
            init_logger().unwrap();
            let payload = "x".repeat(1024 * 1024);
            for index in 0..25 {
                tracing::info!(target: "bourso_cli::private_probe", index, "{payload}");
            }
            tracing::info!(target: "bourso_cli::private_probe", "last-record");
            assert!(init_logger().is_err());
        }
        "log" => {
            init_logger().unwrap();
            tracing::info!(target: "bourso_cli::private_probe", "appended");
        }
        "subscriber-error" => {
            use tracing_subscriber::util::SubscriberInitExt;
            tracing_subscriber::registry().try_init().unwrap();
            assert!(init_logger().is_err());
        }
        #[cfg(unix)]
        "runtime-error" => {
            init_logger().unwrap();
            let log =
                PathBuf::from(std::env::var_os("BOURSO_DATA_DIR").unwrap()).join("bourso.log");
            fs::remove_file(&log).unwrap();
            fs::create_dir(log).unwrap();
            tracing::info!(target: "bourso_cli::private_probe", "must-not-panic");
        }
        _ => panic!("unknown probe"),
    }
}
#[cfg(unix)]
#[test]
fn newly_created_settings_and_logs_are_private_under_permissive_umask() {
    let home = Home::new();
    probe_umask(&home, "create", "000");
    let dir = home.0.join(".bourso");
    for name in ["settings.json", "bourso.log"] {
        assert_eq!(
            fs::metadata(dir.join(name)).unwrap().permissions().mode() & 0o777,
            0o600,
            "{name} must be private independent of umask"
        );
    }
    assert_eq!(
        fs::metadata(dir).unwrap().permissions().mode() & 0o777,
        0o700
    );
}

fn fixture(home: &Home) -> PathBuf {
    let dir = home.0.join(".bourso");
    fs::create_dir(&dir).unwrap();
    dir
}
#[cfg(unix)]
fn mode(path: impl AsRef<std::path::Path>) -> u32 {
    fs::metadata(path).unwrap().permissions().mode() & 0o777
}
#[cfg(unix)]
#[test]
fn exact_private_modes_also_work_under_restrictive_umasks() {
    for mask in ["022", "077", "777"] {
        let home = Home::new();
        probe_umask(&home, "create", mask);
        let dir = home.0.join(".bourso");
        assert_eq!(mode(&dir), 0o700);
        for name in ["settings.json", "bourso.log", "bourso.log.lock"] {
            assert_eq!(mode(dir.join(name)), 0o600);
        }
    }
}
#[cfg(unix)]
#[test]
fn existing_settings_and_logs_preserve_content_and_tighten_modes() {
    let home = Home::new();
    let dir = fixture(&home);
    let json = br#"{"clientId":"preserved","password":null}"#;
    fs::write(dir.join("settings.json"), json).unwrap();
    fs::write(dir.join("bourso.log"), "old-log\n").unwrap();
    fs::write(dir.join("bourso.log.1"), "old-archive\n").unwrap();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o777)).unwrap();
    for name in ["settings.json", "bourso.log", "bourso.log.1"] {
        fs::set_permissions(dir.join(name), fs::Permissions::from_mode(0o666)).unwrap();
    }
    probe(&home, "load");
    probe(&home, "log");
    assert_eq!(fs::read(dir.join("settings.json")).unwrap(), json);
    assert!(fs::read_to_string(dir.join("bourso.log"))
        .unwrap()
        .starts_with("old-log\n"));
    assert_eq!(
        fs::read_to_string(dir.join("bourso.log.1")).unwrap(),
        "old-archive\n"
    );
    assert_eq!(mode(&dir), 0o700);
    for name in ["settings.json", "bourso.log", "bourso.log.1"] {
        assert_eq!(mode(dir.join(name)), 0o600);
    }
}
#[cfg(unix)]
#[test]
fn malformed_or_unreadable_settings_are_never_replaced() {
    let home = Home::new();
    let dir = fixture(&home);
    let path = dir.join("settings.json");
    let original = b"{ invalid json";
    fs::write(&path, original).unwrap();
    probe(&home, "load-error");
    assert_eq!(fs::read(&path).unwrap(), original);
    // Use valid JSON for the permission case: an implementation that swallowed
    // PermissionDenied and wrote defaults must fail the content assertion.
    let valid = br#"{"clientId":"preserved","password":null}"#;
    fs::write(&path, valid).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o0)).unwrap();
    // Permission denial only has meaning for an unprivileged test process.
    if fs::File::open(&path).is_err() {
        probe(&home, "load-error");
        probe(&home, "save-error");
    }
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(fs::read(&path).unwrap(), valid);
}
#[cfg(unix)]
#[test]
fn symlinks_and_hardlinks_never_overwrite_targets() {
    for name in [
        "settings.json",
        "bourso.log",
        "bourso.log.1",
        "bourso.log.lock",
    ] {
        for hardlink in [false, true] {
            let home = Home::new();
            let dir = fixture(&home);
            let target = home.0.join("untouched");
            fs::write(&target, "must survive").unwrap();
            fs::set_permissions(&target, fs::Permissions::from_mode(0o644)).unwrap();
            if hardlink {
                fs::hard_link(&target, dir.join(name)).unwrap();
            } else {
                std::os::unix::fs::symlink(&target, dir.join(name)).unwrap();
            }
            if name == "settings.json" {
                probe(&home, "load-error");
                probe(&home, "save-error");
                assert!(Settings::load(dir.join(name).to_str().unwrap()).is_err());
            } else {
                probe(&home, "logger-error");
            }
            assert_eq!(fs::read_to_string(&target).unwrap(), "must survive");
            assert_eq!(mode(&target), 0o644);
        }
    }
}
#[cfg(unix)]
#[test]
fn linked_directory_is_refused_without_changing_target() {
    let home = Home::new();
    let target = home.0.join("untouched");
    fs::create_dir(&target).unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o755)).unwrap();
    std::os::unix::fs::symlink(&target, home.0.join(".bourso")).unwrap();
    for case in ["load-error", "save-error", "logger-error"] {
        probe(&home, case);
    }
    assert_eq!(mode(&target), 0o755);
    assert_eq!(fs::read_dir(target).unwrap().count(), 0);
}
#[test]
fn nonregular_files_return_errors_not_panics() {
    for name in [
        "settings.json",
        "bourso.log",
        "bourso.log.1",
        "bourso.log.lock",
    ] {
        let home = Home::new();
        let dir = fixture(&home);
        fs::create_dir(dir.join(name)).unwrap();
        if name == "settings.json" {
            probe(&home, "load-error");
            probe(&home, "save-error");
        } else {
            probe(&home, "logger-error");
        }
        assert!(dir.join(name).is_dir());
    }
}
#[cfg(unix)]
#[test]
fn runtime_file_failure_does_not_panic() {
    probe(&Home::new(), "runtime-error");
}
#[test]
fn rotation_is_bounded_during_writes_and_preserves_userdata() {
    let home = Home::new();
    let dir = fixture(&home);
    for name in ["settings.json", "userdata", "bourso.log.2"] {
        fs::write(dir.join(name), "preserved").unwrap();
    }
    probe(&home, "rotate");
    for name in ["bourso.log", "bourso.log.1"] {
        let path = dir.join(name);
        assert!(fs::metadata(&path).unwrap().len() <= 10 * 1024 * 1024);
        #[cfg(unix)]
        assert_eq!(mode(&path), 0o600);
        // Whole JSON events, not arbitrary byte truncation, must survive rotation.
        for line in fs::read_to_string(path).unwrap().lines() {
            serde_json::from_str::<serde_json::Value>(line).unwrap();
        }
    }
    assert!(fs::read_to_string(dir.join("bourso.log"))
        .unwrap()
        .contains("last-record"));
    for name in ["settings.json", "userdata", "bourso.log.2"] {
        assert_eq!(fs::read_to_string(dir.join(name)).unwrap(), "preserved");
    }
    assert_eq!(fs::read_dir(dir).unwrap().count(), 6);
}
#[test]
fn existing_oversized_logs_require_manual_action_without_truncation() {
    for name in ["bourso.log", "bourso.log.1"] {
        let home = Home::new();
        let dir = fixture(&home);
        let path = dir.join(name);
        let file = fs::File::create(&path).unwrap();
        file.set_len(10 * 1024 * 1024 + 1).unwrap();
        probe(&home, "logger-error");
        assert_eq!(fs::metadata(path).unwrap().len(), 10 * 1024 * 1024 + 1);
    }
}

#[test]
fn existing_subscriber_returns_error_not_panic() {
    probe(&Home::new(), "subscriber-error");
}

#[cfg(unix)]
#[test]
fn explicit_credentials_are_read_without_chmod_and_special_files_are_rejected() {
    let home = Home::new();
    let path = home.0.join("credentials.json");
    fs::write(&path, br#"{"clientId":"external","password":null}"#).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o400)).unwrap();
    assert_eq!(
        Settings::load(path.to_str().unwrap())
            .unwrap()
            .customer_id
            .as_deref(),
        Some("external")
    );
    assert_eq!(mode(&path), 0o400);
    let dir = fixture(&home);
    let socket = dir.join("settings.json");
    let _listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
    probe(&home, "load-error");
    probe(&home, "save-error");
    assert!(Settings::load(socket.to_str().unwrap()).is_err());
}

#[cfg(unix)]
#[test]
fn dangling_symlinks_are_not_treated_as_absent_settings() {
    let home = Home::new();
    let dir = fixture(&home);
    let target = home.0.join("must-not-be-created");
    std::os::unix::fs::symlink(&target, dir.join("settings.json")).unwrap();
    probe(&home, "load-error");
    probe(&home, "save-error");
    assert!(!target.exists());
    assert!(fs::symlink_metadata(dir.join("settings.json"))
        .unwrap()
        .file_type()
        .is_symlink());
}
#[cfg(unix)]
#[test]
fn denied_log_permissions_return_error_without_replacing_content() {
    let home = Home::new();
    let dir = fixture(&home);
    let path = dir.join("bourso.log");
    fs::write(&path, "preserved log").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o0)).unwrap();
    if fs::File::open(&path).is_err() {
        probe(&home, "logger-error");
    }
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(fs::read_to_string(path).unwrap(), "preserved log");
}

#[test]
fn storage_without_home_child() {
    if std::env::var_os("BOURSO_NO_HOME_CHILD").is_none() {
        return;
    }
    assert!(std::env::var_os("HOME").is_none());
    let initial = get_settings().unwrap();
    assert!(initial.customer_id.is_none() && initial.password.is_none());
    save_settings(&Settings {
        customer_id: Some("isolated-customer".into()),
        password: None,
    })
    .unwrap();
    assert_eq!(
        get_settings().unwrap().customer_id.as_deref(),
        Some("isolated-customer")
    );
    init_logger().unwrap();
    tracing::info!(target: "bourso_cli::private_probe", "no-home-storage-probe");
}

#[test]
fn explicit_override_supports_settings_and_logger_without_home() {
    let home = Home::new();
    let directory = home.0.join(".bourso");
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "storage_without_home_child", "--nocapture"])
        .env_remove("HOME")
        .env_remove("XDG_CONFIG_HOME")
        .env("BOURSO_DATA_DIR", &directory)
        .env("BOURSO_NO_HOME_CHILD", "1")
        .env("RUST_LOG", "off")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let saved: serde_json::Value =
        serde_json::from_slice(&fs::read(directory.join("settings.json")).unwrap()).unwrap();
    assert_eq!(saved["clientId"], "isolated-customer");
    assert!(saved["password"].is_null());
    assert!(fs::read_to_string(directory.join("bourso.log"))
        .unwrap()
        .contains("no-home-storage-probe"));
    assert!(directory.join("bourso.log.lock").is_file());
}

#[test]
fn regular_file_cannot_be_used_as_managed_directory() {
    let home = Home::new();
    let path = home.0.join(".bourso");
    fs::write(&path, b"untouched").unwrap();
    for case in ["load-error", "save-error", "logger-error"] {
        probe(&home, case);
    }
    assert_eq!(fs::read(path).unwrap(), b"untouched");
}
