//! Synthetic secrets only. Capture synchronously on the current test/runtime thread.
use std::{
    io::Write,
    sync::{Arc, Mutex},
};

#[derive(Clone, Default)]
struct Buffer(Arc<Mutex<Vec<u8>>>);
impl Write for Buffer {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

pub(super) fn capture<T>(enabled: bool, action: impl FnOnce() -> T) -> (T, String) {
    let buffer = Buffer::default();
    // A tiny synchronous subscriber avoids any extra dev dependency and
    // captures every field (including Debug values), not just message text.
    struct Capture {
        buffer: Buffer,
        enabled: bool,
    }
    struct Fields(Buffer);
    impl tracing::field::Visit for Fields {
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            writeln!(self.0, "{}={value:?}", field.name()).unwrap();
        }
    }
    impl tracing::Subscriber for Capture {
        fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
            self.enabled
        }
        fn max_level_hint(&self) -> Option<tracing::level_filters::LevelFilter> {
            Some(if self.enabled {
                tracing::level_filters::LevelFilter::TRACE
            } else {
                tracing::level_filters::LevelFilter::OFF
            })
        }
        fn new_span(&self, attrs: &tracing::span::Attributes<'_>) -> tracing::span::Id {
            attrs.record(&mut Fields(self.buffer.clone()));
            tracing::span::Id::from_u64(1)
        }
        fn record(&self, _: &tracing::span::Id, values: &tracing::span::Record<'_>) {
            values.record(&mut Fields(self.buffer.clone()));
        }
        fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
        fn event(&self, event: &tracing::Event<'_>) {
            event.record(&mut Fields(self.buffer.clone()));
        }
        fn enter(&self, _: &tracing::span::Id) {}
        fn exit(&self, _: &tracing::span::Id) {}
    }
    let subscriber = Capture {
        buffer: buffer.clone(),
        enabled,
    };
    // tracing-core 0.1.33's single-dispatcher cold-callsite path consults the
    // registering thread's default, which may be NoSubscriber on another thread.
    // Keep two DISTINCT registered dispatchers alive through action(): a clone of
    // the same Dispatch does not register twice. The inert one never captures or
    // raises the level hint; it is not installed as a thread/global subscriber.
    let dispatch = tracing::Dispatch::new(subscriber);
    let inert = tracing::Dispatch::new(Capture {
        buffer: Buffer::default(),
        enabled: false,
    });
    let result = tracing::dispatcher::with_default(&dispatch, || {
        tracing::trace!("capture-probe");
        action()
    });
    drop(inert);
    drop(dispatch);
    let text = String::from_utf8(buffer.0.lock().unwrap().clone()).unwrap();
    assert_eq!(
        text.contains("capture-probe"),
        enabled,
        "subscriber must actually capture TRACE"
    );
    (result, text)
}

pub(super) fn assert_clean(error: Option<&anyhow::Error>, logs: &str, secrets: &[&str]) {
    let mut sinks = vec![logs.to_owned()];
    if let Some(error) = error {
        sinks.extend([
            format!("{error}"),
            format!("{error:#}"),
            format!("{error:?}"),
            format!("{error:#?}"),
        ]);
        sinks.extend(error.chain().map(|cause| format!("{cause} {cause:?}")));
    }
    for (i, sink) in sinks.iter().enumerate() {
        for secret in secrets {
            assert!(
                !sink.contains(secret),
                "sensitive sentinel in sink {i}: {sink}"
            );
        }
    }
}

#[test]
fn logging_extraction_errors_do_not_emit_page_secrets() {
    for enabled in [true, false] {
        for extract in [
            super::extract_token,
            super::extract_brs_mit_cookie,
            super::extract_user_contact,
        ] {
            let (result, logs) = capture(enabled, || extract("SENTINEL_PAGE"));
            assert_clean(Some(&result.unwrap_err()), &logs, &["SENTINEL_PAGE"]);
        }
        for page in [
            "SENTINEL_PAGE",
            r#"data-strong-authentication-payload="{&quot;secret&quot;:&quot;SENTINEL_PAGE&quot;,invalid}">"#,
            r#"data-strong-authentication-payload="{&quot;secret&quot;:&quot;SENTINEL_PAGE&quot;}">"#,
            r#"data-strong-authentication-payload="{&quot;secret&quot;:&quot;SENTINEL_PAGE&quot;,&quot;challenges&quot;:[{&quot;parameters&quot;:{&quot;formScreen&quot;:{&quot;actions&quot;:{&quot;check&quot;:{&quot;api&quot;:{&quot;params&quot;:{&quot;resourceId&quot;:&quot;fixture&quot;}}}}}}}]}">"#,
        ] {
            let (result, logs) = capture(enabled, || super::extract_otp_params(page));
            assert_clean(Some(&result.unwrap_err()), &logs, &["SENTINEL_PAGE"]);
        }
    }
}

#[test]
fn logging_config_parse_error_redacts_serde_value_and_source() {
    for enabled in [true, false] {
        // Valid JSON, wrong field type: serde's Display includes the secret value.
        for page in [
            r#"window.BRS_CONFIG = {"DEBUG":"SENTINEL_CONFIG"};"#,
            r#"window.BRS_CONFIG = {"DEFAULT_API_BEARER":"SENTINEL_CONFIG",invalid};"#,
        ] {
            let (result, logs) = capture(enabled, || super::config::extract_brs_config(page));
            let error = result.unwrap_err();
            assert_clean(Some(&error), &logs, &["SENTINEL_CONFIG"]);
            assert!(error.to_string().contains("BRS_CONFIG"));
            assert!(error.to_string().contains("at line"));
        }
    }
}

#[test]
fn logging_password_and_svg_errors_do_not_echo_input() {
    for enabled in [true, false] {
        let (result, logs) = capture(enabled, || {
            super::virtual_pad::password_to_virtual_pad_keys(vec!["fixture".into(); 10], "12☃")
        });
        let error = result.unwrap_err();
        assert_clean(Some(&error), &logs, &["☃"]);
        assert!(error.to_string().contains("password"));
        let (result, logs) = capture(enabled, || {
            super::virtual_pad::extract_data_matrix_keys(
                r#"<button data-matrix-key="ABC" src="data:image/SENTINEL_SVG"></button>"#,
            )
        });
        assert_clean(Some(&result.unwrap_err()), &logs, &["SENTINEL_SVG"]);
    }
}

#[test]
fn capture_cold_callsite_from_uncaptured_thread() {
    // Run exactly one test in a new process: other tests' dispatchers/callsites
    // would hide the single-dispatcher cold-registration condition.
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "client::logging_tests::capture_cold_callsite_child",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "isolated capture regression failed: {}\n{}\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

#[test]
#[ignore = "fresh-process child; invoked by capture_cold_callsite_from_uncaptured_thread"]
fn capture_cold_callsite_child() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn emit(attempts: &AtomicUsize, phase: usize) {
        attempts.fetch_add(1, Ordering::SeqCst);
        tracing::debug!("capture-cold-target phase={phase}");
    }

    let root = std::thread::current().id();
    let attempts = AtomicUsize::new(0);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let (_, logs) = capture(true, || {
        runtime.block_on(async {
            assert_eq!(root, std::thread::current().id());
            assert_eq!(
                tracing::level_filters::LevelFilter::current(),
                tracing::level_filters::LevelFilter::TRACE
            );
            tracing::debug!("capture-cold-control-before");
            std::thread::scope(|scope| {
                scope
                    .spawn(|| {
                        tracing::dispatcher::get_default(|dispatch| {
                            assert!(dispatch.is::<tracing::subscriber::NoSubscriber>());
                        });
                        emit(&attempts, 1);
                    })
                    .join()
                    .unwrap();
            });
            tokio::task::yield_now().await;
            assert_eq!(root, std::thread::current().id());
            emit(&attempts, 2);
            tracing::debug!("capture-cold-control-after");
            eprintln!(
                "root={root:?} resumed={:?} max={:?} attempts={}",
                std::thread::current().id(),
                tracing::level_filters::LevelFilter::current(),
                attempts.load(Ordering::SeqCst)
            );
        });
    });
    eprintln!("CAPTURED\n{logs}END_CAPTURED");
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
    assert!(logs.contains("capture-cold-control-before"));
    assert!(logs.contains("capture-cold-control-after"));
    assert!(!logs.contains("capture-cold-target phase=1"));
    assert_eq!(
        logs.matches("capture-cold-target phase=2").count(),
        1,
        "ON capture must deliver after first registration on an uncaptured thread"
    );
}

#[test]
fn capture_parallel_on_off_shared_callsites() {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Barrier,
    };

    const ROUNDS: usize = 32;
    // Both threads use these exact callsites, including their first registration.
    // Count attempted emissions independently of tracing's filtering/capture path.
    fn emit(attempts: &AtomicUsize, worker: usize, round: usize, phase: usize) {
        attempts.fetch_add(1, Ordering::SeqCst);
        tracing::info!("capture-sync-info worker={worker} round={round} phase={phase}");
        tracing::trace!("capture-sync-trace worker={worker} round={round} phase={phase}");
    }

    let barrier = Barrier::new(2);
    let attempts = AtomicUsize::new(0);
    let results = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..2)
            .map(|worker| {
                let barrier = &barrier;
                let attempts = &attempts;
                scope.spawn(move || {
                    let mut results = Vec::new();
                    for round in 0..ROUNDS {
                        let enabled = (worker + round) % 2 == 0;
                        barrier.wait(); // Race dispatcher registration, not whole test runs.
                                        // Defer helper assertion failures until both workers finish all
                                        // rendezvous; an observed miss must not strand the other thread.
                        let result = std::panic::catch_unwind(|| {
                            capture(enabled, || {
                                barrier.wait(); // Both ON and OFF dispatchers are alive.
                                emit(attempts, worker, round, 0);
                                barrier.wait();
                                // Race an ON emission against OFF registration/rebuild,
                                // then check the restored outer dispatchers at phase 2.
                                let nested = if enabled {
                                    emit(attempts, worker, round, 1);
                                    None
                                } else {
                                    Some(std::panic::catch_unwind(|| {
                                        capture(false, || emit(attempts, worker, round, 1))
                                    }))
                                };
                                barrier.wait();
                                emit(attempts, worker, round, 2);
                                barrier.wait(); // Keep both outer dispatchers alive through emission.
                                nested
                            })
                        });
                        results.push((worker, round, enabled, result));
                    }
                    results
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>()
    });

    assert_eq!(attempts.load(Ordering::SeqCst), 2 * ROUNDS * 3);
    for (worker, round, enabled, result) in results {
        let (nested, logs) = result.unwrap_or_else(|_| {
            panic!("capture helper failed: worker={worker} round={round} enabled={enabled}")
        });
        if let Some(nested) = nested {
            assert!(nested.unwrap().1.is_empty(), "nested OFF capture leaked");
        }
        if enabled {
            let expected: Vec<_> = (0..3)
                .flat_map(|phase| {
                    ["info", "trace"].map(move |level| {
                        format!(
                            "message=capture-sync-{level} worker={worker} round={round} phase={phase}"
                        )
                    })
                })
                .collect();
            let observed: Vec<_> = logs
                .lines()
                .filter(|line| line.starts_with("message=capture-sync-"))
                .collect();
            assert_eq!(
                observed, expected,
                "ON capture: worker={worker} round={round}"
            );
        } else {
            assert!(
                logs.is_empty(),
                "OFF capture leaked: worker={worker} round={round}"
            );
        }
    }
}
