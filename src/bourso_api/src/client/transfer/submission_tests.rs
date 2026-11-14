//! Synthetic local transport tests, not assertions about bank business contracts.
use super::*;
use futures_util::StreamExt;
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

fn fixture(final_body: &'static str) -> (std::net::SocketAddr, thread::JoinHandle<Vec<String>>) {
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
            ("200 OK", "", final_body),
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
            // Windows accepted sockets inherit nonblocking mode; restore blocking I/O.
            stream.set_nonblocking(false).unwrap();
            requests.push(read_request(&mut stream));
            stream.set_write_timeout(Some(BOUND)).unwrap();
            write!(stream, "HTTP/1.1 {status}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        }
        requests
    });
    (address, worker)
}

#[test]
fn final_http_200_never_claims_execution_for_either_public_stream() {
    for body in [
        "Confirmation",
        "<h1>Confirmation</h1><p>Virement refusé</p>",
        "<form>Connexion requise</form>",
        "synthetic-private-response-body",
        "",
    ] {
        for legacy in [false, true] {
            let (address, worker) = fixture(body);
            run(async {
                let client = BoursoWebClient::for_test(address, BOUND).unwrap();
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
                        Box::pin(client.transfer_funds(10.50, from, to, None))
                    } else {
                        Box::pin(client.transfer_money("10.50".parse().unwrap(), from, to, None))
                    };
                let results: Vec<_> = stream.collect().await;
                // Join before assertions so the mock worker never escapes a failure.
                let requests = worker.join().unwrap();
                assert_eq!(requests.len(), 8);
                for (request, step) in requests.iter().skip(2).zip([2, 3, 4, 5, 8, 10]) {
                    assert!(request.starts_with(&format!(
                        "POST /compte/cav/source/virements/immediat/nouveau/fixture/{step} "
                    )));
                }
                assert_eq!(
                    requests.iter().filter(|r| r.starts_with("POST ")).count(),
                    6
                );
                // Preserve the real payload contract while changing only its outcome label.
                assert!(requests[4].contains("name=\"Amount[amount]\"\r\n\r\n10,50\r\n"));
                assert_eq!(results.len(), 10);
                for (index, progress) in results.iter().enumerate() {
                    let progress = progress
                        .as_ref()
                        .expect("HTTP 200 submission remains available");
                    assert_eq!(progress.step_number() as usize, index + 1);
                    assert!(progress.step_number() <= TransferProgress::total_steps());
                }
                let last = results.last().unwrap().as_ref().unwrap();
                assert_eq!(
                    last.description(),
                    "Submission response received; execution unverified"
                );
                assert!(matches!(last, TransferProgress::SubmittedUnverified));
                assert_eq!(format!("{last:?}"), "SubmittedUnverified");
                let warning = last.submission_warning().unwrap();
                assert!(warning.contains("submitted-unverified"));
                assert!(warning.contains("execution unverified"));
                assert!(warning.contains("Verify in the bank before retrying"));
                assert!(!warning.contains("successful"));
                assert!(!warning.contains("synthetic-private-response-body"));
            });
        }
    }
}

#[test]
#[allow(deprecated)]
fn progress_metadata_retains_compatibility_without_success_claims() {
    for progress in [
        TransferProgress::Completed,
        TransferProgress::SubmittedUnverified,
    ] {
        assert_eq!(progress.step_number(), TransferProgress::total_steps());
        assert_eq!(
            progress.description(),
            "Submission response received; execution unverified"
        );
        assert!(progress
            .submission_warning()
            .unwrap()
            .contains("execution unverified"));
    }
    for progress in [
        TransferProgress::Validating,
        TransferProgress::InitializingTransfer,
        TransferProgress::ExtractingFlowInstance,
        TransferProgress::SettingDebitAccount,
        TransferProgress::SettingCreditAccount,
        TransferProgress::SettingAmount,
        TransferProgress::SubmittingStep5,
        TransferProgress::SettingReason,
        TransferProgress::ConfirmingTransfer,
    ] {
        assert!(progress.submission_warning().is_none());
        assert!(progress.step_number() < TransferProgress::total_steps());
    }
}
