use super::*;
use std::{
    net::TcpListener,
    panic::{catch_unwind, AssertUnwindSafe},
    time::Duration,
};

pub(super) fn assert_safe_error<T>(result: Result<T>, stage: &str) {
    let error = match result {
        Ok(_) => panic!("expected an authentication error"),
        Err(error) => error,
    };
    let diagnostic = format!("{error:#} {error:?}");
    assert!(
        diagnostic.contains(stage),
        "missing error context: {diagnostic}"
    );
    assert!(!diagnostic.contains("PRIVATE_RESPONSE"));
}

#[test]
fn absent_config_returns_error_without_panic() {
    for page in ["", "<script>PRIVATE_RESPONSE", "window.BRS_CONFIG ="] {
        let result = catch_unwind(|| extract_brs_config(page));
        assert!(result.is_ok(), "missing config must not panic");
        assert_safe_error(result.unwrap(), "BRS_CONFIG");
    }
}

#[test]
fn absent_challenge_returns_error_without_panic() {
    for page in ["", "<html>PRIVATE_RESPONSE", "data-matrix-random-challenge"] {
        let result = catch_unwind(|| virtual_pad::extract_challenge_token(page));
        assert!(result.is_ok(), "missing challenge must not panic");
        assert_safe_error(result.unwrap(), "challenge");
    }
}

#[test]
fn missing_password_key_returns_error_without_panic() {
    let result = catch_unwind(|| virtual_pad::password_to_virtual_pad_keys(vec![], "1"));
    assert!(result.is_ok(), "uninitialized keyboard must not panic");
    assert_safe_error(result.unwrap(), "virtual pad");
}

#[test]
fn login_before_init_returns_state_error_without_sending() {
    // An unserved local listener proves even an empty password cannot bypass the guard.
    for password in ["12345678", ""] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut client =
            BoursoWebClient::for_test(listener.local_addr().unwrap(), Duration::from_millis(100))
                .unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let result = catch_unwind(AssertUnwindSafe(|| {
            runtime.block_on(client.login("PRIVATE_RESPONSE", password))
        }));
        assert!(result.is_ok(), "login before init must not panic");
        assert_safe_error(result.unwrap(), "session");
        listener.set_nonblocking(true).unwrap();
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
    }
}

#[test]
fn malformed_mfa_success_returns_safe_error_without_panic() {
    for body in [
        "{}",
        "null",
        "[]",
        r#"{"success":null}"#,
        r#"{"success":"PRIVATE_RESPONSE"}"#,
        r#"{"success":1}"#,
        r#"{"success":{}}"#,
    ] {
        let body = serde_json::from_str(body).unwrap();
        let result = catch_unwind(|| mfa_success(&body));
        assert!(result.is_ok());
        assert_safe_error(result.unwrap(), "MFA response");
    }
    assert!(mfa_success(&serde_json::json!({"success":true})).unwrap());
    assert!(!mfa_success(&serde_json::json!({"success":false})).unwrap());
}

#[test]
fn optional_user_hash_is_valid_before_authentication_only() {
    for hash in [None, Some("".to_owned()), Some("  ".to_owned())] {
        let config = Config {
            user_hash: hash,
            ..Default::default()
        };
        let page = format!(
            "window.BRS_CONFIG = {};",
            serde_json::to_string(&config).unwrap()
        );
        let parsed = extract_brs_config(&page).unwrap();
        let result = catch_unwind(|| parsed.require_user_hash());
        assert!(result.is_ok());
        assert_safe_error(result.unwrap(), "USER_HASH");
    }
    let config = Config {
        user_hash: Some("fixture".into()),
        ..Default::default()
    };
    assert_eq!(config.require_user_hash().unwrap(), "fixture");
}

#[test]
fn malformed_config_and_otp_diagnostics_are_safe() {
    for page in [
        "window.BRS_CONFIG = {;",
        r#"window.BRS_CONFIG = {"API_HOST": "PRIVATE_RESPONSE"};"#,
        r#"window.BRS_CONFIG = {"API_HOST": ["PRIVATE_RESPONSE"]};"#,
    ] {
        let result = catch_unwind(|| extract_brs_config(page));
        assert!(result.is_ok());
        assert_safe_error(result.unwrap(), "BRS_CONFIG");
    }
    for page in [
        "PRIVATE_RESPONSE",
        r#"data-strong-authentication-payload="{}">"#,
        r#"data-strong-authentication-payload="{&quot;challenges&quot;:null}">"#,
    ] {
        let result = catch_unwind(|| extract_otp_params(page));
        assert!(result.is_ok());
        assert!(result.unwrap().is_err());
    }
}

// All integration requests use the existing loopback-only facade. The server has
// a deadline and records methods/paths only, never credentials or response data.
pub(super) fn with_responses<T>(
    responses: Vec<(u16, String)>,
    action: impl FnOnce(&mut BoursoWebClient) -> T,
) -> (T, Vec<String>) {
    with_checked_responses(responses, action, |_, _, _| {})
}

// Inspect fixture traffic in memory without retaining request headers or bodies.
fn with_checked_responses<T>(
    responses: Vec<(u16, String)>,
    action: impl FnOnce(&mut BoursoWebClient) -> T,
    inspect: impl Fn(usize, &str, &str) + Send + 'static,
) -> (T, Vec<String>) {
    use std::{
        io::{Read, Write},
        thread,
        time::Instant,
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let mut client =
        BoursoWebClient::for_test(listener.local_addr().unwrap(), Duration::from_secs(1)).unwrap();
    let server = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut requests = vec![];
        for (status, body) in responses {
            let mut socket = loop {
                match listener.accept() {
                    Ok((socket, _)) => break socket,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "missing local fixture request");
                        thread::sleep(Duration::from_millis(1));
                    }
                    Err(error) => panic!("local fixture accept: {error}"),
                }
            };
            socket
                .set_read_timeout(Some(Duration::from_secs(1)))
                .unwrap();
            let mut request = Vec::new();
            loop {
                let mut buffer = [0; 4096];
                let n = socket.read(&mut buffer).unwrap();
                assert!(n > 0, "truncated fixture request");
                request.extend_from_slice(&buffer[..n]);
                if let Some(end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&request[..end]);
                    let length = headers
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if request.len() >= end + 4 + length {
                        inspect(
                            requests.len(),
                            &headers,
                            &String::from_utf8_lossy(&request[end + 4..end + 4 + length]),
                        );
                        requests.push(headers.lines().next().unwrap().to_string());
                        break;
                    }
                }
            }
            write!(socket, "HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        }
        (requests, listener)
    });
    let result = action(&mut client);
    let (requests, listener) = server.join().unwrap();
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock,
        "unexpected request after scripted responses"
    );
    (result, requests)
}

pub(super) fn run<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}

pub(super) fn config_page(user_hash: Option<&str>) -> String {
    let config = Config {
        api_url: "https://api.boursobank.com/fixture".into(),
        user_hash: user_hash.map(str::to_owned),
        ..Default::default()
    };
    format!(
        "window.BRS_CONFIG = {};",
        serde_json::to_string(&config).unwrap()
    )
}

fn mfa_page(user_hash: Option<&str>) -> String {
    format!(
        r#"{} brs-otp-webtoapp <input name="form[_token]" value="fixture-token" > data-strong-authentication-payload="{{&quot;challenges&quot;:[{{&quot;parameters&quot;:{{&quot;formScreen&quot;:{{&quot;actions&quot;:{{&quot;check&quot;:{{&quot;api&quot;:{{&quot;params&quot;:{{&quot;resourceId&quot;:&quot;fixture-otp&quot;,&quot;formState&quot;:&quot;fixture-state&quot;}}}}}}}}}}}}}}]}}">"#,
        config_page(user_hash)
    )
}

#[test]
fn login_null_user_hash_returns_error_regardless_of_logging() {
    for enabled in [true, false] {
        let (_, logs) = super::logging_tests::capture(enabled, || {
            let (result, requests) = with_responses(
                vec![
                    (302, String::new()),
                    (
                        200,
                        format!(r#"href="/se-deconnecter" {}"#, config_page(None)),
                    ),
                ],
                |client| {
                    client.virtual_pad_ids = super::newpad_tests::entries()
                        .into_iter()
                        .map(|entry| entry.0)
                        .collect();
                    catch_unwind(AssertUnwindSafe(|| {
                        run(client.login("PRIVATE_RESPONSE", "12345678"))
                    }))
                },
            );
            assert!(result.is_ok());
            assert_safe_error(result.unwrap(), "USER_HASH");
            assert_eq!(requests.len(), 2);
        });
        assert!(!logs.contains("PRIVATE_RESPONSE"));
        assert!(!logs.contains("now logged in"));
    }
}

#[test]
fn mfa_public_paths_reject_malformed_success() {
    for body in ["{}", r#"{"success":"PRIVATE_RESPONSE"}"#] {
        let (result, requests) = with_responses(
            vec![
                (200, String::new()),
                (200, mfa_page(Some("fixture"))),
                (200, body.into()),
            ],
            |client| catch_unwind(AssertUnwindSafe(|| run(client.request_mfa()))),
        );
        assert!(result.is_ok());
        assert_safe_error(result.unwrap(), "MFA response");
        assert_eq!(requests.len(), 3);
        let (result, requests) = with_responses(vec![(200, body.into())], |client| {
            client.config.api_url = "https://api.boursobank.com/fixture".into();
            client.config.user_hash = Some("fixture".into());
            catch_unwind(AssertUnwindSafe(|| {
                run(client.check_mfa(
                    MfaType::WebToApp,
                    "otp".into(),
                    "state".into(),
                    "token".into(),
                ))
            }))
        });
        assert!(result.is_ok());
        assert_safe_error(result.unwrap(), "MFA response");
        assert_eq!(requests.len(), 1);
    }
}

#[test]
fn mfa_null_user_hash_is_rejected_before_challenge_post() {
    let (result, requests) = with_responses(
        vec![(200, String::new()), (200, mfa_page(None))],
        |client| catch_unwind(AssertUnwindSafe(|| run(client.request_mfa()))),
    );
    assert!(result.is_ok());
    assert_safe_error(result.unwrap(), "USER_HASH");
    assert_eq!(requests.len(), 2);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let mut client =
        BoursoWebClient::for_test(listener.local_addr().unwrap(), Duration::from_millis(100))
            .unwrap();
    let result = catch_unwind(AssertUnwindSafe(|| {
        run(client.check_mfa(
            MfaType::WebToApp,
            "otp".into(),
            "state".into(),
            "token".into(),
        ))
    }));
    assert!(result.is_ok());
    assert_safe_error(result.unwrap(), "USER_HASH");
    listener.set_nonblocking(true).unwrap();
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}

#[test]
fn failed_reinitialization_clears_login_readiness() {
    let (result, requests) = with_responses(vec![(200, "PRIVATE_RESPONSE".into())], |client| {
        client.virtual_pad_ids = super::newpad_tests::entries()
            .into_iter()
            .map(|entry| entry.0)
            .collect();
        assert!(run(client.init_session()).is_err());
        run(client.login("fixture", ""))
    });
    assert_safe_error(result, "session");
    assert_eq!(requests.len(), 1);
}

#[test]
fn mfa_completion_requires_authenticated_user_hash() {
    for user_hash in [None, Some("fixture")] {
        let (result, requests) = with_responses(
            vec![
                (200, r#"{"success":true}"#.into()),
                (302, String::new()),
                (
                    200,
                    format!(r#"href="/se-deconnecter" {}"#, config_page(user_hash)),
                ),
            ],
            |client| {
                client.config.api_url = "https://api.boursobank.com/fixture".into();
                client.config.user_hash = Some("fixture".into());
                catch_unwind(AssertUnwindSafe(|| {
                    run(client.check_mfa(
                        MfaType::WebToApp,
                        "otp".into(),
                        "state".into(),
                        "token".into(),
                    ))
                }))
            },
        );
        assert!(result.is_ok());
        if user_hash.is_none() {
            assert_safe_error(result.unwrap(), "USER_HASH");
        } else {
            assert!(result.unwrap().unwrap());
        }
        assert_eq!(requests.len(), 3);
    }
}

#[test]
fn malformed_initialization_returns_errors_without_panics() {
    for responses in [
        vec![
            (200, "__brs_mit=fixture;".into()),
            (
                200,
                r#"<input name="form[_token]" value="fixture" > PRIVATE_RESPONSE"#.into(),
            ),
        ],
        vec![
            (200, "__brs_mit=fixture;".into()),
            (
                200,
                format!(
                    r#"<input name="form[_token]" value="fixture" > {}"#,
                    config_page(None)
                ),
            ),
            (200, "PRIVATE_RESPONSE".into()),
        ],
    ] {
        let count = responses.len();
        let (result, requests) = with_responses(responses, |client| {
            catch_unwind(AssertUnwindSafe(|| run(client.init_session())))
        });
        assert!(result.is_ok());
        assert_safe_error(
            result.unwrap(),
            if count == 2 {
                "BRS_CONFIG"
            } else {
                "challenge"
            },
        );
        assert_eq!(requests.len(), count);
    }
}

#[test]
fn init_session_then_login_succeeds_with_complete_local_keyboard() {
    // SVGs copied from virtual_pad::tests::VIRTUAL_PAD_RES; all IDs, tokens,
    // cookies and credentials below are synthetic. No client state is injected.
    let keys = [
        "AAA", "AAB", "AAC", "AAD", "AAE", "AAF", "AAG", "AAH", "AAI", "AAJ",
    ];
    let entries = super::newpad_tests::entries();
    let shuffled = [9, 0, 6, 2, 3, 8, 4, 1, 5, 7].map(|digit| entries[digit].clone());
    let keyboard = super::newpad_tests::keyboard(&shuffled);
    let (result, requests) = with_checked_responses(
        vec![
            (200, "__brs_mit=fixture-cookie;".into()),
            (
                200,
                format!(
                    r#"<input name="form[_token]" value="fixture-form-token" > {}"#,
                    config_page(None)
                ),
            ),
            (200, keyboard),
            (302, String::new()),
            (
                200,
                format!(
                    r#"href="/se-deconnecter" {}"#,
                    config_page(Some("fixture-user-hash"))
                ),
            ),
        ],
        |client| {
            catch_unwind(AssertUnwindSafe(|| {
                run(async {
                    client.init_session().await?;
                    assert!(client.config.user_hash.is_none());
                    assert_eq!(client.virtual_pad_ids, keys);
                    assert_eq!(client.token, "fixture-form-token");
                    assert_eq!(client.challenge_id, "fixture-challenge");
                    client.login("fixture-customer", "9072163458").await?;
                    assert_eq!(client.config.require_user_hash()?, "fixture-user-hash");
                    Ok::<(), anyhow::Error>(())
                })
            }))
        },
        |index, headers, body| {
            if index > 0 {
                let cookie = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("cookie").then_some(value)
                    })
                    .expect("session cookies must accompany subsequent requests");
                let cookies: Vec<_> = cookie.split(';').map(str::trim).collect();
                assert!(cookies.contains(&"__brs_mit=fixture-cookie"));
                assert!(cookies.contains(&"brsDomainMigration=migrated"));
            }
            if index == 3 {
                for (name, value) in [
                    ("form[_token]", "fixture-form-token"),
                    ("form[matrixRandomChallenge]", "fixture-challenge"),
                    ("form[clientNumber]", "fixture-customer"),
                    ("form[password]", "AAJ|AAA|AAH|AAC|AAB|AAG|AAD|AAE|AAF|AAI"),
                ] {
                    assert!(
                        body.contains(&format!("name=\"{name}\"\r\n\r\n{value}\r\n")),
                        "login multipart field missing or incorrect: {name}"
                    );
                }
            }
        },
    );
    assert!(result.is_ok(), "composed authentication must not panic");
    result
        .unwrap()
        .expect("complete local keyboard must allow login");
    assert_eq!(
        requests,
        [
            "GET /connexion/ HTTP/1.1",
            "GET /connexion/ HTTP/1.1",
            "GET /connexion/clavier-virtuel?_hinclude=1 HTTP/1.1",
            "POST /connexion/saisie-mot-de-passe HTTP/1.1",
            "GET / HTTP/1.1",
        ]
    );
}
