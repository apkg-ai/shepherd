// Shared early-return helpers for the generated per-operation response enums;
// the enums share variant names, so one macro maps the plan/07 status table
// onto whichever enum the operation uses.

/// Domain error → the operation's response-enum variant for its plan/07 status.
macro_rules! domain_problem {
    ($response:ident, $err:expr) => {{
        let (status, body) = $crate::middleware::problem_for(&$err, $crate::convert::request_id());
        match status {
            axum::http::StatusCode::BAD_REQUEST => $response::BadRequest(body),
            axum::http::StatusCode::UNAUTHORIZED => $response::Unauthorized(body),
            axum::http::StatusCode::FORBIDDEN => $response::Forbidden(body),
            axum::http::StatusCode::NOT_FOUND => $response::NotFound(body),
            axum::http::StatusCode::CONFLICT => $response::Conflict(body),
            axum::http::StatusCode::UNPROCESSABLE_ENTITY => $response::UnprocessableEntity(body),
            axum::http::StatusCode::SERVICE_UNAVAILABLE => $response::ServiceUnavailable(body),
            // A status the operation's contract does not declare: re-coerce the
            // body so wire status and body agree instead of shipping a 500 that
            // claims another status.
            _ => $response::InternalServerError($crate::middleware::problem(
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                &body.detail,
                $crate::convert::request_id(),
            )),
        }
    }};
}

/// domain_problem! plus the 428 variant every command contract declares.
macro_rules! command_problem {
    ($response:ident, $err:expr) => {{
        let (status, body) = $crate::middleware::problem_for(&$err, $crate::convert::request_id());
        match status {
            axum::http::StatusCode::PRECONDITION_REQUIRED => $response::Status428(body),
            _ => $crate::convert::errors::domain_problem!($response, $err),
        }
    }};
}

/// command_problem! plus the 412 variant If-Match mutations declare.
macro_rules! revision_problem {
    ($response:ident, $err:expr) => {{
        let (status, body) = $crate::middleware::problem_for(&$err, $crate::convert::request_id());
        match status {
            axum::http::StatusCode::PRECONDITION_FAILED => $response::Status412(body),
            _ => $crate::convert::errors::command_problem!($response, $err),
        }
    }};
}

/// If-Match presence is the gate's 428; any non-quoted-decimal value is 400.
macro_rules! parse_revision {
    ($response:ident, $value:expr) => {{
        match $crate::middleware::parse_if_match(&$value) {
            Some(revision) => revision,
            None => {
                return $response::BadRequest($crate::middleware::problem(
                    axum::http::StatusCode::BAD_REQUEST,
                    "malformed_request",
                    "If-Match must be the quoted decimal resource revision",
                    $crate::convert::request_id(),
                ));
            }
        }
    }};
}

// Cookie-only session endpoints return 404 for bearer callers: a bearer
// client has no browser session resource.
macro_rules! require_session {
    ($response:ident) => {{
        let ctx = $crate::middleware::current_context();
        match &ctx.0.session {
            Some(session) => (session.0.clone(), session.1.actor.clone()),
            None => {
                return $response::NotFound($crate::middleware::problem(
                    axum::http::StatusCode::NOT_FOUND,
                    "not_found",
                    "no browser session for this credential",
                    $crate::convert::request_id(),
                ));
            }
        }
    }};
}

macro_rules! require_principal {
    ($response:ident) => {{
        match $crate::middleware::current_context().0.principal.clone() {
            Some(principal) => principal,
            None => {
                return $response::Unauthorized($crate::middleware::problem(
                    axum::http::StatusCode::UNAUTHORIZED,
                    "unauthenticated",
                    "authentication required",
                    $crate::convert::request_id(),
                ));
            }
        }
    }};
}

macro_rules! require_store {
    ($self:ident, $response:ident) => {{
        match $self.store() {
            Ok(store) => store,
            Err(err) => return $crate::convert::errors::domain_problem!($response, err),
        }
    }};
}

/// Route identifiers must parse as UUIDs; mismatches are 422 (plan/07).
macro_rules! parse_path_uuid {
    ($response:ident, $value:expr, $name:literal) => {{
        match $value.parse::<uuid::Uuid>() {
            Ok(id) => id,
            Err(_) => {
                return $response::UnprocessableEntity($crate::middleware::problem(
                    axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                    "validation_error",
                    concat!($name, " must be a UUID"),
                    $crate::convert::request_id(),
                ));
            }
        }
    }};
}

/// Idempotency-Key presence is the gate's 428; any non-UUID value is 400.
macro_rules! parse_idempotency_key {
    ($response:ident, $value:expr) => {{
        match $value.parse::<uuid::Uuid>() {
            Ok(key) => key,
            Err(_) => {
                return $response::BadRequest($crate::middleware::problem(
                    axum::http::StatusCode::BAD_REQUEST,
                    "malformed_request",
                    "Idempotency-Key must be a UUID",
                    $crate::convert::request_id(),
                ));
            }
        }
    }};
}

pub(crate) use {
    command_problem, domain_problem, parse_idempotency_key, parse_path_uuid, parse_revision,
    require_principal, require_session, require_store, revision_problem,
};
