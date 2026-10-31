// Router 挂载适配（错误擦除）：把 SecurityBodyService 的错误擦除为
// Infallible，使其可直接用于 axum Router::layer（Router 要求
// Error: Into<Infallible>）。SecurityError 经 IntoResponse 映射为
// 403/413/500 响应，语义与直接使用一致。
use crate::layer::{SecurityBodyLayer, SecurityBodyService};
use axum::body::Body;
use axum::http::Request;
use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

#[derive(Clone)]
pub struct SecurityBodyCompatLayer {
    inner: SecurityBodyLayer,
}

impl SecurityBodyCompatLayer {
    pub fn new() -> Self {
        Self {
            inner: SecurityBodyLayer::new(),
        }
    }

    pub fn strict(mut self) -> Self {
        self.inner = self.inner.strict();
        self
    }

    pub fn body_limit(mut self, limit: usize) -> Self {
        self.inner = self.inner.body_limit(limit);
        self
    }
}

impl Default for SecurityBodyCompatLayer {
    fn default() -> Self {
        Self::new()
    }
}

impl<S> tower::Layer<S> for SecurityBodyCompatLayer {
    type Service = SecurityBodyCompat<S>;

    fn layer(&self, inner: S) -> Self::Service {
        SecurityBodyCompat {
            inner: self.inner.layer(inner),
        }
    }
}

#[derive(Clone)]
pub struct SecurityBodyCompat<S> {
    inner: SecurityBodyService<S>,
}

impl<S> tower::Service<Request<Body>> for SecurityBodyCompat<S>
where
    S: tower::Service<Request<Body>> + Clone + Send + 'static,
    S::Future: Send + 'static,
    S::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
    S::Response: From<axum::response::Response> + Send,
{
    type Response = S::Response;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        // inner readiness 错误在挂载场景不可达（Route 为 Infallible）；即便
        // 发生也不 panic，留到 call() 阶段经 SecurityError 映射为 500 响应。
        match self.inner.poll_ready(cx) {
            Poll::Ready(Ok(())) => Poll::Ready(Ok(())),
            Poll::Ready(Err(e)) => {
                tracing::error!(error = %e, "security middleware not ready");
                Poll::Ready(Ok(()))
            }
            Poll::Pending => Poll::Pending,
        }
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        let mut inner = self.inner.clone();
        Box::pin(async move {
            match inner.call(req).await {
                Ok(resp) => Ok(resp),
                Err(e) => {
                    use axum::response::IntoResponse;
                    let resp: axum::response::Response = e.into_response();
                    Ok(resp.into())
                }
            }
        })
    }
}
