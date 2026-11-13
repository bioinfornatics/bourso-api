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

fn capture<T>(enabled: bool, action: impl FnOnce() -> T) -> (T, String) {
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


use std::sync::atomic::{AtomicUsize, Ordering};
fn emit(attempts: &AtomicUsize, phase: usize) {
    attempts.fetch_add(1, Ordering::SeqCst);
    tracing::debug!("cold-target phase={phase}");
}
fn main() {
    let root = std::thread::current().id();
    let attempts = AtomicUsize::new(0);
    let runtime = tokio::runtime::Builder::new_current_thread().build().unwrap();
    let (_, logs) = capture(true, || runtime.block_on(async {
        println!("root={root:?} runtime={:?} max={:?}", std::thread::current().id(), tracing::level_filters::LevelFilter::current());
        tracing::debug!("control-before");
        std::thread::scope(|scope| {
            scope.spawn(|| {
                tracing::dispatcher::get_default(|d| println!("cold-thread={:?} no_subscriber={} max={:?}", std::thread::current().id(), d.is::<tracing::subscriber::NoSubscriber>(), tracing::level_filters::LevelFilter::current()));
                emit(&attempts, 1);
            }).join().unwrap();
        });
        tokio::task::yield_now().await;
        assert_eq!(root, std::thread::current().id());
        emit(&attempts, 2);
        tracing::debug!("control-after");
        println!("pre-recovery attempts={} root={:?} max={:?}", attempts.load(Ordering::SeqCst), std::thread::current().id(), tracing::level_filters::LevelFilter::current());
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
        tracing_core::callsite::rebuild_interest_cache();
        emit(&attempts, 3);
    }));
    println!("CAPTURED\n{logs}END_CAPTURED");
    assert!(logs.contains("control-before") && logs.contains("control-after"));
    assert_eq!(logs.matches("cold-target phase=3").count(), 1, "recovery must capture target");
    assert_eq!(logs.matches("cold-target phase=1").count(), 0, "uncaptured thread must not emit to ON");
    assert_eq!(logs.matches("cold-target phase=2").count(), 1, "ON delivery after cold registration must capture once");
}
