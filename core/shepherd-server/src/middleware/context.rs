use std::future::Future;
use std::sync::{Arc, Mutex};

use shepherd_core::model::{Actor as CoreActor, BrowserSession, SecretString};
use uuid::Uuid;

pub enum CookieAction {
    Set(SecretString),
    Clear,
}

pub struct RequestContextInner {
    pub request_id: Uuid,
    pub principal: Option<CoreActor>,
    /// Present only for cookie-authenticated requests.
    pub session: Option<(SecretString, BrowserSession)>,
    pub cookie_action: Mutex<Option<CookieAction>>,
}

#[derive(Clone)]
pub struct RequestContext(pub Arc<RequestContextInner>);

impl RequestContext {
    pub(crate) fn new(
        request_id: Uuid,
        principal: Option<CoreActor>,
        session: Option<(SecretString, BrowserSession)>,
    ) -> Self {
        Self(Arc::new(RequestContextInner {
            request_id,
            principal,
            session,
            cookie_action: Mutex::new(None),
        }))
    }
}

tokio::task_local! {
    static REQUEST_CONTEXT: RequestContext;
}

/// Runs a request future inside the given context's task-local scope.
pub(crate) async fn with_context<F: Future>(ctx: RequestContext, future: F) -> F::Output {
    REQUEST_CONTEXT.scope(ctx, future).await
}

/// Handlers run inside the gate's task-local scope (generated trait methods
/// cannot receive extractors, so the principal travels here).
pub fn current_context() -> RequestContext {
    REQUEST_CONTEXT.with(Clone::clone)
}

pub fn set_cookie_action(action: CookieAction) {
    let ctx = current_context();
    *ctx.0.cookie_action.lock().unwrap() = Some(action);
}
