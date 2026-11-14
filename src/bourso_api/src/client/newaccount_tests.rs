use super::{tests::ACCOUNTS_RES, BoursoWebClient};
use crate::account::{Account, AccountKind};
use std::{
    io::{Read, Write},
    net::TcpListener,
    thread,
    time::{Duration, Instant},
};

// One bounded, loopback-only request through the public account API.
fn fetch(status: &str, body: &str, kind: Option<AccountKind>) -> anyhow::Result<Vec<Account>> {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let reply = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let worker = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, peer)) => {
                    assert!(peer.ip().is_loopback());
                    break stream;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline, "fixture accept deadline");
                    thread::sleep(Duration::from_millis(2));
                }
                Err(_) => panic!("fixture accept failed"),
            }
        };
        // Windows accepted sockets inherit nonblocking mode; restore blocking I/O.
        stream.set_nonblocking(false).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut request = Vec::new();
        let mut byte = [0];
        while !request.ends_with(b"\r\n\r\n") {
            assert!(request.len() < 16384);
            stream.read_exact(&mut byte).unwrap();
            request.push(byte[0]);
        }
        assert!(request.starts_with(b"GET /dashboard/liste-comptes?"));
        stream.write_all(reply.as_bytes()).unwrap();
    });
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let result = runtime.block_on(async {
        BoursoWebClient::for_test(address, Duration::from_secs(3))
            .unwrap()
            .get_accounts(kind)
            .await
    });
    worker.join().unwrap();
    result
}

#[test]
fn login_page_is_not_an_empty_portfolio() {
    let result = fetch(
        "200 OK",
        "<html><form action='/connexion'><input type='password'></form>private-body</html>",
        None,
    );
    assert!(
        result.is_err(),
        "login HTML must not become a successful empty list"
    );
}

#[test]
fn http_failure_is_not_a_successful_inventory() {
    assert!(fetch("500 Internal Server Error", ACCOUNTS_RES, None).is_err());
}

#[test]
fn malformed_category_is_not_silently_dropped() {
    let html = ACCOUNTS_RES.replace("11 010,00", "private-invalid-value");
    for kind in [None, Some(AccountKind::Banking), Some(AccountKind::Savings)] {
        let result = fetch("200 OK", &html, kind);
        assert!(result.is_err());
        let diagnostic = format!("{:#}", result.err().unwrap());
        assert!(!diagnostic.contains("private-invalid-value"));
        assert!(!diagnostic.contains("a8a23172b7e7c91c538831578242112e"));
    }
}

#[test]
fn complete_fixture_retains_all_accounts_and_cents() {
    let accounts = fetch("200 OK", ACCOUNTS_RES, None).unwrap();
    assert_eq!(accounts.len(), 6);
    assert_eq!(
        accounts.iter().map(|a| a.balance).collect::<Vec<_>>(),
        vec![1101000, 159972, 2081050, 50040, 14308889, -9495982]
    );
}

#[test]
fn known_empty_section_allows_empty_inventory_and_absent_category() {
    // Synthetic contract fixture, not evidence of a real bank empty portfolio.
    let html = "<ul data-summary-bank> \n </ul>";
    assert!(fetch("200 OK", html, None).unwrap().is_empty());
    assert!(fetch("200 OK", html, Some(AccountKind::Savings))
        .unwrap()
        .is_empty());
}

#[test]
fn missing_category_with_other_valid_accounts_is_legitimate() {
    let html = ACCOUNTS_RES
        .split("<div class=\"c-panel c-panel--primary")
        .take(2)
        .collect::<Vec<_>>()
        .join("<div class=\"c-panel c-panel--primary");
    let accounts = fetch("200 OK", &html, None).unwrap();
    assert_eq!(accounts.len(), 2);
    assert!(accounts.iter().all(|a| a.kind == AccountKind::Banking));
    assert!(fetch("200 OK", &html, Some(AccountKind::Loans))
        .unwrap()
        .is_empty());
}

#[test]
fn unrecognized_or_malformed_sections_fail_closed() {
    for html in [
        "",
        "<html><main>Dashboard: no accounts</main></html>",
        "<ul data-summary-unknown></ul>",
        "<ul data-summary-bank><li>private-broken-account</li></ul>",
        "<ul data-summary-bank><li>private-broken-account</ul>",
        "<ul data-summary-bank>",
        "<ul data-summary-bank><div>private-unexpected-content</div></ul>",
        "<div data-summary-bank></div>",
        "<ul data-summary-bank></ul><div data-summary-savings></div>",
        "<ul data-summary-bank></ul><ul data-summary-savings><li></ul>",
        "<ul title='data-summary-bank'></ul>",
        "<!-- <ul data-summary-bank></ul> -->",
        "<ul data-summary-bank data-summary-loan></ul>",
        "<ul data-summary-bank></ul><ul data-summary-bank></ul>",
        "<input type='password'><ul data-summary-bank></ul>",
    ] {
        for kind in [None, Some(AccountKind::Savings)] {
            let error = fetch("200 OK", html, kind).unwrap_err();
            let diagnostic = format!("{error:#}");
            assert!(!diagnostic.contains("private-"));
            assert!(!diagnostic.contains("<ul"));
        }
    }
}

#[test]
fn malformed_individual_accounts_do_not_hide_behind_valid_siblings() {
    for (from, to) in [
        (
            "/compte/epargne/ldd/a8a23172b7e7c91c538831578242112e/",
            "/compte/epargne/ldd/private-invalid-id/",
        ),
        (
            "c-info-box__account-sub-label",
            "private-missing-bank-label",
        ),
        ("Solde : 11 010,00 €", "private-missing-balance"),
    ] {
        let html = ACCOUNTS_RES.replacen(from, to, 1);
        let result = fetch("200 OK", &html, None);
        assert!(result.is_err(), "malformed account must fail");
        let error = result.err().unwrap();
        let diagnostic = format!("{error:#}");
        assert!(!diagnostic.contains("private-"));
        assert!(!diagnostic.contains("a8a23172b7e7c91c538831578242112e"));
    }
}

#[test]
fn http_status_failures_have_safe_diagnostics() {
    for status in [
        "302 Found",
        "401 Unauthorized",
        "403 Forbidden",
        "500 Internal Server Error",
    ] {
        let result = fetch(status, "private-body-and-account-id", None);
        assert!(result.is_err());
        let diagnostic = format!("{:#}", result.err().unwrap());
        assert!(diagnostic.contains(&status[..3]));
        assert!(!diagnostic.contains("private-"));
        assert!(!diagnostic.contains("http://"));
    }
}

#[test]
fn empty_known_sections_accept_attribute_and_nested_product_variants() {
    for html in [
        "<ul data-summary-bank=\"\" role=\"list\"></ul>",
        "<ul role='list' data-summary-savings ><!-- no rows --></ul>",
    ] {
        assert!(fetch("200 OK", html, None).unwrap().is_empty());
    }
    // Banking's first account already contains a nested product ul/li in the
    // successful source fixture; moving the summary attribute must not alter it.
    let html = ACCOUNTS_RES.replace("data-summary-bank>", "data-summary-bank role='list'>");
    assert_eq!(fetch("200 OK", &html, None).unwrap().len(), 6);
}
