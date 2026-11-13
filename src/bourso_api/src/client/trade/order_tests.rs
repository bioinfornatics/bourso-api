//! Local transport regressions only: these synthetic replies are not bank contracts.
use super::*;
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};
use tracing::{
    field::{Field, Visit},
    span::{Attributes, Id, Record},
    Event, Metadata, Subscriber,
};

const BOUND: Duration = Duration::from_secs(3);
const SECRET: &str = "synthetic-order-private-marker";

// Kept local because the shared HTTP test module's helper is private.
struct Server {
    address: SocketAddr,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Server {
    fn new(prefill: OrderData) -> Self {
        Self::with_check(prefill, None)
    }

    fn with_check(prefill: OrderData, check: Option<OrderCheckResponse>) -> Self {
        Self::with_responses(
            prefill,
            check,
            Vec::new(),
            OrderConfirmResponse {
                order_id: "fixture-order".into(),
                ..Default::default()
            },
        )
    }

    fn with_responses(
        prefill: OrderData,
        check: Option<OrderCheckResponse>,
        messages: Vec<Value>,
        confirm: OrderConfirmResponse,
    ) -> Self {
        let prepare = OrderPrepareResponse {
            resource_id: SECRET.into(),
            prefill_order_data: prefill,
            acceptability_messages: messages,
            account: PrepareOrderAccount {
                iban: SECRET.into(),
                ..Default::default()
            },
            ..Default::default()
        };
        let replies = [
            ("200 OK", serde_json::to_string(&prepare).unwrap()),
            (
                "200 OK",
                serde_json::to_string(&OrderCheckResponse::default()).unwrap(),
            ),
            ("201 Created", serde_json::to_string(&confirm).unwrap()),
        ];
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&requests);
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&stop);
        let worker = thread::spawn(move || {
            for (index, (status, mut body)) in replies.into_iter().enumerate() {
                let deadline = Instant::now() + BOUND;
                let mut stream = loop {
                    if stopped.load(Ordering::Relaxed) {
                        return;
                    }
                    assert!(Instant::now() < deadline, "local fixture accept timed out");
                    match listener.accept() {
                        Ok((stream, peer)) => {
                            assert!(peer.ip().is_loopback());
                            break stream;
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(2));
                        }
                        Err(e) => panic!("local fixture accept: {e}"),
                    }
                };
                stream.set_read_timeout(Some(BOUND)).unwrap();
                stream.set_write_timeout(Some(BOUND)).unwrap();
                let request = read_request(&mut stream);
                if index == 1 {
                    let echo: OrderData =
                        serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
                    body = serde_json::to_string(&check.clone().unwrap_or(OrderCheckResponse {
                        check_order_data: echo,
                        acceptability_messages: None,
                    }))
                    .unwrap();
                }
                captured.lock().unwrap().push(request);
                write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
        });
        Self {
            address,
            requests,
            stop,
            worker: Some(worker),
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let result = self.worker.take().unwrap().join();
        if !thread::panicking() {
            result.unwrap();
        }
    }
}

fn read_request(stream: &mut TcpStream) -> String {
    let deadline = Instant::now() + BOUND;
    let mut bytes = Vec::new();
    loop {
        assert!(Instant::now() < deadline, "local fixture read timed out");
        let mut chunk = [0; 4096];
        let count = stream.read(&mut chunk).unwrap();
        assert_ne!(count, 0, "request ended prematurely");
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

type Fields = BTreeMap<String, String>;
#[derive(Clone, Default)]
struct Logs(Arc<Mutex<Vec<Fields>>>);
struct Visitor(Fields);
impl Visit for Visitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.0.insert(field.name().into(), format!("{value:?}"));
    }
}
impl Subscriber for Logs {
    fn enabled(&self, _: &Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, _: &Attributes<'_>) -> Id {
        Id::from_u64(1)
    }
    fn record(&self, _: &Id, _: &Record<'_>) {}
    fn record_follows_from(&self, _: &Id, _: &Id) {}
    fn enter(&self, _: &Id) {}
    fn exit(&self, _: &Id) {}
    fn event(&self, event: &Event<'_>) {
        // Capture this API module's diagnostics; dependency sinks have their own tests.
        if event.metadata().target().ends_with("trade::order") {
            let mut visitor = Visitor(BTreeMap::new());
            event.record(&mut visitor);
            self.0.lock().unwrap().push(visitor.0);
        }
    }
}

fn client(address: SocketAddr) -> BoursoWebClient {
    let mut client = BoursoWebClient::for_test(address, BOUND).unwrap();
    client.config.api_url = "https://fixture.invalid/api".into();
    client.config.user_hash = Some("fixture".into());
    client
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

fn data(side: Option<OrderSide>, quantity: Option<usize>) -> OrderData {
    OrderData {
        order_side: side,
        order_quantity: quantity,
        order_price_limit: Some(12.5),
        order_risk_mode: SECRET.into(),
        ..Default::default()
    }
}

fn submit(
    side: OrderSide,
    quantity: usize,
    prefill: OrderData,
    custom: Option<OrderData>,
) -> (Value, Vec<Fields>) {
    let server = Server::new(prefill);
    let logs = Logs::default();
    let output = tracing::subscriber::with_default(logs.clone(), || {
        runtime().block_on(client(server.address).order(
            side,
            &Account::default(),
            "FIXTURE",
            quantity,
            custom,
        ))
    })
    .unwrap();
    assert_eq!(output, ("fixture-order".into(), Some(12.5)));
    let requests = server.requests.lock().unwrap();
    assert_eq!(requests.len(), 3);
    assert!(requests[0].starts_with("GET /api/_user_/_fixture_/trading/order/prepare?"));
    assert!(requests[1].starts_with("POST /api/_user_/_fixture_/trading/ordersimple/check "));
    assert!(requests[2].starts_with("POST /api/_user_/_fixture_/trading/ordersimple/confirm "));
    let payload = serde_json::from_str(requests[1].split_once("\r\n\r\n").unwrap().1).unwrap();
    let events = logs.0.lock().unwrap().clone();
    (payload, events)
}

#[test]
fn explicit_future_expiration_survives_on_real_order_path() {
    let mut prefill = data(None, None);
    prefill.order_validity = Some("2099-01-01".into());
    let mut custom = data(None, None);
    custom.order_expiration_date = Some("2099-12-31".into());
    let (payload, _) = submit(OrderSide::Buy, 4, prefill, Some(custom));
    assert_eq!(payload["orderExpirationDate"], "2099-12-31");
}

#[test]
fn prefilled_expiration_is_preserved_without_custom_order_data() {
    let mut prefill = data(None, None);
    prefill.order_expiration_date = Some("2099-12-31".into());
    prefill.order_validity = Some("2099-01-01".into());
    let (payload, _) = submit(OrderSide::Sell, 4, prefill, None);
    assert_eq!(payload["orderExpirationDate"], "2099-12-31");
}

#[test]
fn absent_expiration_defaults_to_provider_validity_for_both_data_sources() {
    for custom in [None, Some(data(None, None))] {
        let mut prefill = data(None, None);
        prefill.order_validity = Some("2099-06-30".into());
        let (payload, _) = submit(OrderSide::Buy, 4, prefill, custom);
        assert_eq!(payload["orderExpirationDate"], "2099-06-30");
    }
}

#[test]
fn custom_data_without_expiration_uses_validity_not_discarded_prefill_expiration() {
    let mut prefill = data(None, None);
    prefill.order_expiration_date = Some("2099-12-31".into());
    prefill.order_validity = Some("2099-06-30".into());
    let (payload, _) = submit(OrderSide::Buy, 4, prefill, Some(data(None, None)));
    assert_eq!(payload["orderExpirationDate"], "2099-06-30");
}

#[test]
fn absent_expiration_and_provider_validity_remain_absent_without_inventing_today() {
    for custom in [None, Some(data(None, None))] {
        let (payload, _) = submit(OrderSide::Buy, 4, data(None, None), custom);
        assert_eq!(payload.get("orderExpirationDate"), Some(&Value::Null));
    }
}

#[test]
fn calendar_boundaries_are_preserved_as_dates_not_timezone_instants() {
    // No market timezone contract is assumed. These are calendar dates, not timestamps.
    for date in ["2028-02-29", "2099-12-31", "2100-01-01"] {
        let mut custom = data(None, None);
        custom.order_expiration_date = Some(date.into());
        let (payload, _) = submit(OrderSide::Buy, 4, data(None, None), Some(custom));
        assert_eq!(payload["orderExpirationDate"], date);
    }
}

#[test]
fn malformed_effective_expiration_is_rejected_before_check_and_confirm() {
    for date in [
        "",
        "not-a-date",
        "2027-02-29",
        "2100-02-29",
        "2099-04-31",
        "2099-13-01",
        "2099-00-01",
        "2099-01-00",
        "2099-1-01",
        "2099-01-1",
        " 2099-01-01",
        "2099-01-01 ",
        "2099-01-01T00:00:00Z",
        "+2099-01-01",
        "２０９９-01-01",
    ] {
        for source in 0..3 {
            let mut prefill = data(None, None);
            let custom = match source {
                0 => {
                    let mut custom = data(None, None);
                    custom.order_expiration_date = Some(date.into());
                    // An invalid explicit date must not silently fall back to a valid default.
                    prefill.order_validity = Some("2099-06-30".into());
                    Some(custom)
                }
                1 => {
                    prefill.order_expiration_date = Some(date.into());
                    None
                }
                _ => {
                    prefill.order_validity = Some(date.into());
                    None
                }
            };
            let server = Server::new(prefill);
            let error = runtime()
                .block_on(client(server.address).order(
                    OrderSide::Buy,
                    &Account::default(),
                    "FIXTURE",
                    4,
                    custom,
                ))
                .expect_err("malformed expiration was submitted");
            assert_eq!(
                error.to_string(),
                "Invalid order expiration date: expected YYYY-MM-DD"
            );
            let requests = server.requests.lock().unwrap();
            assert_eq!(requests.len(), 1, "malformed date reached a POST");
            assert!(requests[0].starts_with("GET /api/_user_/_fixture_/trading/order/prepare?"));
        }
    }
}

#[test]
fn sell_four_overrides_buy_one_prefill_on_real_order_path() {
    let (payload, _) = submit(
        OrderSide::Sell,
        4,
        data(Some(OrderSide::Buy), Some(1)),
        None,
    );
    assert_eq!(payload["orderSide"], "S");
    assert_eq!(payload["orderQuantity"], 4);
}

#[test]
fn explicit_intent_wins_for_both_sides_and_all_data_sources() {
    for (side, expected, opposite) in [
        (OrderSide::Buy, "B", OrderSide::Sell),
        (OrderSide::Sell, "S", OrderSide::Buy),
    ] {
        for initial in [
            data(None, None),
            data(Some(side), Some(4)),
            data(Some(opposite), Some(1)),
        ] {
            for custom in [false, true] {
                let (payload, _) = if custom {
                    submit(
                        side,
                        4,
                        data(Some(opposite), Some(1)),
                        Some(initial.clone()),
                    )
                } else {
                    submit(side, 4, initial.clone(), None)
                };
                assert_eq!(payload["orderSide"], expected);
                assert_eq!(payload["orderQuantity"], 4);
                assert_eq!(payload["orderPriceLimit"], 12.5);
                assert_eq!(payload["orderRiskMode"], SECRET);
            }
        }
    }
}

#[test]
fn zero_quantity_is_rejected_before_even_prepare() {
    for side in [OrderSide::Buy, OrderSide::Sell] {
        for custom in [None, Some(data(Some(side), Some(9)))] {
            let server = Server::new(data(Some(side), Some(9)));
            let result = runtime().block_on(client(server.address).order(
                side,
                &Account::default(),
                "FIXTURE",
                0,
                custom,
            ));
            assert!(result.is_err(), "zero quantity must be rejected");
            assert!(
                server.requests.lock().unwrap().is_empty(),
                "zero quantity emitted HTTP"
            );
            assert!(result
                .unwrap_err()
                .to_string()
                .to_lowercase()
                .contains("quantity"));
        }
    }
}

#[test]
fn submission_log_matches_intent_without_claiming_execution() {
    let (payload, events) = submit(
        OrderSide::Sell,
        4,
        data(Some(OrderSide::Buy), Some(1)),
        None,
    );
    let success = events
        .iter()
        .find(|fields| {
            fields
                .get("message")
                .is_some_and(|message| message.contains("execution unverified"))
        })
        .expect("missing unverified submission log");
    assert_eq!(success.get("side").map(String::as_str), Some("Some(Sell)"));
    assert_eq!(success.get("quantity").map(String::as_str), Some("4"));
    assert_eq!(payload["orderSide"], "S");
    assert_eq!(payload["orderQuantity"], 4);
    assert!(
        !format!("{events:?}").contains(SECRET),
        "private fixture data leaked"
    );
}

#[test]
fn divergent_check_does_not_confirm() {
    let mut echo = data(Some(OrderSide::Sell), Some(4));
    echo.resource_id = Some(SECRET.into());
    let server = Server::with_check(
        data(None, None),
        Some(OrderCheckResponse {
            check_order_data: echo,
            acceptability_messages: None,
        }),
    );
    let result = runtime().block_on(client(server.address).order(
        OrderSide::Buy,
        &Account::default(),
        "FIXTURE",
        4,
        None,
    ));
    assert_eq!(
        server.requests.lock().unwrap().len(),
        2,
        "divergent check reached confirm"
    );
    assert!(result.is_err());
}

#[test]
fn every_divergent_intent_field_prevents_confirmation() {
    for field in [
        "orderSide",
        "orderQuantity",
        "orderPriceLimit",
        "orderType",
        "orderExpirationDate",
        "resourceId",
    ] {
        let mut echo = data(Some(OrderSide::Buy), Some(4));
        echo.resource_id = Some(SECRET.into());
        match field {
            "orderSide" => echo.order_side = Some(OrderSide::Sell),
            "orderQuantity" => echo.order_quantity = Some(5),
            "orderPriceLimit" => echo.order_price_limit = Some(13.0),
            "orderType" => echo.order_type = OrderKind::Market,
            "orderExpirationDate" => echo.order_expiration_date = Some("2099-12-31".into()),
            "resourceId" => echo.resource_id = Some("different-private-resource".into()),
            _ => unreachable!(),
        }
        let server = Server::with_check(
            data(None, None),
            Some(OrderCheckResponse {
                check_order_data: echo,
                acceptability_messages: None,
            }),
        );
        let error = runtime()
            .block_on(client(server.address).order(
                OrderSide::Buy,
                &Account::default(),
                "FIXTURE",
                4,
                None,
            ))
            .unwrap_err();
        assert!(error.to_string().contains("intent differs"), "{field}");
        assert!(!format!("{error:?}").contains(SECRET));
        assert!(error.downcast_ref::<OutcomeUnknown>().is_none());
        assert_eq!(server.requests.lock().unwrap().len(), 2, "{field}");
    }
}

#[test]
fn all_nonempty_check_messages_require_review_without_classifying_warning() {
    for kind in ["ERROR", "WARNING", "INFO", "UNKNOWN", ""] {
        let mut echo = data(Some(OrderSide::Buy), Some(4));
        echo.resource_id = Some(SECRET.into());
        let server = Server::with_check(
            data(None, None),
            Some(OrderCheckResponse {
                check_order_data: echo,
                acceptability_messages: Some(vec![AcceptabilityMessage {
                    type_field: kind.into(),
                    content: SECRET.into(),
                }]),
            }),
        );
        let error = runtime()
            .block_on(client(server.address).order(
                OrderSide::Buy,
                &Account::default(),
                "FIXTURE",
                4,
                None,
            ))
            .unwrap_err();
        assert!(error.to_string().contains("requires review"));
        assert!(!format!("{error:?}").contains(SECRET));
        assert!(error.downcast_ref::<OutcomeUnknown>().is_none());
        assert_eq!(server.requests.lock().unwrap().len(), 2);
    }
}

#[test]
fn preparation_messages_require_review_before_check() {
    let server = Server::with_responses(
        data(None, None),
        None,
        vec![serde_json::json!({"type":"WARNING", "content": SECRET})],
        OrderConfirmResponse::default(),
    );
    let error = runtime()
        .block_on(client(server.address).order(
            OrderSide::Buy,
            &Account::default(),
            "FIXTURE",
            4,
            None,
        ))
        .unwrap_err();
    assert!(error.to_string().contains("requires review"));
    assert!(!format!("{error:?}").contains(SECRET));
    assert_eq!(server.requests.lock().unwrap().len(), 1);
}

#[test]
fn detailed_submission_preserves_raw_state_but_never_asserts_execution() {
    for state in ["rejected", "unknown-provider-code", "executed", ""] {
        let mut echo = data(Some(OrderSide::Buy), Some(4));
        // Missing resourceId is allowed; the other intent fields still match.
        echo.resource_id = None;
        let server = Server::with_responses(
            data(None, None),
            Some(OrderCheckResponse {
                check_order_data: echo,
                acceptability_messages: Some(vec![]),
            }),
            vec![],
            OrderConfirmResponse {
                order_id: SECRET.into(),
                ord_stat: state.into(),
                order_state_label: SECRET.into(),
                action_message: ActionMessage {
                    body: Value::String(SECRET.into()),
                    ..Default::default()
                },
            },
        );
        let logs = Logs::default();
        let result = tracing::subscriber::with_default(logs.clone(), || {
            runtime().block_on(client(server.address).order_detailed(
                OrderSide::Buy,
                &Account::default(),
                "FIXTURE",
                4,
                None,
            ))
        })
        .unwrap();
        assert_eq!(result.status, OrderSubmissionStatus::SubmittedUnverified);
        assert_eq!(result.order_id, SECRET);
        assert_eq!(result.order_price_limit, Some(12.5));
        assert_eq!(result.provider_state.expose().ord_stat, state);
        assert_eq!(result.provider_state.expose().action_message.body, SECRET);
        assert!(!format!("{result:?} {:?}", result.provider_state).contains(SECRET));
        let events = format!("{:?}", logs.0.lock().unwrap());
        assert!(!events.contains(SECRET));
        assert!(!events.contains("successfully"));
        assert!(events.contains("execution unverified"));
        assert_eq!(server.requests.lock().unwrap().len(), 3);
    }
}
