//! N4：request span 必须记录 trace_id 字段。
//!
//! 放在独立集成测试进程：tracing callsite interest 是进程级缓存，与
//! lib.rs 内其他 subscriber 测试并行时会互相关闭 callsite，导致空输出。

use tower::{Layer, ServiceExt};

#[derive(Clone)]
struct CaptureWriter(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for CaptureWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn span_records_trace_id() {
    let buf = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let writer = CaptureWriter(std::sync::Arc::clone(&buf));
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .with_span_events(tracing_subscriber::fmt::format::FmtSpan::FULL)
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);

    let svc = ecat_tracing::TracingLayer::new("svc").layer(tower::service_fn(
        |_req: http::Request<()>| async move {
            Ok::<_, std::convert::Infallible>(http::Response::new(()))
        },
    ));
    svc.oneshot(
        http::Request::builder()
            .header(ecat_metadata::TRACE_ID, "abc123")
            .body(())
            .unwrap(),
    )
    .await
    .unwrap();

    let out = String::from_utf8(buf.lock().unwrap_or_else(|e| e.into_inner()).clone()).unwrap();
    assert!(
        out.contains("trace_id=\"abc123\""),
        "span must record trace_id, got: {out}"
    );
}
