use super::*;
use std::io::Write;

fn directory() -> std::path::PathBuf {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "bourso-export-test-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir(&dir).unwrap();
    dir
}

#[test]
fn export_parse_and_partial_write_failures_preserve_destination() {
    let dir = directory();
    let path = dir.join("old.csv");
    std::fs::write(&path, b"original").unwrap();
    for body in ["", "<html>expired</html>", "wrong;header", "dateOp;dateVal;label;category;categoryParent;supplierFound;amount;comment;accountNum;accountLabel;accountbalance\n2026;2026;ok;;;;1;;test;test;2\n2026;2026;bad;;;;oops;;test;test;2\n"] {
        let result = extract_transactions(body).and_then(|_| write_export_atomically(&path, body.as_bytes()));
        assert!(result.is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"original");
    }
    assert!(publish_export(&path, |file| {
        file.write_all(b"partial")?;
        anyhow::bail!("injected mid-write failure")
    })
    .is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"original");
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn export_csv_roundtrip_preserves_eleven_fields_and_formula_literals() {
    for label in [
        "semicolon;value",
        "a \"quote\"",
        "CR\rLF\nvalue",
        "é漢字",
        "=SUM(A1:A2)",
        "+1",
        "-1",
        "@name",
    ] {
        let tx = Transaction {
            date_op: "2026-01-01".into(),
            date_val: "2026-01-02".into(),
            label: label.into(),
            category: "cat;one".into(),
            category_parent: "parent".into(),
            supplier_found: "supply".into(),
            amount: -12.34,
            comment: "notes\n\"quoted\"".into(),
            account_num: "00123".into(),
            account_label: "Bourso".into(),
            account_balance: 42.0,
        };
        let output = transactions_to_csv(&[tx]).unwrap();
        let mut reader = csv::ReaderBuilder::new()
            .delimiter(b';')
            .from_reader(output.as_bytes());
        assert_eq!(reader.headers().unwrap().len(), 11);
        let rows: Vec<_> = reader
            .records()
            .collect::<std::result::Result<_, _>>()
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].iter().collect::<Vec<_>>(),
            vec![
                "2026-01-01",
                "2026-01-02",
                label,
                "cat;one",
                "parent",
                "supply",
                "-12.34",
                "notes\n\"quoted\"",
                "00123",
                "Bourso",
                "42.00"
            ]
        );
    }
    assert_eq!(transactions_to_csv(&[]).unwrap().trim_end(), "dateOp;dateVal;label;category;categoryParent;supplierFound;amount;comment;accountNum;accountLabel;accountbalance");
}

#[test]
fn export_redirect_url_resolution_checks_original_origin() {
    for (location, expected) in [
        (
            "/download.csv?x=1",
            "https://clients.boursobank.com/download.csv?x=1",
        ),
        (
            "download.csv",
            "https://clients.boursobank.com/budget/download.csv",
        ),
        (
            "../download.csv",
            "https://clients.boursobank.com/download.csv",
        ),
        (
            "//clients.boursobank.com/download.csv",
            "https://clients.boursobank.com/download.csv",
        ),
        (
            "https://clients.boursobank.com/download.csv",
            "https://clients.boursobank.com/download.csv",
        ),
    ] {
        assert_eq!(
            resolve_export_redirect(location).unwrap().as_str(),
            expected
        );
    }
    for location in [
        "",
        " ",
        "http://clients.boursobank.com/a",
        "//evil.invalid/a",
        "https://clients.boursobank.com.evil.invalid/a",
        "https://user:secret@clients.boursobank.com/a",
        "https://clients.boursobank.com:444/a",
        "https://[invalid",
        "javascript:alert(1)",
    ] {
        let error = resolve_export_redirect(location).unwrap_err().to_string();
        assert!(!error.contains("secret"));
    }
}
