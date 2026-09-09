use crate::ring::{LEVEL, enabled, now_ms, push, set_enabled, set_level};
use crate::{Level, LogLine};
use std::sync::atomic::Ordering;
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer};

/// Device capture uses its own filter, narrowed by the runtime severity threshold.
const DEVICE_FILTER_DEFAULT: &str = "info,halogen_webui=trace,halogen_server=info,halogen_download=info,halogen_rss=info,halogen_polling=info,halogen_local_runtime=info,hyper=warn,hyper_util=warn,sea_orm=warn,sqlx=warn,reqwest=warn,h2=warn,idb=warn";

/// Console filter, overridable through RUST_LOG; suppress hot Dioxus signal spans.
const CONSOLE_FILTER_DEFAULT: &str = "info,hyper_util=warn,dioxus_signals=warn";

/// Initialize the subscriber before application startup.
pub fn init() {
    // Seed the runtime gate with the profile default; ConfigProvider overrides
    // it with the persisted setting once the async config load completes.
    set_enabled(cfg!(debug_assertions) || cfg!(test));
    set_level(Level::Info);

    // Console verbosity must not override the independently controlled device ring.
    let device_filter = std::env::var("HALOGEN_DEVICE_LOG")
        .ok()
        .map(EnvFilter::new)
        .unwrap_or_else(|| EnvFilter::new(DEVICE_FILTER_DEFAULT));

    #[cfg(not(target_arch = "wasm32"))]
    let console = {
        let filter = EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| EnvFilter::new(CONSOLE_FILTER_DEFAULT));
        tracing_subscriber::fmt::layer().with_filter(filter)
    };

    #[cfg(target_arch = "wasm32")]
    let console = {
        // Workers have no Window; only the main thread can use the WASM console layer.
        web_sys::window().map(|_| {
            let filter = EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new(CONSOLE_FILTER_DEFAULT));
            let cfg = tracing_wasm::WASMLayerConfigBuilder::new()
                .set_max_level(tracing::Level::TRACE)
                .build();
            tracing_wasm::WASMLayer::new(cfg).with_filter(filter)
        })
    };

    // `try_init` (not `set_global_default`) so a double init in tests is a no-op
    // rather than a panic. Installed before launch → dioxus-logger stands down.
    let _ = tracing_subscriber::registry()
        .with(console)
        .with(DeviceLogLayer.with_filter(device_filter))
        .try_init();
}

/// Convert an event only when capture is enabled and its severity passes the runtime gate.
fn event_to_logline(event: &tracing::Event<'_>) -> Option<LogLine> {
    if !enabled() {
        return None;
    }
    let meta = event.metadata();
    let level = Level::from_tracing(meta.level());
    if (level as u8) > LEVEL.load(Ordering::Relaxed) {
        return None;
    }
    let mut visitor = MessageVisitor::default();
    event.record(&mut visitor);
    Some(LogLine {
        ts_ms: now_ms(),
        level,
        target: meta.target().to_string(),
        msg: visitor.finish(),
    })
}

/// `tracing` layer that records admitted events into the device-log ring.
pub(crate) struct DeviceLogLayer;

impl<S: tracing::Subscriber> Layer<S> for DeviceLogLayer {
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        if let Some(line) = event_to_logline(event) {
            push(line);
        }
    }
}

/// `tracing` layer that hands each admitted event's [`LogLine`] to a callback
/// instead of recording it locally. The sync Web Worker installs this (via
/// [`init_forwarding`]) to ship its logs to the main thread, which owns the single
/// ring + `halogen.logs` store. No console layer, no local ring, no persistence.
struct ForwardLayer<F> {
    forward: F,
}

impl<S, F> Layer<S> for ForwardLayer<F>
where
    S: tracing::Subscriber,
    F: Fn(LogLine) + 'static,
{
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        if let Some(line) = event_to_logline(event) {
            (self.forward)(line);
        }
    }
}

/// Forward admitted events without a console layer, local ring or persistence.
pub fn init_forwarding(forward: impl Fn(LogLine) + Send + Sync + 'static) {
    let device_filter = std::env::var("HALOGEN_DEVICE_LOG")
        .ok()
        .map(EnvFilter::new)
        .unwrap_or_else(|| EnvFilter::new(DEVICE_FILTER_DEFAULT));
    let _ = tracing_subscriber::registry()
        .with(ForwardLayer { forward }.with_filter(device_filter))
        .try_init();
}

/// Renders an event's `message` field plus any structured fields into a single
/// line (`message key=value …`).
#[derive(Default)]
struct MessageVisitor {
    msg: String,
    fields: String,
}

impl MessageVisitor {
    fn finish(self) -> String {
        if self.fields.is_empty() {
            self.msg
        } else {
            format!("{}{}", self.msg, self.fields)
        }
    }
}

impl tracing::field::Visit for MessageVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        use std::fmt::Write;
        if field.name() == "message" {
            let _ = write!(self.msg, "{value:?}");
        } else {
            let _ = write!(self.fields, " {}={:?}", field.name(), value);
        }
    }
}
