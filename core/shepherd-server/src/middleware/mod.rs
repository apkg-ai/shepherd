mod config;
mod context;
mod gate;
mod problem;

pub use config::{AuthConfig, cors_layer};
pub use context::{CookieAction, current_context, set_cookie_action};
pub use gate::gate;
pub use problem::{problem, problem_for, problem_shaper};

pub const SESSION_COOKIE: &str = "shepherd_session";
pub(crate) const API_PREFIX: &str = "/api/v1";
pub(crate) const SPEC_PATH: &str = "/api/v1/openapi.yaml";
pub(crate) const LOGIN_PATH: &str = "/api/v1/session";

/// Headers whose values must never appear in diagnostics (plan/12).
pub const REDACTED_HEADERS: [&str; 5] = [
    "authorization",
    "cookie",
    "set-cookie",
    "x-csrf-token",
    "x-lease-token",
];

pub fn redact_header(name: &str, value: &str) -> String {
    if REDACTED_HEADERS.contains(&name.to_ascii_lowercase().as_str()) {
        "[redacted]".to_string()
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_headers_are_redacted() {
        for name in REDACTED_HEADERS {
            assert_eq!(redact_header(name, "secret-value"), "[redacted]");
        }
        assert_eq!(redact_header("Authorization", "Bearer x"), "[redacted]");
        assert_eq!(
            redact_header("content-type", "application/json"),
            "application/json"
        );
    }
}
