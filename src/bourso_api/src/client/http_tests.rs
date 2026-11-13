use super::BoursoWebClient;
use std::net::SocketAddr;

fn fixture_client(address: SocketAddr) -> BoursoWebClient {
    BoursoWebClient::for_test(address, std::time::Duration::from_secs(2)).unwrap()
}

const DESTINATIONS: &[&str] = &[
    "https://clients.boursobank.com/connexion/?next=%2Fdashboard",
    "https://api.boursobank.com/services/api/v1.7/_user_/_fixture_/trading",
    "https://www.boursorama.com/bourse/action/graph/ws/GetTicksEOD?symbol=TEST&guid=",
    "https://redirect.invalid/export.csv?download=1",
    "http://192.0.2.1:9999/absolute-redirect",
];

#[test]
fn all_request_destinations_are_local() {
    let address: SocketAddr = "127.0.0.1:32123".parse().unwrap();
    let client = fixture_client(address);
    for original in DESTINATIONS {
        for builder in [client.client.get(*original), client.client.post(*original)] {
            let request = builder.build().unwrap();
            let original = reqwest::Url::parse(original).unwrap();
            assert_eq!(request.url().scheme(), "http");
            assert_eq!(request.url().host_str(), Some("127.0.0.1"));
            assert_eq!(request.url().port(), Some(32123));
            assert_eq!(request.url().path(), original.path());
            assert_eq!(request.url().query(), original.query());
        }
    }
}

use std::{
    future::Future,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};
use tokio::sync::mpsc;

const BOUND: Duration = Duration::from_secs(3);

enum Reply {
    Http(String),
    Hold,
    Close,
    /// Keep accepting after the script to detect retries, even after cancellation.
    ObserveQuiet(Duration),
}

fn response(status: &str, headers: &str, body: &str) -> Reply {
    Reply::Http(format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n{body}",
        body.len()
    ))
}

fn ok(body: &str) -> Reply {
    response("200 OK", "", body)
}

#[derive(Debug)]
enum Event {
    Request(String),
    Disconnected,
    QuietObserved,
}

/// Small scripted HTTP/1 fixture. Every accept/read/write has a deadline;
/// Drop stops and joins the worker, including when an assertion fails.
struct Server {
    address: SocketAddr,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<Vec<String>>>,
    events: mpsc::UnboundedReceiver<Event>,
}

impl Server {
    fn new(replies: Vec<Reply>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&stop);
        let (sender, events) = mpsc::unbounded_channel();
        let worker = thread::spawn(move || {
            let mut requests = Vec::new();
            for reply in replies {
                if let Reply::ObserveQuiet(window) = reply {
                    let deadline = Instant::now() + window;
                    while !stopped.load(Ordering::Relaxed) && Instant::now() < deadline {
                        match listener.accept() {
                            Ok(_) => panic!("unexpected extra connection after final submission"),
                            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                                thread::sleep(Duration::from_millis(2));
                            }
                            Err(e) => panic!("fixture quiet observation: {e}"),
                        }
                    }
                    let _ = sender.send(Event::QuietObserved);
                    continue;
                }
                let deadline = Instant::now() + BOUND;
                let mut stream = loop {
                    if stopped.load(Ordering::Relaxed) {
                        return requests;
                    }
                    assert!(Instant::now() < deadline, "fixture accept timed out");
                    match listener.accept() {
                        Ok((stream, peer)) => {
                            assert!(peer.ip().is_loopback());
                            break stream;
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(2));
                        }
                        Err(e) => panic!("fixture accept: {e}"),
                    }
                };
                stream
                    .set_read_timeout(Some(Duration::from_millis(50)))
                    .unwrap();
                stream.set_write_timeout(Some(BOUND)).unwrap();
                let request = read_request(&mut stream);
                requests.push(request.clone());
                let _ = sender.send(Event::Request(request));
                match reply {
                    Reply::Http(response) => stream.write_all(response.as_bytes()).unwrap(),
                    Reply::Close => (),
                    Reply::ObserveQuiet(_) => unreachable!(),
                    Reply::Hold => {
                        let deadline = Instant::now() + BOUND;
                        let mut byte = [0];
                        while !stopped.load(Ordering::Relaxed) && Instant::now() < deadline {
                            match stream.read(&mut byte) {
                                Ok(0) => {
                                    let _ = sender.send(Event::Disconnected);
                                    break;
                                }
                                Ok(_) => panic!("unexpected bytes on held request"),
                                Err(e)
                                    if matches!(
                                        e.kind(),
                                        std::io::ErrorKind::WouldBlock
                                            | std::io::ErrorKind::TimedOut
                                    ) => {}
                                Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => {
                                    let _ = sender.send(Event::Disconnected);
                                    break;
                                }
                                Err(e) => panic!("fixture hold: {e}"),
                            }
                        }
                    }
                }
            }
            requests
        });
        Self {
            address,
            stop,
            worker: Some(worker),
            events,
        }
    }

    async fn event(&mut self) -> Event {
        tokio::time::timeout(BOUND, self.events.recv())
            .await
            .unwrap()
            .unwrap()
    }

    fn finish(mut self) -> Vec<String> {
        self.worker.take().unwrap().join().unwrap()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let result = worker.join();
            if !thread::panicking() {
                result.unwrap();
            }
        }
    }
}

fn read_request(stream: &mut TcpStream) -> String {
    let deadline = Instant::now() + BOUND;
    let mut bytes = Vec::new();
    let mut expected_length = None;
    while Instant::now() < deadline && bytes.len() < 128 * 1024 {
        let mut buffer = [0; 4096];
        match stream.read(&mut buffer) {
            Ok(0) => panic!("incomplete fixture request"),
            Ok(n) => bytes.extend_from_slice(&buffer[..n]),
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                continue
            }
            Err(e) => panic!("fixture read: {e}"),
        }
        if expected_length.is_none() {
            if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&bytes[..end]).to_lowercase();
                assert!(
                    !headers.contains("transfer-encoding:"),
                    "fixture requires Content-Length"
                );
                let length = headers
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length:"))
                    .map(|s| s.trim().parse::<usize>().unwrap())
                    .unwrap_or(0);
                expected_length = Some(end + 4 + length);
            }
        }
        if expected_length == Some(bytes.len()) {
            return String::from_utf8(bytes).unwrap();
        }
    }
    panic!("fixture request exceeded time/size bound");
}

fn run(future: impl Future<Output = ()>) {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            tokio::time::timeout(Duration::from_secs(10), future)
                .await
                .unwrap()
        });
}

#[test]
fn production_destinations_are_unchanged_without_sending() {
    let client = BoursoWebClient::new();
    for original in DESTINATIONS {
        for builder in [client.client.get(*original), client.client.post(*original)] {
            assert_eq!(builder.build().unwrap().url().as_str(), *original);
        }
    }
}

#[test]
fn loopback_only_and_fail_closed_routing() {
    for address in ["192.0.2.1:80", "0.0.0.0:1234", "[::]:1234", "127.0.0.1:0"] {
        assert!(BoursoWebClient::for_test(address.parse().unwrap(), BOUND).is_err());
    }
    for address in ["127.0.0.1:1234", "[::1]:4321"] {
        let client = fixture_client(address.parse().unwrap());
        let original = "https://user:password@outside.invalid:9999//outside.invalid/a%2Fb?url=https%3A%2F%2Foutside.invalid#fragment";
        let first = client.client.get(original).build().unwrap();
        assert_eq!(
            first.url().origin().ascii_serialization(),
            format!("http://{address}")
        );
        assert_eq!(first.url().username(), "");
        assert_eq!(first.url().password(), None);
        assert_eq!(first.url().path(), "//outside.invalid/a%2Fb");
        assert_eq!(
            client
                .client
                .get(first.url().clone())
                .build()
                .unwrap()
                .url(),
            first.url()
        );
        assert!(client.client.get("not a URL").build().is_err());
        assert!(client
            .client
            .post("/relative-without-origin")
            .build()
            .is_err());
    }
}

#[test]
fn export_follows_relative_and_bank_redirects_locally_with_cookies() {
    for location in [
        "/fixture/export.csv?download=1",
        "https://clients.boursobank.com/fixture/export.csv?download=1",
        "../fixture/export.csv?download=1",
        "//clients.boursobank.com/fixture/export.csv?download=1",
    ] {
        run(async {
            let server = Server::new(vec![
                response("200 OK", "Set-Cookie: session=fixture; Path=/\r\n", r#"<input name="movementSearch[_token]" value="fixture-csrf">"#),
                response("302 Found", &format!("Location: {location}\r\nSet-Cookie: redirected=yes; Path=/\r\n"), ""),
                ok("dateOp;dateVal;label;category;categoryParent;supplierFound;amount;comment;accountNum;accountLabel;accountbalance\n2026-01-02;2026-01-02;Fixture;;;;12,34;;fake;Test;42,00\n"),
            ]);
            let client = fixture_client(server.address);
            let transactions = client
                .get_transactions("fake", "01/01/2026", "02/01/2026")
                .await
                .unwrap();
            assert_eq!(transactions.len(), 1);
            assert_eq!(transactions[0].label, "Fixture");
            assert_eq!(transactions[0].amount, 12.34);
            let requests = server.finish();
            assert!(requests[0].starts_with("GET /mon-budget/generate HTTP/1.1"));
            assert!(requests[1].starts_with("POST /budget/exporter-mouvements HTTP/1.1"));
            assert!(requests[1].contains("movementSearch%5B_token%5D=fixture-csrf"));
            assert!(requests[1].contains("session=fixture"));
            assert!(requests[2].starts_with("GET /fixture/export.csv?download=1 HTTP/1.1"));
            assert!(requests[2].contains("session=fixture"));
            assert!(requests[2].contains("redirected=yes"));
        });
    }
}

#[test]
fn session_cookies_dynamic_api_and_public_ticks_share_local_transport() {
    run(async {
        let config = super::config::Config {
            api_url: "https://api.boursobank.com/services/api/v1.7".into(),
            user_hash: Some("fixture".into()),
            ..Default::default()
        };
        let page = format!(
            r#"<input name="form[_token]" value="fixture-token" > <script>window.BRS_CONFIG = {};</script>"#,
            serde_json::to_string(&config).unwrap()
        );
        let server = Server::new(vec![
            ok("document.cookie=\"__brs_mit=fixture-mit;\";"),
            ok(&page),
            ok(&super::newpad_tests::keyboard(
                &super::newpad_tests::entries(),
            )),
            ok("[]"),
            ok("[]"),
            ok(r#"{"d":{"Name":"Fixture","SymbolId":"TEST","Xperiod":0,"QuoteTab":[]}}"#),
        ]);
        let mut client = fixture_client(server.address);
        client.init_session().await.unwrap();
        assert_eq!(client.token, "fixture-token");
        assert_eq!(client.challenge_id, "fixture-challenge");
        client
            .get_trading_summary(crate::account::Account {
                id: "fake".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        // No captured base: later configuration changes still map to the fixture.
        client.config.api_url = "https://changed.invalid/changed-api".into();
        client
            .get_trading_summary(crate::account::Account {
                id: "fake".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        let ticks = client.get_ticks("TEST", 30, 0).await.unwrap();
        assert_eq!(ticks.d.name, "Fixture");
        let requests = server.finish();
        assert_eq!(requests.len(), 6);
        for request in &requests[1..] {
            assert!(request.contains("__brs_mit=fixture-mit"));
            assert!(request.contains("brsDomainMigration=migrated"));
        }
        assert!(requests[2].starts_with("GET /connexion/clavier-virtuel?_hinclude=1 "));
        assert!(requests[3]
            .starts_with("GET /services/api/v1.7/_user_/_fixture_/trading/accounts/summary/fake?"));
        assert!(requests[4]
            .starts_with("GET /changed-api/_user_/_fixture_/trading/accounts/summary/fake?"));
        assert!(requests[5].starts_with(
            "GET /bourse/action/graph/ws/GetTicksEOD?symbol=TEST&length=30&period=0&guid= "
        ));
    });
}

#[test]
fn automatic_redirects_stay_disabled() {
    run(async {
        let server = Server::new(vec![response(
            "302 Found",
            "Location: https://outside.invalid/not-followed\r\n",
            "",
        )]);
        let client = fixture_client(server.address);
        let response = client.client.get(DESTINATIONS[0]).send().await.unwrap();
        assert_eq!(response.status(), 302);
        assert_eq!(server.finish().len(), 1);
    });
}

#[test]
fn public_request_timeout_disconnects_without_fallback() {
    run(async {
        let mut server = Server::new(vec![Reply::Hold]);
        let client = BoursoWebClient::for_test(server.address, Duration::from_millis(150)).unwrap();
        let error = client.get_ticks("TEST", 1, 0).await.unwrap_err();
        assert!(error.downcast_ref::<reqwest::Error>().unwrap().is_timeout());
        assert!(matches!(server.event().await, Event::Request(_)));
        assert!(matches!(server.event().await, Event::Disconnected));
        assert_eq!(server.finish().len(), 1);
    });
}

#[test]
fn dropping_public_request_cancels_in_flight_io() {
    run(async {
        let mut server = Server::new(vec![Reply::Hold]);
        let client = fixture_client(server.address);
        let pending = Box::pin(client.get_ticks("TEST", 1, 0));
        let event = Box::pin(server.event());
        match futures_util::future::select(event, pending).await {
            futures_util::future::Either::Left((Event::Request(request), pending)) => {
                assert!(request.starts_with("GET /bourse/action/graph/ws/GetTicksEOD?"));
                drop(pending);
            }
            _ => panic!("held request completed before cancellation"),
        }
        assert!(matches!(server.event().await, Event::Disconnected));
        assert_eq!(server.finish().len(), 1);
    });
}

#[test]
fn transport_status_and_parse_errors_propagate() {
    run(async {
        for (reply, expected) in [
            (
                response("503 Service Unavailable", "", "fixture unavailable"),
                "503",
            ),
            (ok("not-json"), "Failed to parse get ticks response"),
        ] {
            let server = Server::new(vec![reply]);
            let client = fixture_client(server.address);
            let error = client.get_ticks("TEST", 1, 0).await.unwrap_err();
            assert!(error.to_string().contains(expected));
            assert_eq!(server.finish().len(), 1);
        }
        let server = Server::new(vec![Reply::Close]);
        let client = fixture_client(server.address);
        let error = client.get_ticks("TEST", 1, 0).await.unwrap_err();
        assert!(error.downcast_ref::<reqwest::Error>().is_some());
        assert_eq!(server.finish().len(), 1);
    });
}

// Logging regressions exercise the real MFA branches through the confined seam.
const LOG_SECRETS: &[&str] = &[
    "SENTINEL_BODY",
    "SENTINEL_USER",
    "SENTINEL_OTP",
    "SENTINEL_TOKEN",
    "SENTINEL_STATE",
    "SENTINEL_QR",
];

fn logging_config() -> super::config::Config {
    super::config::Config {
        api_url: "https://fixture.invalid/api".into(),
        user_hash: Some("SENTINEL_USER".into()),
        default_api_bearer: "SENTINEL_TOKEN".into(),
        ..Default::default()
    }
}

fn logging_mfa_page() -> String {
    let params = serde_json::json!({"challenges":[{"parameters":{"formScreen":{"actions":{"check":{"api":{"params":{"resourceId":"SENTINEL_OTP","formState":"SENTINEL_STATE"}}}}}}}]});
    format!(
        r#"brs-otp-webtoapp <input name="form[_token]" value="SENTINEL_TOKEN" > window.BRS_CONFIG = {}; <div data-strong-authentication-payload="{}">"#,
        serde_json::to_string(&logging_config()).unwrap(),
        params.to_string().replace('"', "&quot;")
    )
}

#[test]
fn logging_mfa_request_branches() {
    use super::logging_tests::{assert_clean, capture};
    for enabled in [true, false] {
        for page in [
            "SENTINEL_BODY",
            "brs-otp-sms SENTINEL_BODY",
            "brs-otp-email SENTINEL_BODY",
        ] {
            let (_, logs) = capture(enabled, || {
                run(async {
                    let server = Server::new(vec![ok(""), ok(page)]);
                    let error = fixture_client(server.address)
                        .request_mfa()
                        .await
                        .unwrap_err();
                    assert_clean(Some(&error), "", LOG_SECRETS);
                    assert_eq!(server.finish().len(), 2);
                })
            });
            assert_clean(None, &logs, LOG_SECRETS);
        }
        for (case, reply) in [
            ok(r#"{"success":true,"secret":"SENTINEL_BODY"}"#),
            ok(r#"{"success":false,"secret":"SENTINEL_BODY"}"#),
            response("503 Unavailable", "", "SENTINEL_BODY"),
            ok(r#"{"secret":"SENTINEL_BODY", invalid}"#),
            Reply::Close,
        ]
        .into_iter()
        .enumerate()
        {
            let (_, logs) = capture(enabled, || {
                run(async {
                    let server = Server::new(vec![ok(""), ok(&logging_mfa_page()), reply]);
                    let result = fixture_client(server.address).request_mfa().await;
                    assert_eq!(result.is_ok(), case == 0);
                    assert_clean(result.as_ref().err(), "", LOG_SECRETS);
                    assert_eq!(server.finish().len(), 3);
                })
            });
            assert_clean(None, &logs, LOG_SECRETS);
        }
    }
}

#[test]
fn logging_mfa_check_branches() {
    use super::logging_tests::{assert_clean, capture};
    for enabled in [true, false] {
        let authenticated = format!(
            r#"href="/se-deconnecter" window.BRS_CONFIG = {};"#,
            serde_json::to_string(&logging_config()).unwrap()
        );
        for (case, replies) in [
            vec![response("503 Unavailable", "", "SENTINEL_BODY")],
            vec![ok(r#"{"success":false,"secret":"SENTINEL_BODY"}"#)],
            vec![ok(r#"{"success":false,"qrcode":"SENTINEL_QR"}"#)],
            vec![ok(r#"{"secret":"SENTINEL_BODY", invalid}"#)],
            vec![Reply::Close],
            vec![
                ok(r#"{"success":true}"#),
                response("302 Found", "", ""),
                ok("SENTINEL_BODY"),
            ],
            vec![
                ok(r#"{"success":true}"#),
                response("302 Found", "", ""),
                ok(&authenticated),
            ],
        ]
        .into_iter()
        .enumerate()
        {
            let (_, logs) = capture(enabled, || {
                run(async {
                    let count = replies.len();
                    let server = Server::new(replies);
                    let mut client = fixture_client(server.address);
                    client.config = logging_config();
                    let result = client
                        .check_mfa(
                            super::MfaType::WebToApp,
                            "SENTINEL_OTP".into(),
                            "SENTINEL_STATE".into(),
                            "SENTINEL_TOKEN".into(),
                        )
                        .await;
                    match case {
                        1 => assert!(!result.as_ref().unwrap()),
                        6 => assert!(*result.as_ref().unwrap()),
                        2 => match result
                            .as_ref()
                            .unwrap_err()
                            .downcast_ref::<super::error::ClientError>()
                        {
                            Some(super::error::ClientError::QRCodeRequired(code)) => {
                                assert_eq!(code, "SENTINEL_QR")
                            }
                            _ => panic!("QR payload must remain available to the MFA consumer"),
                        },
                        _ => assert!(result.is_err()),
                    }
                    assert_clean(result.as_ref().err(), "", LOG_SECRETS);
                    assert_eq!(server.finish().len(), count);
                })
            });
            assert_clean(None, &logs, LOG_SECRETS);
        }
    }
}

#[test]
fn logging_login_account_trade_and_export_failures() {
    use super::logging_tests::{assert_clean, capture};
    for enabled in [true, false] {
        let (_, logs) = capture(enabled, || {
            run(async {
                let server = Server::new(vec![ok("SENTINEL_BODY")]);
                let mut client = fixture_client(server.address);
                client.virtual_pad_ids = super::newpad_tests::entries()
                    .into_iter()
                    .map(|entry| entry.0)
                    .collect();
                let error = client.login("SENTINEL_USER", "12345678").await.unwrap_err();
                assert_clean(Some(&error), "", LOG_SECRETS);
                server.finish();
                let server = Server::new(vec![response("302 Found", "", ""), ok("SENTINEL_BODY")]);
                let mut client = fixture_client(server.address);
                client.virtual_pad_ids = super::newpad_tests::entries()
                    .into_iter()
                    .map(|entry| entry.0)
                    .collect();
                let error = client.login("SENTINEL_USER", "12345678").await.unwrap_err();
                assert_clean(Some(&error), "", LOG_SECRETS);
                server.finish();
                let server = Server::new(vec![ok("SENTINEL_BODY")]);
                let error = fixture_client(server.address)
                    .get_accounts(Some(crate::account::AccountKind::Banking))
                    .await
                    .unwrap_err();
                assert_clean(Some(&error), "", LOG_SECRETS);
                server.finish();
                for reply in [
                    response("503 Unavailable", "", "SENTINEL_BODY"),
                    ok(r#""SENTINEL_BODY""#),
                    Reply::Close,
                ] {
                    let server = Server::new(vec![reply]);
                    let mut client = fixture_client(server.address);
                    client.config = logging_config();
                    let error = client
                        .get_trading_summary(crate::account::Account {
                            id: "SENTINEL_USER".into(),
                            ..Default::default()
                        })
                        .await
                        .unwrap_err();
                    assert_clean(Some(&error), "", LOG_SECRETS);
                    server.finish();
                }
                for reply in [
                    response("503 Unavailable", "", "SENTINEL_BODY"),
                    ok(r#""SENTINEL_BODY""#),
                    Reply::Close,
                ] {
                    let server = Server::new(vec![reply]);
                    let error = fixture_client(server.address)
                        .get_ticks("SENTINEL_TOKEN", 1, 0)
                        .await
                        .unwrap_err();
                    assert_clean(Some(&error), "", LOG_SECRETS);
                    server.finish();
                }
                let server = Server::new(vec![
                    ok(r#"<input name="movementSearch[_token]" value="SENTINEL_TOKEN">"#),
                    response("302 Found", "Location: /SENTINEL_TOKEN\r\n", ""),
                    ok("<html>SENTINEL_BODY"),
                ]);
                let error = fixture_client(server.address)
                    .get_transactions("SENTINEL_USER", "01/01/2026", "02/01/2026")
                    .await
                    .unwrap_err();
                assert_clean(Some(&error), "", LOG_SECRETS);
                server.finish();
            })
        });
        assert_clean(None, &logs, LOG_SECRETS);
    }
}

#[test]
fn logging_transfer_all_response_diagnostics() {
    use super::logging_tests::{assert_clean, capture};
    use futures_util::StreamExt;
    for enabled in [true, false] {
        // Every transfer step's non-success response, then the final missing
        // confirmation message. Headers contain a secret to catch Debug(Response).
        for fail_at in 0..=8 {
            let (_, logs) = capture(enabled, || {
                run(async {
                    let mut replies = vec![
                    response("302 Found", "Location: /compte/cav/SENTINEL_USER/virements/immediat/nouveau/SENTINEL_TOKEN/1\r\n", ""),
                    ok(r#"<input name="flow_ImmediateCashTransfer_instance" value="SENTINEL_STATE">"#),
                    ok(""), ok(""), ok(""), ok(""), ok(""), ok("Confirmation"),
                ];
                    let count = if fail_at < 8 {
                        replies.truncate(fail_at + 1);
                        replies[fail_at] = response(
                            "503 Unavailable",
                            "Set-Cookie: session=SENTINEL_TOKEN\r\n",
                            "SENTINEL_BODY",
                        );
                        fail_at + 1
                    } else {
                        replies[7] = ok("SENTINEL_BODY");
                        8
                    };
                    let server = Server::new(replies);
                    let client = fixture_client(server.address);
                    let account = crate::account::Account {
                        id: "SENTINEL_USER".into(),
                        ..Default::default()
                    };
                    let results: Vec<_> = client
                        .transfer_funds(
                            12.0,
                            account.clone(),
                            account,
                            Some("SENTINEL_BODY".into()),
                        )
                        .collect()
                        .await;
                    if fail_at == 8 {
                        assert!(matches!(
                            results.last().unwrap(),
                            Ok(crate::client::transfer::TransferProgress::SubmittedUnverified)
                        ));
                    } else {
                        let error = results.last().unwrap().as_ref().unwrap_err();
                        assert_clean(Some(error), "", LOG_SECRETS);
                    }
                    assert_eq!(server.finish().len(), count);
                })
            });
            assert_clean(None, &logs, LOG_SECRETS);
        }
    }
}

#[test]
fn logging_order_quote_response_diagnostics_and_success() {
    use super::{
        logging_tests::{assert_clean, capture},
        trade::order::{OrderCheckResponse, OrderConfirmResponse, OrderPrepareResponse, OrderSide},
    };
    let mut prepare = OrderPrepareResponse {
        resource_id: "SENTINEL_TOKEN".into(),
        performance_url: "https://fixture.invalid/SENTINEL_TOKEN".into(),
        ..Default::default()
    };
    prepare.account.iban = "SENTINEL_USER".into();
    let prepare = serde_json::to_string(&prepare).unwrap();
    let mut check_value = serde_json::to_value(OrderCheckResponse::default()).unwrap();
    check_value["checkOrderData"]["orderSide"] = serde_json::json!("B");
    check_value["checkOrderData"]["orderQuantity"] = serde_json::json!(2);
    check_value["checkOrderData"]["orderPriceLimit"] = serde_json::json!(0.0);
    let check = serde_json::to_string(&check_value).unwrap();
    let confirm = serde_json::to_string(&OrderConfirmResponse {
        order_id: "SENTINEL_USER".into(),
        ..Default::default()
    })
    .unwrap();
    for enabled in [true, false] {
        for fail_at in 0..=3 {
            for malformed in [false, true] {
                let (_, logs) = capture(enabled, || {
                    run(async {
                        let mut replies = vec![
                            ok(&prepare),
                            ok(&check),
                            response("201 Created", "", &confirm),
                        ];
                        if fail_at < 3 {
                            replies.truncate(fail_at + 1);
                            replies[fail_at] = if malformed {
                                response(
                                    if fail_at == 2 {
                                        "201 Created"
                                    } else {
                                        "200 OK"
                                    },
                                    "",
                                    r#""SENTINEL_BODY""#,
                                )
                            } else {
                                response("503 Unavailable", "", "SENTINEL_BODY")
                            };
                        }
                        let count = replies.len();
                        let server = Server::new(replies);
                        let mut client = fixture_client(server.address);
                        client.config = logging_config();
                        let account = crate::account::Account {
                            id: "SENTINEL_USER".into(),
                            ..Default::default()
                        };
                        let result = client
                            .order(OrderSide::Buy, &account, "TEST", 2, None)
                            .await;
                        assert_eq!(result.is_ok(), fail_at == 3);
                        assert_clean(result.as_ref().err(), "", LOG_SECRETS);
                        assert_eq!(server.finish().len(), count);
                    })
                });
                assert_clean(None, &logs, LOG_SECRETS);
            }
        }
        for reply in [
            ok("{}"),
            ok(r#""SENTINEL_BODY""#),
            response("503 Unavailable", "", "SENTINEL_BODY"),
            Reply::Close,
        ] {
            let (_, logs) = capture(enabled, || {
                run(async {
                    let server = Server::new(vec![reply]);
                    let mut client = fixture_client(server.address);
                    client.config = logging_config();
                    let result = client.instrument_quote("TEST").await;
                    assert_clean(result.as_ref().err(), "", LOG_SECRETS);
                    server.finish();
                })
            });
            assert_clean(None, &logs, LOG_SECRETS);
        }
        for reply in [
            ok("SENTINEL_BODY"),
            response("503 Unavailable", "", "SENTINEL_BODY"),
            Reply::Close,
        ] {
            let (_, logs) = capture(enabled, || {
                run(async {
                    let server = Server::new(vec![reply]);
                    let mut client = fixture_client(server.address);
                    client.config = logging_config();
                    let result = client
                        .cancel_order(&crate::account::Account::default(), "SENTINEL_USER")
                        .await;
                    assert_clean(result.as_ref().err(), "", LOG_SECRETS);
                    server.finish();
                })
            });
            assert_clean(None, &logs, LOG_SECRETS);
        }
    }
}

#[test]
fn logging_session_login_and_empty_export_success() {
    use super::logging_tests::{assert_clean, capture};
    let mut config = logging_config();
    config.app_release_date = "SENTINEL_BODY".into();
    let config = serde_json::to_string(&config).unwrap();
    for enabled in [true, false] {
        let (_, logs) = capture(enabled, || {
            run(async {
                let server = Server::new(vec![
                    ok("__brs_mit=SENTINEL_TOKEN;"),
                    ok(&format!(
                        r#"<input name="form[_token]" value="SENTINEL_TOKEN" > window.BRS_CONFIG = {config};"#
                    )),
                    ok(
                        &super::newpad_tests::keyboard(&super::newpad_tests::entries())
                            .replace("fixture-challenge", "SENTINEL_TOKEN"),
                    ),
                ]);
                fixture_client(server.address).init_session().await.unwrap();
                assert_eq!(server.finish().len(), 3);
                let server = Server::new(vec![
                    response("302 Found", "", ""),
                    ok(&format!(
                        r#"href="/se-deconnecter" window.BRS_CONFIG = {config};"#
                    )),
                ]);
                let mut client = fixture_client(server.address);
                client.virtual_pad_ids = super::newpad_tests::entries()
                    .into_iter()
                    .map(|entry| entry.0)
                    .collect();
                client.login("SENTINEL_USER", "12345678").await.unwrap();
                assert_eq!(server.finish().len(), 2);
                let server = Server::new(vec![
                    ok(r#"<input name="movementSearch[_token]" value="SENTINEL_TOKEN">"#),
                    ok(
                        "dateOp;dateVal;label;category;categoryParent;supplierFound;amount;comment;accountNum;accountLabel;accountbalance\n",
                    ),
                ]);
                let result = fixture_client(server.address)
                    .get_transactions("SENTINEL_USER", "01/01/2026", "02/01/2026")
                    .await
                    .unwrap();
                assert!(result.is_empty());
                assert_eq!(server.finish().len(), 2);
            })
        });
        assert_clean(None, &logs, LOG_SECRETS);
    }
}

#[test]
fn export_rejects_status_and_unverified_empty_bodies() {
    let dir = std::env::temp_dir().join(format!("bourso-http-export-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    let destination = dir.join("old.csv");
    std::fs::write(&destination, b"previous export").unwrap();
    for reply in [
        response("401 Unauthorized", "", ""),
        response("403 Forbidden", "", "dateOp;dateVal;label;category;categoryParent;supplierFound;amount;comment;accountNum;accountLabel;accountbalance\n"),
        response("500 Error", "", ""),
        ok(""),
        ok(r#"<html><input name="movementSearch[_token]" value="fixture">"#),
        ok(" \n<HTML><form name=login>expired</form>"),
    ] {
        run(async {
            let server = Server::new(vec![ok(r#"<input name="movementSearch[_token]" value="fixture">"#), reply]);
            let result = fixture_client(server.address).get_transactions("fake", "01/01/2026", "02/01/2026").await
                .and_then(|transactions| super::transaction::transactions_to_csv(&transactions))
                .and_then(|csv| super::transaction::write_export_atomically(&destination, csv.as_bytes()));
            assert!(result.is_err());
            assert_eq!(std::fs::read(&destination).unwrap(), b"previous export");
            assert_eq!(server.finish().len(), 2);
        });
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn export_rejects_external_redirect_before_sending() {
    for location in [
        "http://clients.boursobank.com/fixture",
        "https://outside.invalid/fixture",
        "//outside.invalid/fixture",
        "https://clients.boursobank.com:444/fixture",
        "https://clients.boursobank.com@outside.invalid/fixture",
    ] {
        run(async {
            let server = Server::new(vec![
                ok(r#"<input name="movementSearch[_token]" value="fixture">"#),
                response("302 Found", &format!("Location: {location}\r\n"), ""),
                ok("dateOp;dateVal;label;category;categoryParent;supplierFound;amount;comment;accountNum;accountLabel;accountbalance\n"),
            ]);
            let result = fixture_client(server.address)
                .get_transactions("fake", "01/01/2026", "02/01/2026")
                .await;
            assert!(result.is_err());
            // Drop stops the server even though the sentinel third response must
            // never be consumed. finish waits for all scripted responses.
            server.stop.store(true, Ordering::Relaxed);
            assert_eq!(server.finish().len(), 2);
        });
    }
}

#[test]
fn export_form_status_missing_redirect_and_redirect_error_rejected() {
    run(async {
        for status in ["401 Unauthorized", "403 Forbidden", "500 Error"] {
            let server = Server::new(vec![response(
                status,
                "",
                r#"<input name="movementSearch[_token]" value="fixture">"#,
            )]);
            assert!(fixture_client(server.address)
                .get_transactions("fake", "01/01/2026", "02/01/2026")
                .await
                .is_err());
            assert_eq!(server.finish().len(), 1);
        }
        for headers in ["", "Location: \r\n", "Location: https://[invalid\r\n"] {
            let server = Server::new(vec![
                ok(r#"<input name="movementSearch[_token]" value="fixture">"#),
                response("302 Found", headers, ""),
            ]);
            assert!(fixture_client(server.address)
                .get_transactions("fake", "01/01/2026", "02/01/2026")
                .await
                .is_err());
            assert_eq!(server.finish().len(), 2);
        }
        let server = Server::new(vec![ok(r#"<input name="movementSearch[_token]" value="fixture">"#), response("302 Found", "Location: /download.csv\r\n", ""), response("500 Error", "", "dateOp;dateVal;label;category;categoryParent;supplierFound;amount;comment;accountNum;accountLabel;accountbalance\n")]);
        assert!(fixture_client(server.address)
            .get_transactions("fake", "01/01/2026", "02/01/2026")
            .await
            .is_err());
        assert_eq!(server.finish().len(), 3);
    });
}

#[test]
fn export_only_verified_header_means_empty_including_bom() {
    for prefix in ["", "\u{feff}"] {
        run(async {
            let body = format!("{prefix}dateOp;dateVal;label;category;categoryParent;supplierFound;amount;comment;accountNum;accountLabel;accountbalance\n");
            let server = Server::new(vec![
                ok(r#"<input name="movementSearch[_token]" value="fixture">"#),
                ok(&body),
            ]);
            assert!(fixture_client(server.address)
                .get_transactions("fake", "01/01/2026", "02/01/2026")
                .await
                .unwrap()
                .is_empty());
            assert_eq!(server.finish().len(), 2);
        });
    }
}

// PUBLICUNKNOWN1-3: traverse the public workflows, not private final-step helpers.
// These synthetic responses establish transport behavior, never bank execution.
const SUBMISSION_QUIET_WINDOW: Duration = Duration::from_millis(150);

#[derive(Clone, Copy, Debug)]
enum PublicSubmission {
    Order,
    OrderDetailed,
    TransferFunds,
    TransferMoney,
}

impl PublicSubmission {
    fn operation(self) -> crate::outcome::OutcomeOperation {
        use crate::outcome::OutcomeOperation;
        match self {
            Self::Order | Self::OrderDetailed => OutcomeOperation::Order,
            Self::TransferFunds | Self::TransferMoney => OutcomeOperation::Transfer,
        }
    }

    fn prefix(self) -> Vec<Reply> {
        use super::trade::order::{OrderCheckResponse, OrderPrepareResponse};
        match self {
            Self::Order | Self::OrderDetailed => {
                let prepare = OrderPrepareResponse {
                    resource_id: "SENTINEL_TOKEN".into(),
                    ..Default::default()
                };
                let mut check = serde_json::to_value(OrderCheckResponse::default()).unwrap();
                check["checkOrderData"]["orderSide"] = serde_json::json!("B");
                check["checkOrderData"]["orderQuantity"] = serde_json::json!(2);
                check["checkOrderData"]["orderPriceLimit"] = serde_json::json!(0.0);
                vec![ok(&serde_json::to_string(&prepare).unwrap()), ok(&check.to_string())]
            }
            Self::TransferFunds | Self::TransferMoney => vec![
                response("302 Found", "Location: /compte/cav/SENTINEL_USER/virements/immediat/nouveau/SENTINEL_TOKEN/1\r\n", ""),
                ok(r#"<input name="flow_ImmediateCashTransfer_instance" value="SENTINEL_STATE">"#),
                ok(""), ok(""), ok(""), ok(""), ok(""),
            ],
        }
    }

    fn request_lines(self) -> Vec<String> {
        match self {
            Self::Order | Self::OrderDetailed => vec![
                "GET /api/_user_/_SENTINEL_USER_/trading/order/prepare?_host=tradingboard.boursobank.com&searchExtendedHours=false&selectedAccount=SENTINEL_USER&symbol=TEST HTTP/1.1".into(),
                "POST /api/_user_/_SENTINEL_USER_/trading/ordersimple/check HTTP/1.1".into(),
                "POST /api/_user_/_SENTINEL_USER_/trading/ordersimple/confirm HTTP/1.1".into(),
            ],
            Self::TransferFunds | Self::TransferMoney => {
                let base = "/compte/cav/SENTINEL_USER/virements/immediat/nouveau";
                let mut lines = vec![format!("GET {base} HTTP/1.1")];
                for (method, step) in [("GET", 1), ("POST", 2), ("POST", 3), ("POST", 4), ("POST", 5), ("POST", 8), ("POST", 10)] {
                    lines.push(format!("{method} {base}/SENTINEL_TOKEN/{step} HTTP/1.1"));
                }
                lines
            }
        }
    }

    // This future owns any stream it drives. Dropping it drops the stream itself,
    // not merely a next() future borrowing an otherwise-live stream.
    async fn submit(self, client: &BoursoWebClient) -> anyhow::Result<()> {
        use super::{trade::order::OrderSide, transfer::TransferProgress};
        use futures_util::{Stream, StreamExt};
        let account = crate::account::Account {
            id: "SENTINEL_USER".into(),
            ..Default::default()
        };
        match self {
            Self::Order => client
                .order(OrderSide::Buy, &account, "TEST", 2, None)
                .await
                .map(|_| ()),
            Self::OrderDetailed => client
                .order_detailed(OrderSide::Buy, &account, "TEST", 2, None)
                .await
                .map(|_| ()),
            Self::TransferFunds | Self::TransferMoney => {
                let mut stream: std::pin::Pin<
                    Box<dyn Stream<Item = anyhow::Result<TransferProgress>> + '_>,
                > = match self {
                    Self::TransferFunds => Box::pin(client.transfer_funds(
                        12.0,
                        account.clone(),
                        account,
                        Some("SENTINEL_BODY".into()),
                    )),
                    Self::TransferMoney => Box::pin(client.transfer_money(
                        "12.00".parse().unwrap(),
                        account.clone(),
                        account,
                        Some("SENTINEL_BODY".into()),
                    )),
                    _ => unreachable!(),
                };
                let mut steps = Vec::new();
                while let Some(item) = stream.next().await {
                    match item {
                        Ok(progress) => {
                            // No SubmittedUnverified or legacy Completed is allowed
                            // on these final-failure/cancellation paths.
                            assert!(
                                progress.step_number() < 10,
                                "unexpected terminal success: {progress:?}"
                            );
                            steps.push(progress.step_number());
                        }
                        Err(error) => {
                            assert_eq!(steps, (1..=9).collect::<Vec<_>>());
                            assert!(
                                stream.next().await.is_none(),
                                "one error then exhaustion required"
                            );
                            return Err(error);
                        }
                    }
                }
                panic!("transfer exhausted without its expected final error")
            }
        }
    }

    fn assert_requests(self, requests: &[String]) {
        let expected = self.request_lines();
        let actual: Vec<_> = requests
            .iter()
            .map(|request| request.lines().next().unwrap())
            .collect();
        assert_eq!(
            actual, expected,
            "full public request sequence for {self:?}"
        );
        assert_eq!(
            actual
                .iter()
                .filter(|line| **line == expected.last().unwrap())
                .count(),
            1,
            "exactly one final POST"
        );
        for request in requests {
            let (headers, body) = request.split_once("\r\n\r\n").unwrap();
            if headers.starts_with("POST ") {
                let length: usize = headers
                    .lines()
                    .find_map(|line| {
                        let (key, value) = line.split_once(':')?;
                        key.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse().unwrap())
                    })
                    .unwrap();
                assert_eq!(body.len(), length, "recorded POST must be complete");
                assert!(length > 0);
            }
        }
        let body = requests.last().unwrap().split_once("\r\n\r\n").unwrap().1;
        match self {
            Self::Order | Self::OrderDetailed => assert_eq!(
                serde_json::from_str::<serde_json::Value>(body).unwrap(),
                serde_json::json!({"resourceId": "SENTINEL_TOKEN"})
            ),
            Self::TransferFunds | Self::TransferMoney => {
                assert!(body.contains(
                    "name=\"flow_ImmediateCashTransfer_instance\"\r\n\r\nSENTINEL_STATE\r\n"
                ));
                assert!(body.contains("name=\"flow_ImmediateCashTransfer_step\"\r\n\r\n9\r\n"));
                assert!(
                    body.ends_with("--\r\n"),
                    "complete multipart closing boundary"
                );
            }
        }
    }
}

fn public_submission_failures(api: PublicSubmission) {
    use super::logging_tests::{assert_clean, capture};
    use crate::outcome::{OutcomeStage, OutcomeUnknown};
    for enabled in [true, false] {
        let status = if api.operation() == crate::outcome::OutcomeOperation::Order {
            "201 Created"
        } else {
            "200 OK"
        };
        let mut cases = vec![
            (Reply::Close, OutcomeStage::Sending),
            (Reply::Http(format!("HTTP/1.1 {status}\r\nContent-Length: 999\r\nConnection: close\r\n\r\nSENTINEL_BODY")), OutcomeStage::ReadingResponse),
            (response("503 Unavailable", "Set-Cookie: session=SENTINEL_TOKEN\r\n", "SENTINEL_BODY"), OutcomeStage::HttpStatus),
        ];
        if api.operation() == crate::outcome::OutcomeOperation::Order {
            cases.push((
                response("201 Created", "", r#"{"secret":"SENTINEL_BODY",invalid}"#),
                OutcomeStage::ParsingResponse,
            ));
        }
        for (reply, stage) in cases {
            let (_, logs) = capture(enabled, || {
                run(async {
                    let mut replies = api.prefix();
                    replies.extend([reply, Reply::ObserveQuiet(SUBMISSION_QUIET_WINDOW)]);
                    let server = Server::new(replies);
                    let mut client = fixture_client(server.address);
                    client.config = logging_config();
                    let error = api.submit(&client).await.unwrap_err();
                    let assert_unknown = |error: &anyhow::Error| {
                        let unknown = error
                            .downcast_ref::<OutcomeUnknown>()
                            .expect("typed outcome survives public wrapper and caller context");
                        assert_eq!(unknown.operation, api.operation());
                        assert_eq!(unknown.stage, stage);
                        assert_eq!(unknown.reference(), "SENTINEL_TOKEN");
                        assert_eq!(unknown.correlation.len(), 32);
                        assert!(unknown
                            .correlation
                            .bytes()
                            .all(|byte| byte.is_ascii_hexdigit()));
                        assert!(
                            format!("{error:#}").contains("Do not reissue or retry automatically")
                        );
                        assert_clean(Some(error), "", LOG_SECRETS);
                        assert_clean(
                            None,
                            &format!("{unknown} {unknown:#} {unknown:?} {unknown:#?}"),
                            LOG_SECRETS,
                        );
                    };
                    assert_unknown(&error);
                    assert_unknown(&error.context("public submission caller context"));
                    // Keep the runtime polling until the server's bounded quiet
                    // window ends, rather than blocking it in worker.join().
                    let mut server = server;
                    for _ in api.request_lines() {
                        assert!(matches!(server.event().await, Event::Request(_)));
                    }
                    assert!(matches!(server.event().await, Event::QuietObserved));
                    api.assert_requests(&server.finish());
                })
            });
            assert_clean(None, &logs, LOG_SECRETS);
        }
    }
}

#[test]
fn public_order_final_failures_are_unknown_without_retry() {
    public_submission_failures(PublicSubmission::Order);
}

#[test]
fn public_order_detailed_final_failures_are_unknown_without_retry() {
    public_submission_failures(PublicSubmission::OrderDetailed);
}

#[test]
fn public_transfer_funds_final_failures_are_unknown_then_exhausted() {
    public_submission_failures(PublicSubmission::TransferFunds);
}

#[test]
fn public_transfer_money_final_failures_are_unknown_then_exhausted() {
    public_submission_failures(PublicSubmission::TransferMoney);
}

fn cancel_public_submission_after_complete_final_post(api: PublicSubmission) {
    run(async {
        let mut replies = api.prefix();
        replies.extend([Reply::Hold, Reply::ObserveQuiet(SUBMISSION_QUIET_WINDOW)]);
        let mut server = Server::new(replies);
        let mut client = fixture_client(server.address);
        client.config = logging_config();
        let mut pending = Box::pin(api.submit(&client));
        let mut observed = Vec::new();
        for expected in api.request_lines() {
            match futures_util::future::select(Box::pin(server.event()), pending).await {
                futures_util::future::Either::Left((Event::Request(request), still_pending)) => {
                    assert_eq!(request.lines().next().unwrap(), expected);
                    observed.push(request);
                    pending = still_pending;
                }
                _ => panic!("held public submission completed before owning future/stream drop"),
            }
        }
        // Event::Request is sent only after read_request receives Content-Length
        // bytes. Confirm the final payload before actually cancelling the owner.
        api.assert_requests(&observed);
        drop(pending);
        // This is shorter than the client's two-second request timeout: a
        // forgotten owner must not pass merely by eventually timing out.
        let disconnected = tokio::time::timeout(Duration::from_millis(500), server.event())
            .await
            .expect("owner drop must disconnect before the request timeout");
        assert!(matches!(disconnected, Event::Disconnected));
        assert!(matches!(server.event().await, Event::QuietObserved));
        api.assert_requests(&server.finish());
        // The quiet listener stays alive for 150 ms after disconnect, so a retry
        // would fail this test rather than merely meet a closed listening socket.
    });
}

#[test]
fn dropping_public_order_owner_after_final_post_disconnects_without_retry() {
    for api in [PublicSubmission::Order, PublicSubmission::OrderDetailed] {
        cancel_public_submission_after_complete_final_post(api);
    }
}

#[test]
fn dropping_public_transfer_owner_after_final_post_disconnects_without_retry() {
    for api in [
        PublicSubmission::TransferFunds,
        PublicSubmission::TransferMoney,
    ] {
        cancel_public_submission_after_complete_final_post(api);
    }
}

// LIBGUARD: real public order/transfer entrypoints through the opt-in guard.
#[cfg(unix)]
struct ReplayTemp(std::path::PathBuf);
#[cfg(unix)]
impl ReplayTemp {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!(
            "bourso-api-loopback-replay-{}-{:032x}",
            std::process::id(),
            rand::random::<u128>()
        )))
    }
}
#[cfg(unix)]
impl Drop for ReplayTemp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[cfg(unix)]
const GUARDED_APIS: [PublicSubmission; 4] = [
    PublicSubmission::Order,
    PublicSubmission::OrderDetailed,
    PublicSubmission::TransferFunds,
    PublicSubmission::TransferMoney,
];

#[test]
#[cfg(unix)]
fn replay_guard_public_errors_preserve_unknown_downcast_and_block_reopen() {
    use crate::{replay_guard::ReplayGuard, OutcomeUnknown};
    for api in GUARDED_APIS {
        run(async {
            let temp = ReplayTemp::new();
            let mut guard = ReplayGuard::open(&temp.0).unwrap();
            let mut replies = api.prefix();
            replies.extend([Reply::Close, Reply::ObserveQuiet(SUBMISSION_QUIET_WINDOW)]);
            let mut server = Server::new(replies);
            let mut client = fixture_client(server.address);
            client.config = logging_config();
            let error = guard
                .invoke(api.operation(), || {
                    let json: serde_json::Value = serde_json::from_slice(
                        &std::fs::read(temp.0.join("replay-unresolved.json")).unwrap(),
                    )
                    .unwrap();
                    assert_eq!(json["version"], 1);
                    api.submit(&client)
                })
                .await
                .unwrap()
                .unwrap_err()
                .context("guard caller");
            let unknown = error.downcast_ref::<OutcomeUnknown>().unwrap();
            assert_eq!(unknown.operation, api.operation());
            assert_eq!(unknown.reference(), "SENTINEL_TOKEN");
            let bytes = std::fs::read(temp.0.join("replay-unresolved.json")).unwrap();
            super::logging_tests::assert_clean(
                None,
                &String::from_utf8(bytes.clone()).unwrap(),
                LOG_SECRETS,
            );
            drop(guard);
            let mut reopened = ReplayGuard::open(&temp.0).unwrap();
            assert!(reopened
                .invoke(api.operation(), || {
                    panic!("reopened guard must not construct public submission");
                    #[allow(unreachable_code)]
                    api.submit(&client)
                })
                .await
                .is_err());
            assert_eq!(
                std::fs::read(temp.0.join("replay-unresolved.json")).unwrap(),
                bytes
            );
            for _ in api.request_lines() {
                assert!(matches!(server.event().await, Event::Request(_)));
            }
            assert!(matches!(server.event().await, Event::QuietObserved));
            api.assert_requests(&server.finish());
        });
    }
}

#[test]
#[cfg(unix)]
fn replay_guard_owner_drop_after_complete_public_post_blocks_reopen() {
    use crate::replay_guard::ReplayGuard;
    for api in GUARDED_APIS {
        run(async {
            let temp = ReplayTemp::new();
            let mut guard = ReplayGuard::open(&temp.0).unwrap();
            let mut replies = api.prefix();
            replies.extend([Reply::Hold, Reply::ObserveQuiet(SUBMISSION_QUIET_WINDOW)]);
            let mut server = Server::new(replies);
            let mut client = fixture_client(server.address);
            client.config = logging_config();
            let mut pending = Box::pin(guard.invoke(api.operation(), || api.submit(&client)));
            let mut observed = Vec::new();
            for expected in api.request_lines() {
                match futures_util::future::select(Box::pin(server.event()), pending).await {
                    futures_util::future::Either::Left((
                        Event::Request(request),
                        still_pending,
                    )) => {
                        assert_eq!(request.lines().next().unwrap(), expected);
                        // Independent evidence: fully parseable reservation BEFORE
                        // even the first HTTP request has been processed.
                        let entry: serde_json::Value = serde_json::from_slice(
                            &std::fs::read(temp.0.join("replay-unresolved.json")).unwrap(),
                        )
                        .unwrap();
                        assert_eq!(entry["version"], 1);
                        observed.push(request);
                        pending = still_pending;
                    }
                    _ => panic!("guarded public owner completed before cancellation"),
                }
            }
            api.assert_requests(&observed);
            let bytes = std::fs::read(temp.0.join("replay-unresolved.json")).unwrap();
            drop(pending); // owns the future AND its transfer stream, not next().
            drop(guard);
            assert!(matches!(
                tokio::time::timeout(Duration::from_millis(500), server.event())
                    .await
                    .unwrap(),
                Event::Disconnected
            ));
            let mut reopened = ReplayGuard::open(&temp.0).unwrap();
            assert!(reopened
                .invoke(api.operation(), || {
                    panic!("drop/reopen must not issue another public request");
                    #[allow(unreachable_code)]
                    api.submit(&client)
                })
                .await
                .is_err());
            assert_eq!(
                std::fs::read(temp.0.join("replay-unresolved.json")).unwrap(),
                bytes
            );
            assert!(matches!(server.event().await, Event::QuietObserved));
            api.assert_requests(&server.finish());
        });
    }
}

// Isolated child; the parent fixture verifies complete final POST then SIGKILLs
// it. No Drop, result handling or journal cleanup is allowed to run in the child.
#[test]
#[ignore]
#[cfg(unix)]
fn replay_guard_http_crash_child() {
    use crate::replay_guard::ReplayGuard;
    let path = std::env::var_os("BOURSO_REPLAY_HTTP_PATH").unwrap();
    let address = std::env::var("BOURSO_REPLAY_HTTP_ADDRESS")
        .unwrap()
        .parse()
        .unwrap();
    let index: usize = std::env::var("BOURSO_REPLAY_HTTP_API")
        .unwrap()
        .parse()
        .unwrap();
    run(async {
        let mut guard = ReplayGuard::open(std::path::PathBuf::from(path)).unwrap();
        let mut client = fixture_client(address);
        client.config = logging_config();
        let api = GUARDED_APIS[index];
        let _ = guard.invoke(api.operation(), || api.submit(&client)).await;
        panic!("held submission must be killed before returning");
    });
}
#[cfg(unix)]
struct ReplayChild(std::process::Child);
#[cfg(unix)]
impl Drop for ReplayChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
#[cfg(unix)]
fn replay_guard_process_crash_after_complete_public_post_blocks_reopen() {
    use crate::replay_guard::{GuardError, ReplayGuard};
    for (index, api) in GUARDED_APIS.into_iter().enumerate() {
        run(async {
            let temp = ReplayTemp::new();
            let mut replies = api.prefix();
            replies.extend([Reply::Hold, Reply::ObserveQuiet(SUBMISSION_QUIET_WINDOW)]);
            let mut server = Server::new(replies);
            let mut child = ReplayChild(
                std::process::Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "client::http_tests::replay_guard_http_crash_child",
                        "--ignored",
                    ])
                    .env("BOURSO_REPLAY_HTTP_PATH", &temp.0)
                    .env("BOURSO_REPLAY_HTTP_ADDRESS", server.address.to_string())
                    .env("BOURSO_REPLAY_HTTP_API", index.to_string())
                    .stdout(std::process::Stdio::null())
                    .spawn()
                    .unwrap(),
            );
            let mut observed = Vec::new();
            for _ in api.request_lines() {
                match server.event().await {
                    Event::Request(request) => observed.push(request),
                    _ => panic!("expected complete public request before crash"),
                }
            }
            api.assert_requests(&observed);
            assert!(matches!(
                ReplayGuard::open(&temp.0),
                Err(GuardError::Locked)
            ));
            let bytes = std::fs::read(temp.0.join("replay-unresolved.json")).unwrap();
            child.0.kill().unwrap();
            assert!(!child.0.wait().unwrap().success());
            assert!(matches!(server.event().await, Event::Disconnected));
            let mut reopened = ReplayGuard::open(&temp.0).unwrap();
            let client = fixture_client(server.address);
            assert!(reopened
                .invoke(api.operation(), || {
                    panic!("crash/reopen must not issue another public request");
                    #[allow(unreachable_code)]
                    api.submit(&client)
                })
                .await
                .is_err());
            assert_eq!(
                std::fs::read(temp.0.join("replay-unresolved.json")).unwrap(),
                bytes
            );
            assert!(matches!(server.event().await, Event::QuietObserved));
            api.assert_requests(&server.finish());
        });
    }
}

#[test]
#[cfg(unix)]
fn replay_guard_nominal_public_outputs_remain_original_and_unresolved() {
    use super::{
        trade::order::{OrderConfirmResponse, OrderSide, OrderSubmissionStatus},
        transfer::TransferProgress,
    };
    use crate::replay_guard::ReplayGuard;
    use futures_util::StreamExt;
    for api in GUARDED_APIS {
        run(async {
            let temp = ReplayTemp::new();
            let mut guard = ReplayGuard::open(&temp.0).unwrap();
            let mut replies = api.prefix();
            let confirm = OrderConfirmResponse {
                order_id: "SENTINEL_USER".into(),
                ..Default::default()
            };
            replies.push(match api.operation() {
                crate::OutcomeOperation::Order => {
                    response("201 Created", "", &serde_json::to_string(&confirm).unwrap())
                }
                crate::OutcomeOperation::Transfer => ok("Confirmation"),
            });
            replies.push(Reply::ObserveQuiet(SUBMISSION_QUIET_WINDOW));
            let mut server = Server::new(replies);
            let mut client = fixture_client(server.address);
            client.config = logging_config();
            let account = crate::account::Account {
                id: "SENTINEL_USER".into(),
                ..Default::default()
            };
            match api {
                PublicSubmission::Order => {
                    let original = guard
                        .invoke(api.operation(), || {
                            client.order(OrderSide::Buy, &account, "TEST", 2, None)
                        })
                        .await
                        .unwrap()
                        .unwrap();
                    assert_eq!(original.0, "SENTINEL_USER");
                }
                PublicSubmission::OrderDetailed => {
                    let original = guard
                        .invoke(api.operation(), || {
                            client.order_detailed(OrderSide::Buy, &account, "TEST", 2, None)
                        })
                        .await
                        .unwrap()
                        .unwrap();
                    assert_eq!(original.order_id, "SENTINEL_USER");
                    assert_eq!(original.status, OrderSubmissionStatus::SubmittedUnverified);
                }
                PublicSubmission::TransferFunds | PublicSubmission::TransferMoney => {
                    let original: Vec<_> = guard
                        .invoke(api.operation(), || async {
                            match api {
                                PublicSubmission::TransferFunds => {
                                    client
                                        .transfer_funds(
                                            12.0,
                                            account.clone(),
                                            account,
                                            Some("SENTINEL_BODY".into()),
                                        )
                                        .collect()
                                        .await
                                }
                                PublicSubmission::TransferMoney => {
                                    client
                                        .transfer_money(
                                            "12.00".parse().unwrap(),
                                            account.clone(),
                                            account,
                                            Some("SENTINEL_BODY".into()),
                                        )
                                        .collect()
                                        .await
                                }
                                _ => unreachable!(),
                            }
                        })
                        .await
                        .unwrap();
                    assert!(original.iter().all(Result::is_ok));
                    assert!(matches!(
                        original.last().unwrap(),
                        Ok(TransferProgress::SubmittedUnverified)
                    ));
                }
            }
            assert!(guard.unresolved().unwrap().is_some());
            drop(guard);
            let mut reopened = ReplayGuard::open(&temp.0).unwrap();
            assert!(reopened
                .invoke(api.operation(), || async {
                    panic!("nominal output must not release replay barrier")
                })
                .await
                .is_err());
            for _ in api.request_lines() {
                assert!(matches!(server.event().await, Event::Request(_)));
            }
            assert!(matches!(server.event().await, Event::QuietObserved));
            api.assert_requests(&server.finish());
        });
    }
}
