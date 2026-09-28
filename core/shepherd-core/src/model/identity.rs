use std::fmt;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::error::DomainError;
use crate::model::{Actor, ActorKind};

pub const TOKEN_BYTES: usize = 32;
/// Fixed browser-session lifetime; no sliding extension (plan/12).
pub const SESSION_TTL_SECONDS: i64 = 43_200;
pub const IDEMPOTENCY_TTL_DAYS: i64 = 7;

/// Secret carrier: Debug/Display never print the value (plan/12: no secrets in logs).
#[derive(Clone)]
pub struct SecretString(String);

impl SecretString {
    pub fn new(value: String) -> Self {
        Self(value)
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[redacted]")
    }
}

impl fmt::Display for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[redacted]")
    }
}

pub fn generate_token() -> SecretString {
    let mut bytes = [0u8; TOKEN_BYTES];
    getrandom::fill(&mut bytes).expect("system RNG is available");
    SecretString(URL_SAFE_NO_PAD.encode(bytes))
}

pub fn token_digest(token: &SecretString) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(token.0.as_bytes()))
}

/// Both sides are SHA-256 digests, so equality leaks nothing invertible; the
/// comparison itself is still constant-time (plan/12).
pub fn digest_matches(computed: &str, stored: &str) -> bool {
    computed.as_bytes().ct_eq(stored.as_bytes()).into()
}

/// Replay AAD component. serde_json's default map is key-ordered, so the body
/// serialization is already canonical.
pub fn canonical_request_hash(method: &str, path: &str, body: &serde_json::Value) -> String {
    let mut hasher = Sha256::new();
    hasher.update(method.as_bytes());
    hasher.update(b"\n");
    hasher.update(path.as_bytes());
    hasher.update(b"\n");
    hasher.update(body.to_string().as_bytes());
    URL_SAFE_NO_PAD.encode(hasher.finalize())
}

/// One variant per row of the plan/12 capability matrix, in table order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    ReadProjectResources,
    AdministerProject,
    CreateWorkItems,
    EditDescriptions,
    LowerRequirements,
    ResolveWork,
    AddDependency,
    RemoveDependencyOrUnblock,
    BlockWithReason,
    AuthorDocuments,
    ClaimWork,
    ReviewHumanPolicy,
    ReviewAgentPolicy,
    WithdrawSubmission,
    ManageCredentials,
}

impl Capability {
    pub const ALL: [Capability; 15] = [
        Capability::ReadProjectResources,
        Capability::AdministerProject,
        Capability::CreateWorkItems,
        Capability::EditDescriptions,
        Capability::LowerRequirements,
        Capability::ResolveWork,
        Capability::AddDependency,
        Capability::RemoveDependencyOrUnblock,
        Capability::BlockWithReason,
        Capability::AuthorDocuments,
        Capability::ClaimWork,
        Capability::ReviewHumanPolicy,
        Capability::ReviewAgentPolicy,
        Capability::WithdrawSubmission,
        Capability::ManageCredentials,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Capability::ReadProjectResources => "read-project-resources",
            Capability::AdministerProject => "administer-project",
            Capability::CreateWorkItems => "create-work-items",
            Capability::EditDescriptions => "edit-descriptions",
            Capability::LowerRequirements => "lower-requirements",
            Capability::ResolveWork => "resolve-work",
            Capability::RemoveDependencyOrUnblock => "remove-dependency-or-unblock",
            Capability::AddDependency => "add-dependency",
            Capability::BlockWithReason => "block-with-reason",
            Capability::AuthorDocuments => "author-documents",
            Capability::ClaimWork => "claim-work",
            Capability::ReviewHumanPolicy => "review-human-policy",
            Capability::ReviewAgentPolicy => "review-agent-policy",
            Capability::WithdrawSubmission => "withdraw-submission",
            Capability::ManageCredentials => "manage-credentials",
        }
    }
}

/// The plan/12 matrix, base owner/agent column only. Conditional qualifiers
/// ("idle only", "own claim only", proposal gates) stay at the command sites.
pub fn base_allow(kind: ActorKind, capability: Capability) -> bool {
    let owner = kind == ActorKind::Human;
    match capability {
        Capability::ReadProjectResources => true,
        Capability::AdministerProject => owner,
        Capability::CreateWorkItems => true,
        Capability::EditDescriptions => true,
        Capability::LowerRequirements => owner,
        Capability::ResolveWork => owner,
        Capability::AddDependency => true,
        Capability::RemoveDependencyOrUnblock => owner,
        Capability::BlockWithReason => true,
        Capability::AuthorDocuments => true,
        Capability::ClaimWork => true,
        Capability::ReviewHumanPolicy => owner,
        // Owner may change policy only after withdrawal, never approve directly.
        Capability::ReviewAgentPolicy => !owner,
        Capability::WithdrawSubmission => true,
        Capability::ManageCredentials => owner,
    }
}

pub fn require_capability(actor: &Actor, capability: Capability) -> Result<(), DomainError> {
    if base_allow(actor.kind, capability) {
        Ok(())
    } else {
        Err(DomainError::Forbidden(format!(
            "{} does not hold the {} capability",
            actor.kind.as_str(),
            capability.as_str()
        )))
    }
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Utc};

    use super::*;
    use crate::model::ActorId;

    fn actor(kind: ActorKind) -> Actor {
        let now: DateTime<Utc> = "2026-09-14T00:00:00Z".parse().unwrap();
        Actor {
            id: ActorId::generate(now),
            kind,
            label: "someone".to_string(),
            revoked: false,
            created_at: now,
        }
    }

    #[test]
    fn secret_string_redacts_debug_and_display() {
        let secret = SecretString::new("owner-token-value".to_string());
        assert_eq!(format!("{secret:?}"), "[redacted]");
        assert_eq!(format!("{secret}"), "[redacted]");
        assert_eq!(secret.expose(), "owner-token-value");
    }

    #[test]
    fn generated_tokens_are_32_random_bytes_base64url() {
        let first = generate_token();
        let second = generate_token();
        assert_ne!(first.expose(), second.expose());
        let decoded = URL_SAFE_NO_PAD.decode(first.expose()).unwrap();
        assert_eq!(decoded.len(), TOKEN_BYTES);
    }

    #[test]
    fn token_digest_is_deterministic_sha256() {
        let token = SecretString::new("fixed".to_string());
        let digest = token_digest(&token);
        assert_eq!(digest, token_digest(&token));
        assert_ne!(digest, token_digest(&SecretString::new("other".into())));
        assert_eq!(URL_SAFE_NO_PAD.decode(&digest).unwrap().len(), 32);
        assert!(digest_matches(&digest, &token_digest(&token)));
        assert!(!digest_matches(&digest, "different"));
    }

    #[test]
    fn canonical_request_hash_ignores_json_key_order() {
        let a: serde_json::Value = serde_json::from_str(r#"{"b":1,"a":{"y":2,"x":3}}"#).unwrap();
        let b: serde_json::Value = serde_json::from_str(r#"{"a":{"x":3,"y":2},"b":1}"#).unwrap();
        assert_eq!(
            canonical_request_hash("POST", "/api/v1/agents", &a),
            canonical_request_hash("POST", "/api/v1/agents", &b)
        );
        assert_ne!(
            canonical_request_hash("POST", "/api/v1/agents", &a),
            canonical_request_hash("POST", "/api/v1/agents/other", &a)
        );
        assert_ne!(
            canonical_request_hash("POST", "/api/v1/agents", &a),
            canonical_request_hash("DELETE", "/api/v1/agents", &a)
        );
    }

    // One assertion per plan/12 table row: (owner column, agent column).
    #[test]
    fn capability_matrix_matches_plan12_table() {
        let expected = [
            (Capability::ReadProjectResources, true, true),
            (Capability::AdministerProject, true, false),
            (Capability::CreateWorkItems, true, true),
            (Capability::EditDescriptions, true, true),
            (Capability::LowerRequirements, true, false),
            (Capability::ResolveWork, true, false),
            (Capability::AddDependency, true, true),
            (Capability::RemoveDependencyOrUnblock, true, false),
            (Capability::BlockWithReason, true, true),
            (Capability::AuthorDocuments, true, true),
            (Capability::ClaimWork, true, true),
            (Capability::ReviewHumanPolicy, true, false),
            (Capability::ReviewAgentPolicy, false, true),
            (Capability::WithdrawSubmission, true, true),
            (Capability::ManageCredentials, true, false),
        ];
        assert_eq!(expected.len(), Capability::ALL.len());
        for (capability, owner, agent) in expected {
            assert_eq!(
                base_allow(ActorKind::Human, capability),
                owner,
                "owner column for {}",
                capability.as_str()
            );
            assert_eq!(
                base_allow(ActorKind::Agent, capability),
                agent,
                "agent column for {}",
                capability.as_str()
            );
        }
    }

    #[test]
    fn agent_is_denied_human_review_capability() {
        let err = require_capability(&actor(ActorKind::Agent), Capability::ReviewHumanPolicy)
            .unwrap_err();
        assert!(matches!(err, DomainError::Forbidden(_)));
        assert_eq!(err.code(), "forbidden");
    }

    #[test]
    fn agents_cannot_manage_credentials_and_owners_cannot_agent_review() {
        assert!(matches!(
            require_capability(&actor(ActorKind::Agent), Capability::ManageCredentials),
            Err(DomainError::Forbidden(_))
        ));
        assert!(matches!(
            require_capability(&actor(ActorKind::Human), Capability::ReviewAgentPolicy),
            Err(DomainError::Forbidden(_))
        ));
        assert!(
            require_capability(&actor(ActorKind::Human), Capability::ManageCredentials).is_ok()
        );
        assert!(require_capability(&actor(ActorKind::Agent), Capability::ClaimWork).is_ok());
    }

    #[test]
    fn capability_names_are_distinct() {
        let mut names: Vec<&str> = Capability::ALL.iter().map(|c| c.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), Capability::ALL.len());
    }
}
