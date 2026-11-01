use std::fmt;
use std::path::Path;
use std::time::Duration;

use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderValue};
use reqwest::{Method, StatusCode, Url};
use serde::Serialize;
use serde::de::DeserializeOwned;
use uuid::Uuid;

use crate::error::Error;
use crate::generated::types::Problem;

pub const DEFAULT_BASE_URL: &str = "http://127.0.0.1:7437";

// Mirrors ui/src/api/client.ts: fixed backoff, retry only transport errors and storage_busy.
const RETRY_DELAYS_MS: [u64; 3] = [250, 500, 1000];
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Credential wrapper whose Debug output never contains the value.
#[derive(Clone)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: String) -> Self {
        Self(value)
    }

    pub(crate) fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(redacted)")
    }
}

/// Caller-supplied protocol headers; reused verbatim on every retry attempt.
#[derive(Debug, Clone, Default)]
pub struct CallOptions {
    pub idempotency_key: Option<Uuid>,
    pub if_match: Option<u64>,
    pub lease_token: Option<Secret>,
}

impl CallOptions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn idempotency_key(mut self, key: Uuid) -> Self {
        self.idempotency_key = Some(key);
        self
    }

    pub fn if_match(mut self, revision: u64) -> Self {
        self.if_match = Some(revision);
        self
    }

    pub fn lease_token(mut self, token: Secret) -> Self {
        self.lease_token = Some(token);
        self
    }
}

/// Loads an agent/owner credential file, refusing group/world-accessible modes.
pub fn load_token_file(path: &Path) -> Result<Secret, Error> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let metadata = std::fs::metadata(path).map_err(|err| Error::Usage {
            message: format!("token file {}: {err}", path.display()),
        })?;
        let mode = metadata.permissions().mode();
        if mode & 0o077 != 0 {
            return Err(Error::Usage {
                message: format!(
                    "token file {} is group/world accessible (mode {:03o}); expected 0600",
                    path.display(),
                    mode & 0o777
                ),
            });
        }
    }
    let raw = std::fs::read_to_string(path).map_err(|err| Error::Usage {
        message: format!("token file {}: {err}", path.display()),
    })?;
    let token = raw.trim_end();
    if token.is_empty() || token.chars().any(char::is_whitespace) {
        return Err(Error::Usage {
            message: format!(
                "token file {} must contain a single token on one line",
                path.display()
            ),
        });
    }
    Ok(Secret::new(token.to_owned()))
}

#[derive(Clone)]
pub(crate) struct Transport {
    base: Url,
    auth: HeaderValue,
    http: reqwest::Client,
}

impl fmt::Debug for Transport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Transport")
            .field("base", &self.base.as_str())
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum OpKind {
    Read,
    Mutation,
}

impl Transport {
    pub(crate) fn new(base_url: &str, token: Secret) -> Result<Self, Error> {
        let base = Url::parse(base_url).map_err(|err| usage(format!("base URL: {err}")))?;
        if base.scheme() != "http" {
            return Err(usage(
                "base URL must use plain http on the loopback interface",
            ));
        }
        if !matches!(base.host_str(), Some("127.0.0.1") | Some("localhost")) {
            return Err(usage(
                "base URL host must be 127.0.0.1 or localhost; refusing non-loopback target",
            ));
        }
        if !base.username().is_empty() || base.password().is_some() {
            return Err(usage("base URL must not contain credentials"));
        }
        if !matches!(base.path(), "" | "/") || base.query().is_some() || base.fragment().is_some() {
            return Err(usage("base URL must not contain a path, query or fragment"));
        }
        let auth = sensitive_header(&format!("Bearer {}", token.expose()))
            .map_err(|_| usage("token contains characters not valid in a header"))?;
        // Tokens must never leave the loopback service: no proxies, no redirects.
        let http = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|err| usage(format!("failed to build HTTP client: {err}")))?;
        Ok(Self { base, auth, http })
    }

    fn url(&self, path: &str, query: &[(&'static str, String)]) -> Url {
        let mut url = self.base.clone();
        url.set_path(path);
        if !query.is_empty() {
            let mut pairs = url.query_pairs_mut();
            for (name, value) in query {
                pairs.append_pair(name, value);
            }
        }
        url
    }

    fn apply_options(
        &self,
        mut request: reqwest::RequestBuilder,
        opts: &CallOptions,
    ) -> Result<reqwest::RequestBuilder, Error> {
        if let Some(key) = opts.idempotency_key {
            request = request.header("Idempotency-Key", key.to_string());
        }
        if let Some(revision) = opts.if_match {
            request = request.header("If-Match", format!("\"{revision}\""));
        }
        if let Some(lease) = &opts.lease_token {
            let value = sensitive_header(lease.expose())
                .map_err(|_| usage("lease token contains characters not valid in a header"))?;
            request = request.header("X-Lease-Token", value);
        }
        Ok(request)
    }

    pub(crate) async fn request<T, B>(
        &self,
        method: Method,
        path: &str,
        query: &[(&'static str, String)],
        body: Option<&B>,
        opts: &CallOptions,
        kind: OpKind,
    ) -> Result<T, Error>
    where
        T: DeserializeOwned,
        B: Serialize + ?Sized,
    {
        // Retry only when a replay is safe: reads, or mutations the caller keyed.
        let retry_allowed = kind == OpKind::Read || opts.idempotency_key.is_some();
        let url = self.url(path, query);
        let mut attempt: u32 = 0;
        loop {
            attempt += 1;
            let mut request = self
                .http
                .request(method.clone(), url.clone())
                .timeout(REQUEST_TIMEOUT)
                .header(AUTHORIZATION, self.auth.clone());
            request = self.apply_options(request, opts)?;
            if let Some(body) = body {
                request = request.json(body);
            }

            let can_retry = retry_allowed && (attempt as usize) <= RETRY_DELAYS_MS.len();
            match request.send().await {
                Ok(response) => {
                    let status = response.status();
                    let bytes = match response.bytes().await {
                        Ok(bytes) => bytes,
                        Err(err) => {
                            if can_retry {
                                backoff(attempt, path).await;
                                continue;
                            }
                            return Err(transport_failure(err, attempt));
                        }
                    };
                    if status.is_success() {
                        tracing::debug!(%method, path, status = status.as_u16(), attempt, "request succeeded");
                        return serde_json::from_slice(&bytes).map_err(|err| Error::Decode {
                            status: status.as_u16(),
                            message: err.to_string(),
                        });
                    }
                    let problem = parse_problem(status, &bytes)?;
                    tracing::debug!(%method, path, status = status.as_u16(), code = %problem.code, attempt, "request rejected");
                    if problem.code == "storage_busy" && can_retry {
                        backoff(attempt, path).await;
                        continue;
                    }
                    return Err(Error::Api(Box::new(problem)));
                }
                Err(err) => {
                    if can_retry {
                        tracing::debug!(%method, path, attempt, "transport failure; retrying");
                        backoff(attempt, path).await;
                        continue;
                    }
                    return Err(transport_failure(err, attempt));
                }
            }
        }
    }

    /// Opens the SSE response for getEvents; connection retry follows read semantics.
    pub(crate) async fn start_events(
        &self,
        path: &str,
        after: Option<&str>,
        last_event_id: Option<&str>,
    ) -> Result<reqwest::Response, Error> {
        let after_query = after.map(|after| ("after", after.to_owned()));
        let url = self.url(path, after_query.as_slice());
        let mut attempt: u32 = 0;
        loop {
            attempt += 1;
            let mut request = self
                .http
                .get(url.clone())
                .header(AUTHORIZATION, self.auth.clone())
                .header(ACCEPT, "text/event-stream");
            if let Some(id) = last_event_id {
                request = request.header("Last-Event-ID", id);
            }

            let can_retry = (attempt as usize) <= RETRY_DELAYS_MS.len();
            match request.send().await {
                Ok(response) if response.status().is_success() => {
                    tracing::debug!(path, attempt, "event stream connected");
                    return Ok(response);
                }
                Ok(response) => {
                    let status = response.status();
                    let bytes = response.bytes().await.unwrap_or_default();
                    let problem = parse_problem(status, &bytes)?;
                    if problem.code == "storage_busy" && can_retry {
                        backoff(attempt, path).await;
                        continue;
                    }
                    return Err(Error::Api(Box::new(problem)));
                }
                Err(err) => {
                    if can_retry {
                        backoff(attempt, path).await;
                        continue;
                    }
                    return Err(transport_failure(err, attempt));
                }
            }
        }
    }
}

fn usage(message: impl Into<String>) -> Error {
    Error::Usage {
        message: message.into(),
    }
}

fn sensitive_header(value: &str) -> Result<HeaderValue, reqwest::header::InvalidHeaderValue> {
    let mut header = HeaderValue::from_str(value)?;
    header.set_sensitive(true);
    Ok(header)
}

fn transport_failure(err: reqwest::Error, attempts: u32) -> Error {
    Error::Transport {
        // reqwest::Error never carries header values; the URL is loopback-only.
        message: err.to_string(),
        attempts,
        source: Some(err),
    }
}

fn parse_problem(status: StatusCode, bytes: &[u8]) -> Result<Problem, Error> {
    serde_json::from_slice(bytes).map_err(|err| Error::Decode {
        status: status.as_u16(),
        message: format!("expected problem+json error body: {err}"),
    })
}

async fn backoff(attempt: u32, path: &str) {
    let delay = RETRY_DELAYS_MS[(attempt as usize - 1).min(RETRY_DELAYS_MS.len() - 1)];
    tracing::debug!(path, attempt, delay_ms = delay, "retrying");
    tokio::time::sleep(Duration::from_millis(delay)).await;
}
