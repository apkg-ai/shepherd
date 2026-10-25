pub(crate) mod errors;
pub(crate) mod hierarchy;

use serde::Serialize;
use shepherd_core::commands::CommandContext;
use shepherd_core::model::{Actor as CoreActor, ActorKind, CommandId, canonical_request_hash};
use shepherd_core::queries::{ListParams, Page};
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

/// Shared query-parameter mapping for list operations. Negative limits are
/// unreachable past the generated validation (minimum: 1); 0 falls to
/// effective_limit's range error.
pub(crate) fn list_params(
    cursor: Option<String>,
    limit: Option<i32>,
    include_archived: Option<bool>,
) -> ListParams {
    ListParams {
        limit: limit.map(|value| u32::try_from(value).unwrap_or(0)),
        cursor,
        include_archived: include_archived.unwrap_or(false),
    }
}

/// Shared pagination mapping: every wire PageX is one constructor over this.
pub(crate) fn page<T, W>(page: Page<T>, map: impl FnMut(T) -> W) -> (Vec<W>, Option<String>) {
    (page.items.into_iter().map(map).collect(), page.next_cursor)
}
