//! Conservative classification of errors after a final financial POST is attempted.
//!
//! This is not a bank receipt, idempotency key, reconciliation API, or durable
//! journal. Dropping a pending future/stream cannot return this error; callers
//! still need an external durable journal before starting financial operations.
//!
//! Currently covers final order placement and immediate-transfer confirmation
//! only, not order cancellation or earlier multi-step mutations. Existing success
//! recognition is unchanged and is not an established provider business contract.
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutcomeOperation {
    Order,
    Transfer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutcomeStage {
    Sending,
    ReadingResponse,
    HttpStatus,
    ParsingResponse,
}

/// The remote operation may have taken effect. Do not automatically reissue it.
///
/// Downcast an `anyhow::Error` (including errors with added context) to this type.
/// Diagnostics deliberately omit the server response, URL, tokens and reference.
#[derive(Clone)]
pub struct OutcomeUnknown {
    pub operation: OutcomeOperation,
    pub stage: OutcomeStage,
    /// Local diagnostic identifier; NOT a provider reference or idempotency key.
    pub correlation: String,
    reference: String,
}

impl OutcomeUnknown {
    pub(crate) fn new(operation: OutcomeOperation, reference: &str) -> Self {
        Self {
            operation,
            stage: OutcomeStage::Sending,
            correlation: format!("{:032x}", rand::random::<u128>()),
            reference: reference.to_owned(),
        }
    }

    pub(crate) fn at_stage(&self, stage: OutcomeStage) -> Self {
        let mut error = self.clone();
        error.stage = stage;
        error
    }

    /// Sensitive provider reference for privileged handling only. Never log it.
    /// Its presence does not establish that reconciliation is supported.
    pub fn reference(&self) -> &str {
        &self.reference
    }
}

impl fmt::Display for OutcomeUnknown {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Outcome unknown for {:?} at {:?} (correlation {}): the operation may have been applied. Do not reissue or retry automatically; verify its status with the bank before taking further action.", self.operation, self.stage, self.correlation)
    }
}

impl fmt::Debug for OutcomeUnknown {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OutcomeUnknown")
            .field("operation", &self.operation)
            .field("stage", &self.stage)
            .field("correlation", &self.correlation)
            .field("reference", &"[REDACTED]")
            .finish()
    }
}

impl std::error::Error for OutcomeUnknown {}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::client::BoursoWebClient;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        },
        thread,
        time::{Duration, Instant},
    };

    pub(crate) const SECRET: &str = "synthetic-private-resource-token";

    pub(crate) fn run(future: impl std::future::Future<Output = ()>) {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(future);
    }

    // The local server records a complete POST as applied, then cuts the response
    // or supplies a synthetic reply. This is not a provider business contract.
    pub(crate) fn server(
        reply: &'static str,
    ) -> (BoursoWebClient, Arc<AtomicUsize>, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let mut client =
            BoursoWebClient::for_test(listener.local_addr().unwrap(), Duration::from_secs(2))
                .unwrap();
        client.config.api_url = "https://fixture.invalid/api".into();
        client.config.user_hash = Some(SECRET.into());
        let applied = Arc::new(AtomicUsize::new(0));
        let count = Arc::clone(&applied);
        let worker = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(3);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, peer)) => {
                        assert!(peer.ip().is_loopback());
                        break stream;
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "local POST not received");
                        thread::sleep(Duration::from_millis(2));
                    }
                    Err(e) => panic!("local accept: {e}"),
                }
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            stream
                .set_write_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = Vec::new();
            loop {
                let mut buf = [0; 4096];
                let n = stream.read(&mut buf).unwrap();
                assert!(n > 0);
                request.extend_from_slice(&buf[..n]);
                if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                    let header = String::from_utf8_lossy(&request[..end]);
                    assert!(header.starts_with("POST "));
                    let length: usize = header
                        .lines()
                        .filter_map(|s| s.split_once(':'))
                        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                        .unwrap()
                        .1
                        .trim()
                        .parse()
                        .unwrap();
                    if request.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            count.fetch_add(1, Ordering::SeqCst);
            stream.write_all(reply.as_bytes()).unwrap();
            drop(stream);
            // Keep the listener alive briefly so an accidental immediate retry
            // cannot silently escape the request-count assertion.
            let deadline = Instant::now() + Duration::from_millis(100);
            while Instant::now() < deadline {
                if let Ok((stream, _)) = listener.accept() {
                    count.fetch_add(1, Ordering::SeqCst);
                    drop(stream);
                }
                thread::sleep(Duration::from_millis(2));
            }
        });
        (client, applied, worker)
    }

    pub(crate) fn assert_unknown(
        error: anyhow::Error,
        operation: OutcomeOperation,
        stage: OutcomeStage,
    ) {
        let error = error.context("financial operation failed");
        let unknown = error
            .downcast_ref::<OutcomeUnknown>()
            .expect("post-submission error must downcast to OutcomeUnknown");
        assert_eq!(unknown.operation, operation);
        assert_eq!(unknown.stage, stage);
        assert_eq!(unknown.reference(), SECRET);
        assert_eq!(unknown.correlation.len(), 32);
        for output in [
            format!("{error}"),
            format!("{error:#}"),
            format!("{error:?}"),
            format!("{unknown:?}"),
            format!("{unknown}"),
        ] {
            assert!(!output.contains(SECRET));
        }
        assert!(unknown.to_string().contains("Do not reissue"));
        // This is the formatting used by a CLI main returning anyhow::Result.
        assert!(format!("{error:?}").contains("Do not reissue"));
    }

    #[test]
    fn context_preserves_unknown_and_redacts_reference() {
        assert_unknown(
            OutcomeUnknown::new(OutcomeOperation::Order, SECRET).into(),
            OutcomeOperation::Order,
            OutcomeStage::Sending,
        );
    }
}
