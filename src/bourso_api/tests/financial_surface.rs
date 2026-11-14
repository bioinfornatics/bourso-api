//! Compiler probes run as an external crate; no bank or network operations.
use std::{fs, path::PathBuf, process::Command};
struct Temp(PathBuf);
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
#[test]
fn public_surface_has_no_raw_financial_bypass() {
    // Ask Cargo for this source/toolchain's artifact, never a cached sibling by mtime.
    // This runs after Cargo has released its compilation lock to execute tests.
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let compiler = Command::new(&rustc).arg("--version").output().unwrap();
    assert!(compiler.status.success());
    let build = Command::new(env!("CARGO"))
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .env("RUSTC", &rustc)
        .args([
            "build",
            "-p",
            "bourso_api",
            "--lib",
            "--locked",
            "--offline",
            "--message-format=json",
        ])
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "Cargo artifact build failed: {}\n{}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr)
    );
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let mut libraries = Vec::new();
    for line in String::from_utf8(build.stdout).unwrap().lines() {
        let message: serde_json::Value = serde_json::from_str(line).unwrap();
        if message["reason"] == "compiler-artifact"
            && message["manifest_path"].as_str() == manifest.to_str()
            && message["target"]["name"] == "bourso_api"
            && message["target"]["kind"] == serde_json::json!(["lib"])
        {
            for filename in message["filenames"].as_array().unwrap() {
                let path = PathBuf::from(filename.as_str().unwrap());
                if path.extension().is_some_and(|ext| ext == "rlib") {
                    libraries.push(path);
                }
            }
        }
    }
    assert_eq!(libraries.len(), 1, "expected one Cargo-selected API rlib");
    let lib = &libraries[0];
    let deps = lib.parent().unwrap().join("deps");
    eprintln!(
        "surface compiler: {}; Cargo-selected API rlib: {}",
        String::from_utf8_lossy(&compiler.stdout).trim(),
        lib.display()
    );
    let temp = Temp(std::env::temp_dir().join(format!(
        "bourso-surface-{}-{:032x}",
        std::process::id(),
        rand::random::<u128>()
    )));
    fs::create_dir(&temp.0).unwrap();
    let check = |body: &str| {
        let src = temp.0.join("probe.rs");
        fs::write(&src, format!("#![allow(dead_code, unused_imports)]\nuse bourso_api::{{client::BoursoWebClient, FinancialSession}};\n{body}\nfn main() {{}}" )).unwrap();
        Command::new(&rustc)
            .args([
                "--edition=2021",
                "--crate-name",
                "surface_probe",
                "--emit=metadata",
            ])
            .arg(&src)
            .arg("--extern")
            .arg(format!("bourso_api={}", lib.display()))
            .arg("-L")
            .arg(format!("dependency={}", deps.display()))
            .arg("-o")
            .arg(temp.0.join("probe.rmeta"))
            .output()
            .unwrap()
    };
    let pass = check("async fn pass() { let mut c = bourso_api::get_client(); let _ = BoursoWebClient::new(); let _ = BoursoWebClient::default(); let _ = c.init_session().await; let _ = c.login(\"x\", \"y\").await; let _ = c.get_accounts(None).await; let mut s = FinancialSession::open(\"/trusted/private\").unwrap(); let _ = s.unresolved(); let _ = s.order(&c, bourso_api::client::trade::order::OrderSide::Buy, &Default::default(), \"TEST\", 2, None).await; let _ = s.transfer_money(&c, \"12.00\".parse().unwrap(), Default::default(), Default::default(), None).await; }");
    assert!(
        pass.status.success(),
        "{}",
        String::from_utf8_lossy(&pass.stderr)
    );
    for constructor in [
        "bourso_api::get_client()",
        "BoursoWebClient::new()",
        "BoursoWebClient::default()",
    ] {
        for method in [
            "order",
            "order_detailed",
            "transfer_funds",
            "transfer_money",
            "cancel_order",
            "prepare",
            "check",
            "confirm",
            "init_transfer",
            "set_debit_account",
            "set_credit_account",
            "set_transfer_amount",
            "submit_step_5",
            "set_transfer_reason",
            "extract_flow_instance",
            "transfer_validated",
            "confirm_transfer",
        ] {
            let output = check(&format!(
                "fn probe() {{ let c = {constructor}; let _ = c.{method}(); }}"
            ));
            let err = String::from_utf8_lossy(&output.stderr);
            assert!(
                !output.status.success()
                    && err.contains(method)
                    && (err.contains("error[E0624]") || err.contains("error[E0599]")),
                "{constructor}.{method}: {err}"
            );
            assert!(
                !err.contains("error[E0061]"),
                "wrong argument count is not access proof: {err}"
            );
        }
    }
    for (body, diagnostic) in [
        ("fn probe(c: BoursoWebClient) { let _ = c.client; }", "error[E0616]"),
        ("fn probe(s: FinancialSession) { let _ = s.guard; }", "error[E0616]"),
        ("fn probe(s: FinancialSession) { let _ = *s; }", "error[E0614]"),
        ("fn probe(s: FinancialSession) { let _ = s.into_inner(); }", "error[E0599]"),
        ("async fn probe(s: &mut FinancialSession, c: &BoursoWebClient) { let account = Default::default(); let f = s.order(c, bourso_api::client::trade::order::OrderSide::Buy, &account, \"TEST\", 2, None); let _ = s.review(\"id\", bourso_api::replay_guard::ReviewDecision::NotApplied); f.await; }", "error[E0499]"),
    ] {
        let output = check(body); let err = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success() && err.contains(diagnostic), "{err}");
    }
}
