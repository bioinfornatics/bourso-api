#![cfg(not(unix))]

use bourso_api::replay_guard::{GuardError, ReplayGuard};
use std::{error::Error, path::Path};

#[test]
fn unsupported_platform_refuses_paths_without_creating_storage() {
    let absent = std::env::temp_dir().join(format!(
        "bourso-api-unsupported-SENTINEL_PRIVATE-{:032x}",
        rand::random::<u128>()
    ));
    assert!(absent.is_absolute());
    assert!(!absent.exists());
    for path in [Path::new(""), Path::new("SENTINEL_PRIVATE"), &absent] {
        let error = ReplayGuard::open(path).unwrap_err();
        assert_eq!(error, GuardError::UnsupportedPlatform);
        assert_eq!(
            error.to_string(),
            "Replay guard storage is supported only on Unix"
        );
        assert_eq!(format!("{error:?}"), "UnsupportedPlatform");
        assert!(error.source().is_none());
    }
    assert!(!absent.exists(), "refusal must not create storage");
}

#[test]
fn unsupported_platform_refuses_before_inspecting_caller_path() {
    struct UninspectedPath;
    impl AsRef<Path> for UninspectedPath {
        fn as_ref(&self) -> &Path {
            panic!("unsupported platform must refuse before inspecting the path");
        }
    }
    assert_eq!(
        ReplayGuard::open(UninspectedPath).unwrap_err(),
        GuardError::UnsupportedPlatform
    );
}
