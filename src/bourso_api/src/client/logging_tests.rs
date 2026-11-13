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
    let result = tracing::subscriber::with_default(subscriber, || {
        tracing::trace!("capture-probe");
        action()
    });
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
