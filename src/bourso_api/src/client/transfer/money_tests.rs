//! Synthetic local transport tests, not assertions about bank business contracts.
use super::*;
use futures_util::{pin_mut, StreamExt};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    thread,
    time::{Duration, Instant},
};

const BOUND: Duration = Duration::from_secs(3);

fn run(future: impl std::future::Future<Output = ()>) {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future);
}

#[test]
fn invalid_float_never_contacts_local_listener() {
    run(async {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let client = BoursoWebClient::for_test(listener.local_addr().unwrap(), BOUND).unwrap();
        for amount in [
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
            -10.0,
            -0.0,
            0.0,
            9.99,
            10.001,
            10.999,
            f64::MAX,
            f64::from_bits(10.0f64.to_bits() + 1),
        ] {
            let stream =
                client.transfer_funds(amount, Account::default(), Account::default(), None);
            pin_mut!(stream);
            assert!(matches!(
                stream.next().await.unwrap().unwrap(),
                TransferProgress::Validating
            ));
            let result = stream.next().await.unwrap();
            assert!(
                result.is_err(),
                "invalid amount advanced past validation: {amount}"
            );
            assert!(stream.next().await.is_none());
            // Bound listener is the only possible endpoint; no accepted TCP
            // connection means not even initialization, let alone a POST, occurred.
            assert_eq!(
                listener.accept().unwrap_err().kind(),
                std::io::ErrorKind::WouldBlock
            );
        }
    });
}

#[test]
fn decimal_policy_and_boundaries_are_exact() {
    for (input, cents, display, bank) in [
        ("10", 1000, "10.00", "10,00"),
        ("10.0", 1000, "10.00", "10,00"),
        ("10,50", 1050, "10.50", "10,50"),
        ("10.01", 1001, "10.01", "10,01"),
        ("00010,5", 1050, "10.50", "10,50"),
        (
            "184467440737095516.15",
            u64::MAX,
            "184467440737095516.15",
            "184467440737095516,15",
        ),
    ] {
        let amount: Money = input.parse().unwrap();
        assert_eq!(amount.cents(), cents);
        assert_eq!(amount.to_string(), display);
        assert_eq!(amount.bank_amount(), bank);
    }
    for input in [
        "",
        "abc",
        "NaN",
        "inf",
        "-10",
        "+10",
        " 10",
        "10 ",
        "9.99",
        "0",
        "10.001",
        "10.000",
        "10,000",
        "1e2",
        "10.",
        ".50",
        "10,",
        "10,1.0",
        "1,000.00",
        "١٠",
        "184467440737095516.16",
        "184467440737095517",
        "999999999999999999999999999",
    ] {
        assert!(input.parse::<Money>().is_err(), "accepted {input:?}");
    }
    for (input, cents) in [
        (10.0, 1000),
        (10.01, 1001),
        (10.50, 1050),
        (1234.56, 123456),
    ] {
        assert_eq!(Money::try_from(input).unwrap().cents(), cents);
    }
}

fn read_request(stream: &mut TcpStream) -> String {
    stream.set_read_timeout(Some(BOUND)).unwrap();
    let deadline = Instant::now() + BOUND;
    let mut bytes = Vec::new();
    loop {
        assert!(Instant::now() < deadline, "fixture read deadline");
        let mut chunk = [0; 4096];
        let count = stream.read(&mut chunk).unwrap();
        assert_ne!(count, 0);
        bytes.extend_from_slice(&chunk[..count]);
        if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            let header = String::from_utf8_lossy(&bytes[..end]);
            let length = header
                .lines()
                .filter_map(|line| line.split_once(':'))
                .find(|(key, _)| key.eq_ignore_ascii_case("content-length"))
                .map(|(_, value)| value.trim().parse::<usize>().unwrap())
                .unwrap_or(0);
            if bytes.len() >= end + 4 + length {
                return String::from_utf8(bytes).unwrap();
            }
        }
    }
}

fn fixture() -> (std::net::SocketAddr, thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let worker = thread::spawn(move || {
        let replies = [
            (
                "302 Found",
                "Location: /compte/cav/source/virements/immediat/nouveau/fixture/1\r\n",
                "",
            ),
            (
                "200 OK",
                "",
                r#"<input name="flow_ImmediateCashTransfer_instance" value="synthetic-flow">"#,
            ),
            ("200 OK", "", ""),
            ("200 OK", "", ""),
            ("200 OK", "", ""),
            ("200 OK", "", ""),
            ("200 OK", "", ""),
            ("200 OK", "", "Confirmation"),
        ];
        let mut requests = Vec::new();
        for (status, headers, body) in replies {
            let deadline = Instant::now() + BOUND;
            let mut stream = loop {
                assert!(Instant::now() < deadline, "fixture accept deadline");
                match listener.accept() {
                    Ok((stream, peer)) => {
                        assert!(peer.ip().is_loopback());
                        break stream;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2))
                    }
                    Err(error) => panic!("fixture accept: {error}"),
                }
            };
            requests.push(read_request(&mut stream));
            stream.set_write_timeout(Some(BOUND)).unwrap();
            write!(stream, "HTTP/1.1 {status}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        }
        requests
    });
    (address, worker)
}

#[test]
fn actual_multipart_payload_preserves_cents_without_rounding() {
    // Test both the compatible float entry and the exact CLI/API entry, including
    // values that cannot be transported losslessly through f64.
    for (input, expected, legacy) in [
        ("10,50", "10,50", true),
        ("10.01", "10,01", true),
        ("10,50", "10,50", false),
        ("184467440737095516.15", "184467440737095516,15", false),
    ] {
        let (address, worker) = fixture();
        let (_, logs) = crate::client::logging_tests::capture(true, || {
            run(async {
                let client = BoursoWebClient::for_test(address, BOUND).unwrap();
                let amount: Money = input.parse().unwrap();
                let from = Account {
                    id: "source".into(),
                    ..Default::default()
                };
                let to = Account {
                    id: "destination".into(),
                    ..Default::default()
                };
                let stream: std::pin::Pin<Box<dyn Stream<Item = Result<TransferProgress>> + '_>> =
                    if legacy {
                        Box::pin(client.transfer_funds(
                            input.replace(',', ".").parse().unwrap(),
                            from,
                            to,
                            None,
                        ))
                    } else {
                        Box::pin(client.transfer_money(amount, from, to, None))
                    };
                pin_mut!(stream);
                let mut completed = false;
                while let Some(progress) = stream.next().await {
                    completed = matches!(progress.unwrap(), TransferProgress::SubmittedUnverified);
                }
                assert!(completed);
                assert_eq!(amount.to_string().replace('.', ","), expected);
            })
        });
        assert!(logs.contains(&format!(
            "Initiating transfer of {} EUR",
            expected.replace(',', ".")
        )));
        let requests = worker.join().unwrap();
        assert_eq!(requests.len(), 8);
        let payload = &requests[4];
        assert!(
            payload.starts_with("POST /compte/cav/source/virements/immediat/nouveau/fixture/4 ")
        );
        assert_eq!(payload.matches("name=\"Amount[amount]\"").count(), 1);
        assert!(
            payload.contains(&format!("name=\"Amount[amount]\"\r\n\r\n{expected}\r\n")),
            "incorrect monetary field"
        );
    }
}
