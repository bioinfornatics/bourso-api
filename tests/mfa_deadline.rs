//! Uses the actual private MFA implementation, not a cloned polling algorithm.
//! Fixtures substitute only setup/check responses and QR presentation; no bank IO.
//! This is common-seam coverage, not a live API/CLI authentication test.
#[path = "../src/mfa.rs"]
mod mfa;

use anyhow::Result;
use bourso_api::client::error::ClientError;
use std::{cell::Cell, future::pending, time::Duration};

mod polling {
    use super::*;

    async fn wait_until<F>(budget: Duration, interval: Duration, check: F) -> Result<()>
    where
        F: std::ops::AsyncFnMut() -> Result<bool>,
    {
        mfa::within_deadline(budget, mfa::wait_until(interval, check)).await
    }
    use std::{cell::Cell, future::pending};

    const BUDGET: Duration = Duration::from_millis(10);
    // Watchdog only: never confuse a stuck helper with its own timeout result.
    const WATCHDOG: Duration = Duration::from_secs(1);

    fn assert_deadline(error: anyhow::Error) {
        assert!(error
            .downcast_ref::<tokio::time::error::Elapsed>()
            .is_some());
        assert!(error.to_string().starts_with("MFA validation timed out"));
    }

    #[tokio::test]
    async fn validation_returns_without_sleep_or_another_poll() {
        let mut calls = 0;
        tokio::time::timeout(
            WATCHDOG,
            wait_until(WATCHDOG, WATCHDOG, async || {
                calls += 1;
                Ok(true)
            }),
        )
        .await
        .expect("validated MFA must not sleep")
        .unwrap();
        assert_eq!(calls, 1);
    }

    #[tokio::test]
    async fn callback_error_is_preserved_without_retry() {
        let mut calls = 0;
        let error = tokio::time::timeout(
            WATCHDOG,
            wait_until(WATCHDOG, WATCHDOG, async || {
                calls += 1;
                Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "rejected").into())
            }),
        )
        .await
        .expect("callback error must not sleep")
        .unwrap_err();
        assert_eq!(calls, 1);
        assert_eq!(
            error.downcast_ref::<std::io::Error>().unwrap().kind(),
            std::io::ErrorKind::PermissionDenied
        );
    }

    #[tokio::test]
    async fn repeated_pending_responses_cannot_reset_the_budget() {
        let mut calls = 0;
        let error = tokio::time::timeout(
            WATCHDOG,
            wait_until(BUDGET, Duration::from_millis(1), async || {
                calls += 1;
                Ok(false)
            }),
        )
        .await
        .expect("pending polling must terminate under its own deadline")
        .unwrap_err();
        assert_deadline(error);
        assert!(calls >= 1);
    }

    #[tokio::test]
    async fn deadline_interrupts_a_sleep_longer_than_the_budget() {
        let mut calls = 0;
        let error = tokio::time::timeout(
            WATCHDOG,
            wait_until(BUDGET, WATCHDOG, async || {
                calls += 1;
                Ok(false)
            }),
        )
        .await
        .expect("retry sleep must be inside the deadline")
        .unwrap_err();
        assert_deadline(error);
        assert_eq!(calls, 1);
    }

    #[tokio::test]
    async fn hung_request_is_dropped_at_the_deadline() {
        struct OnDrop<'a>(&'a Cell<bool>);
        impl Drop for OnDrop<'_> {
            fn drop(&mut self) {
                self.0.set(true);
            }
        }
        let dropped = Cell::new(false);
        let error = tokio::time::timeout(
            WATCHDOG,
            wait_until(BUDGET, BUDGET, async || {
                let _guard = OnDrop(&dropped);
                pending::<Result<bool>>().await
            }),
        )
        .await
        .expect("hung request must be interrupted by the helper")
        .unwrap_err();
        assert_deadline(error);
        assert!(dropped.get(), "timeout must drop the in-flight request");
    }

    #[tokio::test]
    async fn mutable_callback_can_validate_after_pending() {
        let mut calls = 0;
        wait_until(WATCHDOG, Duration::from_millis(1), async || {
            calls += 1;
            tokio::task::yield_now().await;
            Ok(calls == 2)
        })
        .await
        .unwrap();
        assert_eq!(calls, 2);
    }
}

const BUDGET: Duration = Duration::from_millis(100);
const WATCHDOG: Duration = Duration::from_secs(2);

// This adapter supplies fixtures only. Deadline, poll loop and QR/error mapping
// are all imported unchanged from production. It deliberately owns no loop/timer.
async fn fixture<S, C, D>(
    budget: Duration,
    interval: Duration,
    mut setup: S,
    mut check: C,
    mut display: D,
) -> Result<()>
where
    S: std::ops::AsyncFnMut() -> Result<()>,
    C: std::ops::AsyncFnMut() -> Result<bool>,
    D: FnMut(&str) -> Result<()>,
{
    mfa::within_deadline(budget, async {
        setup().await?;
        mfa::wait_until(interval, async || {
            mfa::validation_status(check().await, &mut display)
        })
        .await
    })
    .await
}

fn assert_deadline(error: anyhow::Error) {
    assert!(
        error
            .downcast_ref::<tokio::time::error::Elapsed>()
            .is_some(),
        "{error:#}"
    );
    assert_eq!(
        error.to_string(),
        "MFA validation timed out after 0.1 seconds"
    );
}

struct OnDrop<'a>(&'a Cell<bool>);
impl Drop for OnDrop<'_> {
    fn drop(&mut self) {
        self.0.set(true);
    }
}

#[tokio::test]
async fn hung_setup_is_cancelled_without_starting_a_check() {
    let dropped = Cell::new(false);
    let calls = Cell::new(0);
    let error = tokio::time::timeout(
        WATCHDOG,
        fixture(
            BUDGET,
            Duration::ZERO,
            async || {
                let _guard = OnDrop(&dropped);
                pending::<Result<()>>().await
            },
            async || {
                calls.set(calls.get() + 1);
                Ok(true)
            },
            |_| panic!("setup never returned a QR"),
        ),
    )
    .await
    .expect("setup must expire under the shared deadline")
    .unwrap_err();
    assert_deadline(error);
    assert!(dropped.get());
    assert_eq!(calls.get(), 0);
}

#[tokio::test]
async fn slow_setup_consumes_the_polling_budget() {
    let mut setup_calls = 0;
    let mut check_calls = 0;
    let error = tokio::time::timeout(
        WATCHDOG,
        fixture(
            BUDGET,
            Duration::ZERO,
            async || {
                setup_calls += 1;
                tokio::time::sleep(Duration::from_millis(60)).await;
                Ok(())
            },
            async || {
                check_calls += 1;
                // Each step fits in 100ms separately, but their sum does not.
                tokio::time::sleep(Duration::from_millis(60)).await;
                Ok(true)
            },
            |_| panic!("fixture contains no QR"),
        ),
    )
    .await
    .expect("setup and checks must share a budget")
    .unwrap_err();
    assert_deadline(error);
    assert_eq!(setup_calls, 1);
    assert_eq!(check_calls, 1);
}

#[tokio::test]
async fn pending_and_repeated_qr_events_cannot_restart_the_deadline() {
    let mut setups = 0;
    let mut checks = 0;
    let mut displayed = Vec::new();
    let error = tokio::time::timeout(
        WATCHDOG,
        fixture(
            BUDGET,
            Duration::from_millis(1),
            async || {
                setups += 1;
                Ok(())
            },
            async || {
                checks += 1;
                if checks % 2 == 1 {
                    Ok(false)
                } else {
                    Err(anyhow::anyhow!(ClientError::QRCodeRequired(format!(
                        "qr-{checks}"
                    ))))
                }
            },
            |code| {
                displayed.push(code.to_owned());
                Ok(())
            },
        ),
    )
    .await
    .expect("repeated QR responses must expire without watchdog intervention")
    .unwrap_err();
    assert_deadline(error);
    assert_eq!(setups, 1);
    assert!(
        displayed.len() >= 2,
        "fixture must actually observe repeated QR events"
    );
    assert_eq!(displayed[0], "qr-2");
    assert_eq!(displayed[1], "qr-4");
    assert_eq!(displayed.len(), checks / 2);
}

#[tokio::test]
async fn ready_after_pending_and_qr_finishes_without_an_extra_check() {
    let mut events = std::collections::VecDeque::from([
        Ok(false),
        Err(anyhow::anyhow!(ClientError::QRCodeRequired(
            "fixture-qr".into()
        ))),
        Ok(true),
    ]);
    let mut displayed = Vec::new();
    tokio::time::timeout(
        WATCHDOG,
        fixture(
            WATCHDOG,
            Duration::ZERO,
            async || Ok(()),
            async || events.pop_front().expect("no check after validation"),
            |code| {
                displayed.push(code.to_owned());
                Ok(())
            },
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(events.is_empty());
    assert_eq!(displayed, ["fixture-qr"]);
}

#[tokio::test]
async fn setup_error_is_preserved_without_polling() {
    let error = fixture(
        WATCHDOG,
        WATCHDOG,
        async || {
            Err(std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "setup fixture").into())
        },
        async || panic!("no check after setup error"),
        |_| panic!("no QR after setup error"),
    )
    .await
    .unwrap_err();
    assert_eq!(
        error.downcast_ref::<std::io::Error>().unwrap().kind(),
        std::io::ErrorKind::ConnectionRefused
    );
}

#[tokio::test]
async fn invalid_mfa_is_not_swallowed_or_retried() {
    let mut calls = 0;
    let error = fixture(
        WATCHDOG,
        WATCHDOG,
        async || Ok(()),
        async || {
            calls += 1;
            Err(anyhow::anyhow!(ClientError::InvalidMfa))
        },
        |_| panic!("invalid MFA is not a QR"),
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error.downcast_ref::<ClientError>(),
        Some(ClientError::InvalidMfa)
    ));
    assert_eq!(calls, 1);
}

#[tokio::test]
async fn qr_display_failure_propagates_without_retry() {
    let mut calls = 0;
    let error = fixture(
        WATCHDOG,
        WATCHDOG,
        async || Ok(()),
        async || {
            calls += 1;
            Err(anyhow::anyhow!(ClientError::QRCodeRequired(
                "fixture".into()
            )))
        },
        |_| Err(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "display fixture").into()),
    )
    .await
    .unwrap_err();
    assert_eq!(
        error.downcast_ref::<std::io::Error>().unwrap().kind(),
        std::io::ErrorKind::BrokenPipe
    );
    assert_eq!(calls, 1);
}

#[tokio::test]
async fn caller_cancellation_drops_the_inflight_check() {
    let started = tokio::sync::Notify::new();
    let dropped = Cell::new(false);
    {
        let operation = fixture(
            WATCHDOG,
            Duration::ZERO,
            async || Ok(()),
            async || {
                let _guard = OnDrop(&dropped);
                started.notify_one();
                pending::<Result<bool>>().await
            },
            |_| panic!("fixture contains no QR"),
        );
        tokio::pin!(operation);
        tokio::time::timeout(WATCHDOG, async {
            tokio::select! {
                _ = started.notified() => (),
                result = &mut operation => panic!("must remain pending: {result:?}"),
            }
        })
        .await
        .expect("fixture must enter the check");
    }
    assert!(
        dropped.get(),
        "dropping the orchestration must drop the request"
    );
}

#[test]
fn production_wires_setup_and_polling_inside_the_same_300_second_boundary() {
    assert_eq!(mfa::VALIDATION_TIMEOUT, Duration::from_secs(300));
    assert_eq!(mfa::POLL_INTERVAL, Duration::from_secs(5));
    // Static wiring guard only, not a claim of executing the CLI/bank workflow.
    let source = include_str!("../src/lib.rs");
    let block = source
        .split("mfa::within_deadline(mfa::VALIDATION_TIMEOUT, async {")
        .nth(1)
        .expect("CLI must use the tested deadline helper")
        .split("info!(\"MFA successful")
        .next()
        .unwrap();
    let setup = block.find("web_client.request_mfa().await?").unwrap();
    let poll = block.find("mfa::wait_until(mfa::POLL_INTERVAL").unwrap();
    assert!(setup < poll);
    assert!(block.contains("mfa::validation_status(response,"));
    assert!(!block.contains("tokio::time::timeout"), "no nested budget");
}
