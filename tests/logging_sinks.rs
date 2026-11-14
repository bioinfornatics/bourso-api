//! Real logger sinks tested in separate processes: global subscriber and HOME stay isolated.
use std::{
    fs,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

const SECRET: &str = "987654321098765432";

#[test]
fn logger_child_probe() {
    if std::env::var_os("BOURSO_LOGGING_CHILD").is_none() {
        return;
    }
    bourso_cli::settings::init_logger().unwrap();
    let credentials = std::env::var("BOURSO_TEST_CREDENTIALS").unwrap();
    let settings_error = bourso_cli::settings::Settings::load(&credentials).unwrap_err();
    let html = format!("window.BRS_CONFIG = {{\"DEBUG\":\"{SECRET}\"}};");
    let config_error = bourso_api::client::config::extract_brs_config(&html).unwrap_err();
    let password_error =
        bourso_api::client::virtual_pad::password_to_virtual_pad_keys(vec![String::new(); 10], "§")
            .unwrap_err();
    let qr_error = anyhow::anyhow!(bourso_api::client::error::ClientError::QRCodeRequired(
        SECRET.to_owned()
    ));
    for error in [settings_error, config_error, password_error, qr_error] {
        tracing::error!(target: "bourso_cli::logging_probe", "{error:#?}");
        tracing::debug!(target: "bourso_cli::logging_probe", diagnostic = ?error, "Diagnostic probe");
        eprintln!("{error:#}");
    }
    // The real logger installs the log->tracing bridge. Exercise actual dependency
    // messages, not merely simulated tracing targets.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        use std::io::{Read, Write};
        listener.set_nonblocking(true).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(std::time::Instant::now() < deadline);
                    std::thread::sleep(std::time::Duration::from_millis(2));
                }
                Err(error) => panic!("{error}"),
            }
        };
        // Accepted sockets inherit listener nonblocking mode on Windows.
        stream.set_nonblocking(false).unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(2)))
            .unwrap();
        let mut bytes = [0; 4096];
        let read = stream.read(&mut bytes).unwrap();
        assert!(read > 0, "expected an HTTP request");
        write!(stream, "HTTP/1.1 302 Found\r\nLocation: http://{address}/challenge?token={SECRET}\r\nSet-Cookie: session={SECRET}; Path=/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
    });
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let client = reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .cookie_store(true)
                .timeout(std::time::Duration::from_secs(2))
                .build()
                .unwrap();
            let response = client
                .get(format!("http://{address}/"))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), 302);
        });
    server.join().unwrap();
    tracing::info!(target: "bourso_cli::logging_probe", "application-diagnostic-probe");
    tracing::trace!(target: "bourso_cli::logging_probe", "trace-sink-probe");
}

#[test]
fn real_console_and_file_sinks_do_not_disclose_sentinels() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    for filter in ["trace", "off"] {
        let home = std::env::temp_dir().join(format!(
            "bourso-log-sinks-{}-{unique}-{filter}",
            std::process::id()
        ));
        fs::create_dir_all(&home).unwrap();
        let credentials = home.join("credentials.json");
        fs::write(
            &credentials,
            format!("{{\"clientId\":null,\"password\":{SECRET}}}"),
        )
        .unwrap();
        let output = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "logger_child_probe", "--nocapture"])
            .env("BOURSO_LOGGING_CHILD", "1")
            .env("BOURSO_TEST_CREDENTIALS", &credentials)
            .env("BOURSO_DATA_DIR", home.join(".bourso"))
            .env("HOME", &home)
            .env("RUST_LOG", filter)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let file = fs::read_to_string(home.join(".bourso/bourso.log")).unwrap();
        assert!(
            file.contains("trace-sink-probe"),
            "TRACE file sink was not exercised"
        );
        assert!(
            file.contains("Diagnostic probe"),
            "application DEBUG diagnostics must remain visible"
        );
        assert!(!file.contains("reqwest::") && !file.contains("cookie_store::"));
        for sink in [
            file,
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ] {
            assert!(!sink.contains(SECRET), "sensitive sentinel leaked: {sink}");
            assert!(!sink.contains('§'), "password character leaked: {sink}");
        }
        fs::remove_dir_all(home).unwrap();
    }
}
