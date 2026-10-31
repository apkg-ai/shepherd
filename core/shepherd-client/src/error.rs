use crate::generated::types::Problem;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Server-produced problem+json response, any non-2xx status.
    #[error("{} {}: {}", .0.status, .0.code, .0.detail)]
    Api(Box<Problem>),
    /// Request failed in transit; the server may or may not have executed it.
    #[error("transport failure after {attempts} attempt(s); outcome unknown: {message}")]
    Transport {
        message: String,
        attempts: u32,
        #[source]
        source: Option<reqwest::Error>,
    },
    /// Local pre-send rejection (bad URL, token file problems); no request was made.
    #[error("client usage error: {message}")]
    Usage { message: String },
    /// Response body did not match the contract.
    #[error("response decode failure (status {status}): {message}")]
    Decode { status: u16, message: String },
}

impl Error {
    pub fn code(&self) -> &str {
        match self {
            Error::Api(problem) => &problem.code,
            Error::Transport { .. } | Error::Decode { .. } => "client_transport",
            Error::Usage { .. } => "client_usage",
        }
    }

    pub fn status(&self) -> Option<u16> {
        match self {
            Error::Api(problem) => u16::try_from(problem.status).ok(),
            Error::Decode { status, .. } => Some(*status),
            _ => None,
        }
    }

    /// Server-assigned request id; never invented locally.
    pub fn request_id(&self) -> Option<uuid::Uuid> {
        match self {
            Error::Api(problem) => Some(problem.request_id),
            _ => None,
        }
    }

    /// True when the request may have executed despite the failure; retry with the same idempotency key.
    pub fn unknown_outcome(&self) -> bool {
        matches!(self, Error::Transport { .. })
    }

    pub fn problem(&self) -> Option<&Problem> {
        match self {
            Error::Api(problem) => Some(problem.as_ref()),
            _ => None,
        }
    }
}
