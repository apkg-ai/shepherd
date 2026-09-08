//! Tower middleware: CORS and stub rate-limit headers.
//!
//! The rate-limit headers satisfy the OpenAPI contract without enforcing
//! actual limits — shepherd is a local daemon.

use axum::http::{HeaderValue, Request, Response};
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use tower::{Layer, Service};
use tower_http::cors::CorsLayer;

/// Permissive CORS layer suitable for a localhost daemon.
pub fn cors_layer() -> CorsLayer {
    CorsLayer::permissive()
}

// ── Rate-limit header middleware ────────────────────────────────────────

/// Layer that injects stub `RateLimit-*` headers on every response.
#[derive(Clone)]
pub struct RateLimitHeaderLayer;

impl<S> Layer<S> for RateLimitHeaderLayer {
    type Service = RateLimitHeaderService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        RateLimitHeaderService { inner }
    }
}

/// Service that injects stub rate-limit headers.
#[derive(Clone)]
pub struct RateLimitHeaderService<S> {
    inner: S,
}

impl<S, ReqBody, ResBody> Service<Request<ReqBody>> for RateLimitHeaderService<S>
where
    S: Service<Request<ReqBody>, Response = Response<ResBody>> + Clone + Send + 'static,
    S::Future: Send,
    ReqBody: Send + 'static,
    ResBody: Send + 'static,
{
    type Response = Response<ResBody>;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<ReqBody>) -> Self::Future {
        let mut inner = self.inner.clone();
        Box::pin(async move {
            let mut response = inner.call(req).await?;
            let headers = response.headers_mut();
            headers.insert("RateLimit-Limit", HeaderValue::from_static("1000"));
            headers.insert("RateLimit-Remaining", HeaderValue::from_static("999"));
            headers.insert("RateLimit-Reset", HeaderValue::from_static("60"));
            Ok(response)
        })
    }
}
