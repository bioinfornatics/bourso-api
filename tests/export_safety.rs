use bourso_api::client::transaction::{transactions_to_csv, write_export_atomically};

#[test]
fn explicit_export_replaces_only_after_success_and_preserves_private_mode() {
    let dir = std::env::temp_dir().join(format!("bourso-cli-export-safety-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    let path = dir.join("output.csv");
    std::fs::write(&path, b"old data").unwrap();
    let csv = transactions_to_csv(&[]).unwrap();
    write_export_atomically(&path, csv.as_bytes()).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), csv);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    // Failed rename cannot present a partial export or leave a temporary file.
    let directory_destination = dir.join("directory");
    std::fs::create_dir(&directory_destination).unwrap();
    assert!(write_export_atomically(&directory_destination, b"not published").is_err());
    assert!(directory_destination.is_dir());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), csv);
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
    assert!(write_export_atomically(&dir.join("missing/output.csv"), b"not published").is_err());
    std::fs::remove_dir_all(dir).unwrap();
}
