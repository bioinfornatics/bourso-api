//! Synthetic loopback checks of the existing extraction policy, not a bank contract.
use super::*;
use futures_util::StreamExt;
use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

const BOUND: Duration = Duration::from_secs(3);
const SOURCE: &str = "synthetic-private-account";
const REFERENCE: &str = "synthetic-private-reference";
const FLOW: &str = "synthetic-private-flow";
const BODY: &str = "synthetic-private-body";
const REASON: &str = "synthetic-private-reason";

struct Fixture {
    address: SocketAddr,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<Vec<String>>>,
}

impl Fixture {
    fn new(location: Option<String>, flow_body: String) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let worker = thread::spawn(move || {
            let deadline = Instant::now() + BOUND * 4;
            let mut requests = Vec::new();
            while !worker_stop.load(Ordering::SeqCst) {
                assert!(Instant::now() < deadline, "fixture accept deadline");
                let mut stream = match listener.accept() {
                    Ok((stream, peer)) => {
                        assert!(peer.ip().is_loopback());
                        stream
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(error) => panic!("fixture accept: {error}"),
                };
                requests.push(read_request(&mut stream));
                let (status, headers, body) = match requests.len() {
                    1 => (
                        "302 Found",
                        location
                            .as_ref()
                            .map(|value| format!("Location: {value}\r\n"))
                            .unwrap_or_default(),
                        BODY,
                    ),
                    2 => ("200 OK", String::new(), flow_body.as_str()),
                    _ => ("200 OK", String::new(), BODY),
                };
                stream.set_write_timeout(Some(BOUND)).unwrap();
                write!(stream, "HTTP/1.1 {status}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
                assert!(requests.len() <= 8, "unexpected extra transfer request");
            }
            requests
        });
        Self {
            address,
            stop,
            worker: Some(worker),
        }
    }

    fn finish(mut self) -> Vec<String> {
        self.stop.store(true, Ordering::SeqCst);
        self.worker.take().unwrap().join().expect("fixture worker")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            // Always join, including unwinding; avoid a second panic in Drop.
            let _ = worker.join();
        }
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
        assert_ne!(count, 0, "fixture premature EOF");
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

fn exercise(
    legacy: bool,
    location: Option<String>,
    flow_body: String,
) -> (Vec<Result<TransferProgress>>, Vec<String>) {
    let fixture = Fixture::new(location, flow_body);
    let (results, logs) = crate::client::logging_tests::capture(true, || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let client = BoursoWebClient::for_test(fixture.address, BOUND).unwrap();
                let from = Account {
                    id: SOURCE.into(),
                    ..Default::default()
                };
                let to = Account {
                    id: "synthetic-private-destination".into(),
                    ..Default::default()
                };
                let stream: std::pin::Pin<Box<dyn Stream<Item = Result<TransferProgress>> + '_>> =
                    if legacy {
                        Box::pin(client.transfer_funds(10.50, from, to, Some(REASON.into())))
                    } else {
                        Box::pin(client.transfer_money(
                            "10.50".parse().unwrap(),
                            from,
                            to,
                            Some(REASON.into()),
                        ))
                    };
                stream.collect::<Vec<_>>().await
            })
    });
    let requests = fixture.finish();
    for result in &results {
        crate::client::logging_tests::assert_clean(
            result.as_ref().err(),
            &logs,
            &[
                SOURCE,
                REFERENCE,
                FLOW,
                BODY,
                REASON,
                "synthetic-private-destination",
            ],
        );
    }
    (results, requests)
}

fn assert_early_error(results: &[Result<TransferProgress>], expected: &str, progress_steps: &[u8]) {
    let error = results
        .last()
        .expect("terminal result")
        .as_ref()
        .expect_err("early error");
    assert!(
        error.downcast_ref::<OutcomeUnknown>().is_none(),
        "initialization is not a final-submission uncertainty"
    );
    assert_eq!(error.to_string(), expected);
    assert_eq!(results.len(), progress_steps.len() + 1);
    for (result, step) in results.iter().zip(progress_steps) {
        assert_eq!(result.as_ref().unwrap().step_number(), *step);
    }
    let diagnostics = format!("{error}\n{error:#}\n{error:?}\n{results:?}");
    for marker in [
        SOURCE,
        REFERENCE,
        FLOW,
        BODY,
        REASON,
        "synthetic-private-destination",
    ] {
        assert!(
            !diagnostics.contains(marker),
            "sensitive fixture marker in diagnostics"
        );
    }
}

#[test]
fn empty_location_reference_stops_transfer_money_before_next_request() {
    assert_empty_location_reference(false);
}

#[test]
fn empty_location_reference_stops_transfer_funds_before_next_request() {
    assert_empty_location_reference(true);
}

fn assert_empty_location_reference(legacy: bool) {
    for suffix in ["", "/1"] {
        let (results, requests) = exercise(
            legacy,
            Some(format!(
                "/compte/cav/{SOURCE}/virements/immediat/nouveau/{suffix}"
            )),
            BODY.into(),
        );
        assert_eq!(
            requests.len(),
            1,
            "empty reference must not reach flow extraction"
        );
        assert!(requests[0].starts_with(&format!(
            "GET /compte/cav/{SOURCE}/virements/immediat/nouveau HTTP/1.1"
        )));
        assert_early_error(&results, "Failed to extract transfer id", &[1, 2]);
    }
}

#[test]
fn missing_and_short_location_references_stop_both_public_streams() {
    for legacy in [false, true] {
        for (location, expected) in [
            (None, "Missing Location header"),
            (Some(String::new()), "Failed to extract transfer id"),
            (
                Some(format!("/compte/cav/{SOURCE}/virements/immediat/nouveau")),
                "Failed to extract transfer id",
            ),
        ] {
            let (results, requests) = exercise(legacy, location, BODY.into());
            assert_eq!(requests.len(), 1);
            assert_early_error(&results, expected, &[1, 2]);
        }
    }
}

#[test]
fn missing_and_empty_flow_instances_stop_before_debit_for_both_public_streams() {
    for legacy in [false, true] {
        for body in [
            BODY.to_string(),
            format!(r#"{BODY}<input name="flow_ImmediateCashTransfer_instance" value="">"#),
        ] {
            let (results, requests) = exercise(
                legacy,
                Some(format!(
                    "/compte/cav/{SOURCE}/virements/immediat/nouveau/{REFERENCE}/1"
                )),
                body,
            );
            assert_eq!(requests.len(), 2);
            assert!(requests.iter().all(|request| request.starts_with("GET ")));
            assert_early_error(&results, "Failed to extract flow instance", &[1, 2, 3]);
        }
    }
}

#[test]
fn nonempty_reference_preserves_positional_extraction_and_every_submit_stage() {
    for legacy in [false, true] {
        // The existing positional policy is intentionally not a prefix validator.
        for prefix in ["compte", "synthetic-alternate-prefix"] {
            let (results, requests) = exercise(
                legacy,
                Some(format!(
                    "/{prefix}/cav/{SOURCE}/virements/immediat/nouveau/{REFERENCE}/1"
                )),
                format!(
                    r#"{BODY}<input name="flow_ImmediateCashTransfer_instance" value="{FLOW}">"#
                ),
            );
            assert_eq!(requests.len(), 8);
            assert!(requests[1].starts_with(&format!(
                "GET /compte/cav/{SOURCE}/virements/immediat/nouveau/{REFERENCE}/1 HTTP/1.1"
            )));
            for (request, step) in requests.iter().skip(2).zip([2, 3, 4, 5, 8, 10]) {
                assert!(request.starts_with(&format!("POST /compte/cav/{SOURCE}/virements/immediat/nouveau/{REFERENCE}/{step} HTTP/1.1")));
                assert!(request.contains(&format!(
                    "name=\"flow_ImmediateCashTransfer_instance\"\r\n\r\n{FLOW}\r\n"
                )));
            }
            assert_eq!(results.len(), 10);
            for (index, result) in results.iter().enumerate() {
                assert_eq!(result.as_ref().unwrap().step_number() as usize, index + 1);
            }
            assert!(matches!(
                results.last().unwrap(),
                Ok(TransferProgress::SubmittedUnverified)
            ));
        }
    }
}
