pub(crate) mod errors;

use serde::Serialize;
use shepherd_core::commands::CommandContext;
use shepherd_core::model::{Actor as CoreActor, ActorKind, CommandId, canonical_request_hash};
use shepherd_core::storage::Store;
use uuid::Uuid;

use crate::generated::types as wire;
use crate::middleware::current_context;

pub(crate) fn request_id() -> Uuid {
    current_context().0.request_id
}

pub(crate) fn wire_actor(actor: &CoreActor) -> wire::Actor {
    wire::Actor {
        id: actor.id.as_uuid(),
        kind: match actor.kind {
            ActorKind::Human => wire::ActorKind::Human,
            ActorKind::Agent => wire::ActorKind::Agent,
        },
        label: actor.label.clone(),
        revoked: actor.revoked,
    }
}

pub(crate) fn command_context(
    actor: &CoreActor,
    idempotency_key: Uuid,
    expected_revision: Option<i64>,
    store: &Store,
) -> CommandContext {
    let now = store.clock().now();
    CommandContext {
        actor: actor.clone(),
        command_id: CommandId::generate(now),
        idempotency_key,
        expected_revision,
        now,
    }
}

/// Canonical replay hash over the actual request (plan/07: stored method,
/// normalized path and canonical parsed JSON must match on replay).
pub(crate) fn request_hash(method: &str, path: &str, body: &impl Serialize) -> String {
    let value = serde_json::to_value(body).expect("wire body serializes");
    canonical_request_hash(method, path, &value)
}
