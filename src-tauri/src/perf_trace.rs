//! Compile-time gated structured performance trace.
//!
//! A normal build has no trace writer and the disabled macros discard their
//! arguments before type checking/code generation, so JSON payloads, clocks
//! and field expressions are not evaluated.  A `perf-trace` build still keeps
//! the sink dormant unless `TALMINAL_PERF_TRACE=1` *and*
//! `TALMINAL_PERF_LOG` is non-empty.

#[cfg(feature = "perf-trace")]
mod enabled {
    use std::cell::RefCell;
    use std::ffi::OsStr;
    use std::fs::{File, OpenOptions};
    use std::io::Write as _;
    use std::path::PathBuf;
    use std::sync::{mpsc, LazyLock};
    use std::time::{Instant, SystemTime, UNIX_EPOCH};

    use serde::Deserialize;
    use serde_json::{json, Value};

    #[derive(Clone)]
    pub struct TraceContext {
        trace_id: String,
        kind: String,
        client_started_ms: Option<f64>,
        started: Instant,
    }

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct FrontendMark {
        pub event: String,
        pub ts_epoch_ms: f64,
        #[serde(default)]
        pub fields: Value,
    }

    thread_local! {
        static CURRENT: RefCell<Option<TraceContext>> = const { RefCell::new(None) };
    }

    fn configured_path(flag: Option<&OsStr>, log: Option<&OsStr>) -> Option<PathBuf> {
        if flag != Some(OsStr::new("1")) {
            return None;
        }
        let log = log.filter(|value| !value.is_empty())?;
        Some(PathBuf::from(log))
    }

    static TRACE_SENDER: LazyLock<Option<mpsc::Sender<Value>>> = LazyLock::new(|| {
        let flag = std::env::var_os("TALMINAL_PERF_TRACE");
        let log = std::env::var_os("TALMINAL_PERF_LOG");
        let path = configured_path(flag.as_deref(), log.as_deref())?;
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent).ok()?;
        }
        let mut file: File = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .ok()?;
        let (sender, receiver) = mpsc::channel::<Value>();
        std::thread::Builder::new()
            .name("perf-trace-writer".into())
            .spawn(move || {
                while let Ok(value) = receiver.recv() {
                    if serde_json::to_writer(&mut file, &value).is_ok() {
                        let _ = file.write_all(b"\n");
                        // This flush is deliberately off the measured thread.
                        let _ = file.flush();
                    }
                }
            })
            .ok()?;
        Some(sender)
    });

    pub fn enabled() -> bool {
        TRACE_SENDER.is_some()
    }

    fn epoch_ms() -> f64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs_f64() * 1_000.0)
            .unwrap_or(0.0)
    }

    fn write_line(value: Value) {
        if let Some(sender) = TRACE_SENDER.as_ref() {
            let _ = sender.send(value);
        }
    }

    pub fn new_context(
        trace_id: String,
        kind: impl Into<String>,
        client_started_ms: Option<f64>,
    ) -> TraceContext {
        TraceContext {
            trace_id,
            kind: kind.into(),
            client_started_ms,
            started: Instant::now(),
        }
    }

    pub fn with_context<T>(context: Option<TraceContext>, f: impl FnOnce() -> T) -> T {
        if !enabled() || context.is_none() {
            return f();
        }
        let previous = CURRENT.with(|slot| slot.replace(context));
        let result = f();
        CURRENT.with(|slot| {
            slot.replace(previous);
        });
        result
    }

    pub fn capture_context() -> Option<TraceContext> {
        CURRENT.with(|slot| slot.borrow().clone())
    }

    pub fn current_trace_id() -> Option<String> {
        CURRENT.with(|slot| slot.borrow().as_ref().map(|ctx| ctx.trace_id.clone()))
    }

    pub fn mark(event: &str, fields: Value) {
        if !enabled() {
            return;
        }
        let context = CURRENT.with(|slot| slot.borrow().clone());
        let Some(context) = context else {
            return;
        };
        write_line(json!({
            "schema": 1,
            "source": "rust",
            "trace_id": context.trace_id,
            "kind": context.kind,
            "event": event,
            "ts_epoch_ms": epoch_ms(),
            "rust_elapsed_ms": context.started.elapsed().as_secs_f64() * 1_000.0,
            "client_started_ms": context.client_started_ms,
            "thread": format!("{:?}", std::thread::current().id()),
            "fields": fields,
        }));
    }

    pub fn mark_for(trace_id: Option<&str>, kind: &str, event: &str, fields: Value) {
        if !enabled() {
            return;
        }
        let Some(trace_id) = trace_id else {
            return;
        };
        write_line(json!({
            "schema": 1,
            "source": "rust",
            "trace_id": trace_id,
            "kind": kind,
            "event": event,
            "ts_epoch_ms": epoch_ms(),
            "thread": format!("{:?}", std::thread::current().id()),
            "fields": fields,
        }));
    }

    pub fn mark_background(event: &str, fields: Value) {
        if !enabled() {
            return;
        }
        write_line(json!({
            "schema": 1,
            "source": "rust",
            "trace_id": "background",
            "kind": "background",
            "event": event,
            "ts_epoch_ms": epoch_ms(),
            "thread": format!("{:?}", std::thread::current().id()),
            "fields": fields,
        }));
    }

    pub fn write_frontend_batch(
        trace_id: &str,
        kind: &str,
        client_started_ms: f64,
        marks: Vec<FrontendMark>,
        final_batch: bool,
    ) {
        if !enabled() {
            return;
        }
        for mark in marks {
            write_line(json!({
                "schema": 1,
                "source": "frontend",
                "trace_id": trace_id,
                "kind": kind,
                "event": mark.event,
                "ts_epoch_ms": mark.ts_epoch_ms,
                "client_started_ms": client_started_ms,
                "fields": mark.fields,
            }));
        }
        if final_batch {
            write_line(json!({
                "schema": 1,
                "source": "frontend",
                "trace_id": trace_id,
                "kind": kind,
                "event": "frontend.trace_flushed_final",
                "ts_epoch_ms": epoch_ms(),
                "client_started_ms": client_started_ms,
                "fields": {},
            }));
        }
    }

    #[cfg(test)]
    mod tests {
        use super::configured_path;
        use std::ffi::OsStr;

        #[test]
        fn sink_requires_exact_enable_flag_and_nonempty_path() {
            assert!(configured_path(None, Some(OsStr::new("trace.jsonl"))).is_none());
            assert!(
                configured_path(Some(OsStr::new("true")), Some(OsStr::new("trace.jsonl")))
                    .is_none()
            );
            assert!(configured_path(Some(OsStr::new("1")), None).is_none());
            assert!(configured_path(Some(OsStr::new("1")), Some(OsStr::new(""))).is_none());
            assert_eq!(
                configured_path(Some(OsStr::new("1")), Some(OsStr::new("trace.jsonl"))),
                Some("trace.jsonl".into())
            );
        }
    }
}

#[cfg(feature = "perf-trace")]
pub use enabled::{
    capture_context, current_trace_id, enabled, mark, mark_background, mark_for, new_context,
    with_context, write_frontend_batch, FrontendMark,
};

#[cfg(feature = "perf-trace")]
#[macro_export]
macro_rules! perf_mark {
    ($event:expr, $fields:expr $(,)?) => {{
        $crate::perf_trace::mark($event, $fields)
    }};
}

#[cfg(not(feature = "perf-trace"))]
#[macro_export]
macro_rules! perf_mark {
    ($($discarded:tt)*) => {{}};
}

#[cfg(feature = "perf-trace")]
#[macro_export]
macro_rules! perf_mark_for {
    ($trace_id:expr, $kind:expr, $event:expr, $fields:expr $(,)?) => {{
        $crate::perf_trace::mark_for($trace_id, $kind, $event, $fields)
    }};
}

#[cfg(not(feature = "perf-trace"))]
#[macro_export]
macro_rules! perf_mark_for {
    ($($discarded:tt)*) => {{}};
}

#[cfg(feature = "perf-trace")]
#[macro_export]
macro_rules! perf_mark_background {
    ($event:expr, $fields:expr $(,)?) => {{
        $crate::perf_trace::mark_background($event, $fields)
    }};
}

#[cfg(not(feature = "perf-trace"))]
#[macro_export]
macro_rules! perf_mark_background {
    ($($discarded:tt)*) => {{}};
}

#[cfg(feature = "perf-trace")]
#[macro_export]
macro_rules! perf_with_context {
    ($context:expr, $body:block) => {{
        $crate::perf_trace::with_context($context, || $body)
    }};
}

#[cfg(not(feature = "perf-trace"))]
#[macro_export]
macro_rules! perf_with_context {
    ($discarded:expr, $body:block) => {{
        $body
    }};
}

/// Gate trace-only mutations/counter updates that cannot carry `#[cfg]`
/// directly because Rust attributes on expression statements are unstable.
#[cfg(feature = "perf-trace")]
#[macro_export]
macro_rules! perf_only {
    ($body:block) => {{
        $body
    }};
}

#[cfg(not(feature = "perf-trace"))]
#[macro_export]
macro_rules! perf_only {
    ($discarded:block) => {{}};
}

#[cfg(all(test, not(feature = "perf-trace")))]
mod disabled_tests {
    use std::cell::Cell;

    #[test]
    fn disabled_macros_do_not_evaluate_arguments_or_context() {
        let evaluated = Cell::new(false);
        crate::perf_mark!(
            {
                evaluated.set(true);
                "event"
            },
            {
                evaluated.set(true);
                serde_json::json!({})
            },
        );
        crate::perf_mark_for!(
            {
                evaluated.set(true);
                None::<&str>
            },
            "kind",
            "event",
            serde_json::json!({}),
        );
        crate::perf_mark_background!(
            {
                evaluated.set(true);
                "event"
            },
            serde_json::json!({}),
        );
        crate::perf_only!({
            evaluated.set(true);
        });
        let answer = crate::perf_with_context!(
            {
                evaluated.set(true);
                ()
            },
            { 40 + 2 }
        );
        assert!(!evaluated.get());
        assert_eq!(answer, 42);
    }
}
