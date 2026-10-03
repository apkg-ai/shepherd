use axum::http::{HeaderValue, Method, header};
use tower_http::cors::CorsLayer;

/// Transport allowlists, parsed once at startup; requests only compare.
pub struct AuthConfig {
    dev: bool,
    hosts: [String; 2],
    /// Exact origin strings as sent in CORS allow-origin headers.
    origin_values: Vec<String>,
    /// The same origins pre-parsed for request comparison.
    origins: Vec<url::Url>,
}

impl AuthConfig {
    pub fn new(port: u16, dev: bool) -> Self {
        let hosts = [format!("127.0.0.1:{port}"), format!("localhost:{port}")];
        let mut origin_values = vec![
            format!("http://127.0.0.1:{port}"),
            format!("http://localhost:{port}"),
        ];
        if dev {
            // The Vite dev origin, plan/12: explicit only.
            origin_values.push("http://localhost:5173".to_string());
        }
        let origins = origin_values
            .iter()
            .map(|origin| url::Url::parse(origin).expect("allowlist origins parse"))
            .collect();
        Self {
            dev,
            hosts,
            origin_values,
            origins,
        }
    }

    // Parse, never prefix-match (plan/12): scheme, host and port must all be
    // exact, which defeats tricks like http://localhost:7437.evil.com.
    pub(crate) fn origin_allowed(&self, origin: &str) -> bool {
        let Ok(parsed) = url::Url::parse(origin) else {
            return false;
        };
        self.origins.iter().any(|expected| {
            parsed.scheme() == expected.scheme()
                && parsed.host_str() == expected.host_str()
                && parsed.port_or_known_default() == expected.port_or_known_default()
                && parsed.username().is_empty()
                && parsed.password().is_none()
                && parsed.path() == "/"
                && parsed.query().is_none()
                && parsed.fragment().is_none()
        })
    }

    // RFC 9110: host names compare case-insensitively (the Origin path
    // already lowercases through Url::parse).
    pub(crate) fn host_allowed(&self, host: &str) -> bool {
        self.hosts
            .iter()
            .any(|allowed| allowed.eq_ignore_ascii_case(host))
    }
}

/// Exact-allowlist CORS; credentialed cross-origin only for the dev origin.
pub fn cors_layer(config: &AuthConfig) -> CorsLayer {
    let origins: Vec<HeaderValue> = config
        .origin_values
        .iter()
        .map(|origin| HeaderValue::from_str(origin).expect("origin is a valid header value"))
        .collect();
    CorsLayer::new()
        .allow_origin(origins)
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
        ])
        .allow_headers([
            header::CONTENT_TYPE,
            header::AUTHORIZATION,
            header::IF_MATCH,
            "x-csrf-token".parse().unwrap(),
            "idempotency-key".parse().unwrap(),
        ])
        .allow_credentials(config.dev)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_allowlist_is_case_insensitive_and_exact() {
        let config = AuthConfig::new(7437, false);
        assert!(config.host_allowed("127.0.0.1:7437"));
        assert!(config.host_allowed("localhost:7437"));
        assert!(config.host_allowed("LocalHost:7437"));
        assert!(config.host_allowed("LOCALHOST:7437"));
        assert!(!config.host_allowed("localhost:7438"));
        assert!(!config.host_allowed("localhost"));
        assert!(!config.host_allowed("evil.example.com:7437"));
    }

    #[test]
    fn origin_allowlist_is_parsed_exactly() {
        let config = AuthConfig::new(7437, false);
        assert!(config.origin_allowed("http://127.0.0.1:7437"));
        assert!(config.origin_allowed("http://localhost:7437"));
        assert!(!config.origin_allowed("http://localhost:5173"));
        assert!(!config.origin_allowed("http://localhost:7437.evil.com"));
        assert!(!config.origin_allowed("https://localhost:7437"));
        assert!(!config.origin_allowed("http://user@localhost:7437"));
        assert!(!config.origin_allowed("http://user:pass@localhost:7437"));
        assert!(!config.origin_allowed("http://:pass@localhost:7437"));
        assert!(!config.origin_allowed("http://localhost:7437?x=1"));
        assert!(!config.origin_allowed("http://localhost:7437#frag"));
        let dev = AuthConfig::new(7437, true);
        assert!(dev.origin_allowed("http://localhost:5173"));
    }
}
