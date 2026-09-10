//! Typed domain events emitted on every mutation.
//!
//! An [`EventBus`] backed by a [`tokio::sync::broadcast`] channel delivers
//! events to any number of subscribers (the SSE fan-out layer, tests, or
//! future Tauri embeddings). Events are fire-and-forget: if no subscriber
//! is listening, the event is silently dropped.

use chrono::{DateTime, Utc};
use tokio::sync::broadcast;

use crate::model::*;

/// Capacity of the broadcast channel. 256 is generous for a local daemon
/// with a handful of SSE clients.
const CHANNEL_CAPACITY: usize = 256;

/// A domain event emitted by the store on every mutation.
///
/// The 13 variants match the event catalog defined in
/// `openapi/shepherd-events.asyncapi.yaml`.
#[derive(Clone, Debug)]
pub enum DomainEvent {
    // ── Project ───────────────────────────────────────────────────
    ProjectCreated {
        project_id: ProjectId,
        name: String,
    },
    ProjectUpdated {
        project_id: ProjectId,
        updated_fields: Vec<String>,
    },

    // ── Task ──────────────────────────────────────────────────────
    TaskCreated {
        project_id: ProjectId,
        task_id: TaskId,
        title: String,
        task_type: TaskType,
        status: TaskStatus,
    },
    TaskUpdated {
        project_id: ProjectId,
        task_id: TaskId,
        updated_fields: Vec<String>,
    },
    TaskStatusChanged {
        project_id: ProjectId,
        task_id: TaskId,
        old_status: TaskStatus,
        new_status: TaskStatus,
    },

    // ── Relation ──────────────────────────────────────────────────
    RelationAdded {
        project_id: ProjectId,
        relation_id: RelationId,
        relation_type: RelationType,
        source_task_id: TaskId,
        target_task_id: TaskId,
    },
    RelationRemoved {
        project_id: ProjectId,
        relation_id: RelationId,
    },

    // ── Claim ─────────────────────────────────────────────────────
    ClaimAcquired {
        project_id: ProjectId,
        task_id: TaskId,
        claim_id: ClaimId,
        identity: Identity,
        expires_at: DateTime<Utc>,
    },
    ClaimRenewed {
        project_id: ProjectId,
        task_id: TaskId,
        claim_id: ClaimId,
        expires_at: DateTime<Utc>,
    },
    ClaimReleased {
        project_id: ProjectId,
        task_id: TaskId,
        claim_id: ClaimId,
    },
    ClaimExpired {
        project_id: ProjectId,
        task_id: TaskId,
        claim_id: ClaimId,
    },

    // ── Session ───────────────────────────────────────────────────
    SessionRecorded {
        project_id: ProjectId,
        task_id: TaskId,
        session_id: SessionId,
        outcome: SessionOutcome,
    },

    // ── Knowledge ─────────────────────────────────────────────────
    KnowledgeAdded {
        project_id: ProjectId,
        knowledge_id: KnowledgeId,
        knowledge_type: KnowledgeType,
        scope: KnowledgeScope,
    },
}

impl DomainEvent {
    /// The SSE event type string (e.g. `"task.status_changed"`).
    pub fn event_type(&self) -> &'static str {
        match self {
            Self::ProjectCreated { .. } => "project.created",
            Self::ProjectUpdated { .. } => "project.updated",
            Self::TaskCreated { .. } => "task.created",
            Self::TaskUpdated { .. } => "task.updated",
            Self::TaskStatusChanged { .. } => "task.status_changed",
            Self::RelationAdded { .. } => "relation.added",
            Self::RelationRemoved { .. } => "relation.removed",
            Self::ClaimAcquired { .. } => "claim.acquired",
            Self::ClaimRenewed { .. } => "claim.renewed",
            Self::ClaimReleased { .. } => "claim.released",
            Self::ClaimExpired { .. } => "claim.expired",
            Self::SessionRecorded { .. } => "session.recorded",
            Self::KnowledgeAdded { .. } => "knowledge.added",
        }
    }

    /// The project this event belongs to, used for SSE filtering.
    pub fn project_id(&self) -> ProjectId {
        match self {
            Self::ProjectCreated { project_id, .. }
            | Self::ProjectUpdated { project_id, .. }
            | Self::TaskCreated { project_id, .. }
            | Self::TaskUpdated { project_id, .. }
            | Self::TaskStatusChanged { project_id, .. }
            | Self::RelationAdded { project_id, .. }
            | Self::RelationRemoved { project_id, .. }
            | Self::ClaimAcquired { project_id, .. }
            | Self::ClaimRenewed { project_id, .. }
            | Self::ClaimReleased { project_id, .. }
            | Self::ClaimExpired { project_id, .. }
            | Self::SessionRecorded { project_id, .. }
            | Self::KnowledgeAdded { project_id, .. } => *project_id,
        }
    }

    /// JSON payload matching the AsyncAPI schema for this event type.
    ///
    /// Field names use the spec naming (`"type"` not `"task_type"`, etc.).
    pub fn to_payload(&self) -> serde_json::Value {
        match self {
            Self::ProjectCreated { project_id, name } => serde_json::json!({
                "project_id": project_id,
                "name": name,
            }),
            Self::ProjectUpdated {
                project_id,
                updated_fields,
            } => serde_json::json!({
                "project_id": project_id,
                "updated_fields": updated_fields,
            }),
            Self::TaskCreated {
                project_id,
                task_id,
                title,
                task_type,
                status,
            } => serde_json::json!({
                "project_id": project_id,
                "task_id": task_id,
                "title": title,
                "type": task_type,
                "status": status,
            }),
            Self::TaskUpdated {
                project_id,
                task_id,
                updated_fields,
            } => serde_json::json!({
                "project_id": project_id,
                "task_id": task_id,
                "updated_fields": updated_fields,
            }),
            Self::TaskStatusChanged {
                project_id,
                task_id,
                old_status,
                new_status,
            } => serde_json::json!({
                "project_id": project_id,
                "task_id": task_id,
                "old_status": old_status,
                "new_status": new_status,
            }),
            Self::RelationAdded {
                project_id,
                relation_id,
                relation_type,
                source_task_id,
                target_task_id,
            } => serde_json::json!({
                "project_id": project_id,
                "relation_id": relation_id,
                "type": relation_type,
                "source_task_id": source_task_id,
                "target_task_id": target_task_id,
            }),
            Self::RelationRemoved {
                project_id,
                relation_id,
            } => serde_json::json!({
                "project_id": project_id,
                "relation_id": relation_id,
            }),
            Self::ClaimAcquired {
                project_id,
                task_id,
                claim_id,
                identity,
                expires_at,
            } => serde_json::json!({
                "project_id": project_id,
                "task_id": task_id,
                "claim_id": claim_id,
                "identity": identity,
                "expires_at": expires_at,
            }),
            Self::ClaimRenewed {
                project_id,
                task_id,
                claim_id,
                expires_at,
            } => serde_json::json!({
                "project_id": project_id,
                "task_id": task_id,
                "claim_id": claim_id,
                "expires_at": expires_at,
            }),
            Self::ClaimReleased {
                project_id,
                task_id,
                claim_id,
            } => serde_json::json!({
                "project_id": project_id,
                "task_id": task_id,
                "claim_id": claim_id,
            }),
            Self::ClaimExpired {
                project_id,
                task_id,
                claim_id,
            } => serde_json::json!({
                "project_id": project_id,
                "task_id": task_id,
                "claim_id": claim_id,
            }),
            Self::SessionRecorded {
                project_id,
                task_id,
                session_id,
                outcome,
            } => serde_json::json!({
                "project_id": project_id,
                "task_id": task_id,
                "session_id": session_id,
                "outcome": outcome,
            }),
            Self::KnowledgeAdded {
                project_id,
                knowledge_id,
                knowledge_type,
                scope,
            } => serde_json::json!({
                "project_id": project_id,
                "knowledge_id": knowledge_id,
                "type": knowledge_type,
                "scope": scope,
            }),
        }
    }
}

/// Event bus backed by a [`tokio::sync::broadcast`] channel.
///
/// Holds the sending half. Cloning the bus shares the same underlying
/// channel. All `Store` instances created from the same `EventBus` (via
/// `Clone`) publish to the same set of subscribers.
#[derive(Clone, Debug)]
pub struct EventBus {
    sender: broadcast::Sender<DomainEvent>,
}

impl EventBus {
    /// Create a new event bus with the default channel capacity.
    pub fn new() -> Self {
        let (sender, _) = broadcast::channel(CHANNEL_CAPACITY);
        Self { sender }
    }

    /// Emit a domain event. If no subscribers are listening, the event is
    /// silently dropped.
    pub fn emit(&self, event: DomainEvent) {
        let _ = self.sender.send(event);
    }

    /// Subscribe to the event stream. Returns a receiver that yields every
    /// event emitted after this call.
    pub fn subscribe(&self) -> broadcast::Receiver<DomainEvent> {
        self.sender.subscribe()
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_type_strings_match_spec() {
        let pid = ProjectId::new();
        let tid = TaskId::new();

        let cases: Vec<(DomainEvent, &str)> = vec![
            (
                DomainEvent::ProjectCreated {
                    project_id: pid,
                    name: "x".into(),
                },
                "project.created",
            ),
            (
                DomainEvent::ProjectUpdated {
                    project_id: pid,
                    updated_fields: vec![],
                },
                "project.updated",
            ),
            (
                DomainEvent::TaskCreated {
                    project_id: pid,
                    task_id: tid,
                    title: "x".into(),
                    task_type: TaskType::Code,
                    status: TaskStatus::Proposed,
                },
                "task.created",
            ),
            (
                DomainEvent::TaskUpdated {
                    project_id: pid,
                    task_id: tid,
                    updated_fields: vec![],
                },
                "task.updated",
            ),
            (
                DomainEvent::TaskStatusChanged {
                    project_id: pid,
                    task_id: tid,
                    old_status: TaskStatus::Ready,
                    new_status: TaskStatus::InProgress,
                },
                "task.status_changed",
            ),
            (
                DomainEvent::RelationAdded {
                    project_id: pid,
                    relation_id: RelationId::new(),
                    relation_type: RelationType::DependsOn,
                    source_task_id: tid,
                    target_task_id: TaskId::new(),
                },
                "relation.added",
            ),
            (
                DomainEvent::RelationRemoved {
                    project_id: pid,
                    relation_id: RelationId::new(),
                },
                "relation.removed",
            ),
            (
                DomainEvent::ClaimAcquired {
                    project_id: pid,
                    task_id: tid,
                    claim_id: ClaimId::new(),
                    identity: Identity {
                        harness: "h".into(),
                        agent_model: "m".into(),
                        session_id: "s".into(),
                        label: None,
                    },
                    expires_at: Utc::now(),
                },
                "claim.acquired",
            ),
            (
                DomainEvent::ClaimRenewed {
                    project_id: pid,
                    task_id: tid,
                    claim_id: ClaimId::new(),
                    expires_at: Utc::now(),
                },
                "claim.renewed",
            ),
            (
                DomainEvent::ClaimReleased {
                    project_id: pid,
                    task_id: tid,
                    claim_id: ClaimId::new(),
                },
                "claim.released",
            ),
            (
                DomainEvent::ClaimExpired {
                    project_id: pid,
                    task_id: tid,
                    claim_id: ClaimId::new(),
                },
                "claim.expired",
            ),
            (
                DomainEvent::SessionRecorded {
                    project_id: pid,
                    task_id: tid,
                    session_id: SessionId::new(),
                    outcome: SessionOutcome::Succeeded,
                },
                "session.recorded",
            ),
            (
                DomainEvent::KnowledgeAdded {
                    project_id: pid,
                    knowledge_id: KnowledgeId::new(),
                    knowledge_type: KnowledgeType::Note,
                    scope: KnowledgeScope::Project,
                },
                "knowledge.added",
            ),
        ];

        for (event, expected) in cases {
            assert_eq!(event.event_type(), expected, "wrong type for {event:?}");
        }
    }

    #[test]
    fn project_id_extraction_for_all_variants() {
        let pid = ProjectId::new();
        let tid = TaskId::new();

        let events = vec![
            DomainEvent::ProjectCreated {
                project_id: pid,
                name: "x".into(),
            },
            DomainEvent::TaskStatusChanged {
                project_id: pid,
                task_id: tid,
                old_status: TaskStatus::Ready,
                new_status: TaskStatus::InProgress,
            },
            DomainEvent::ClaimReleased {
                project_id: pid,
                task_id: tid,
                claim_id: ClaimId::new(),
            },
            DomainEvent::KnowledgeAdded {
                project_id: pid,
                knowledge_id: KnowledgeId::new(),
                knowledge_type: KnowledgeType::Note,
                scope: KnowledgeScope::Project,
            },
        ];

        for event in events {
            assert_eq!(event.project_id(), pid, "wrong project_id for {event:?}");
        }
    }

    #[test]
    fn payload_matches_asyncapi_shape_all_13_types() {
        let pid = ProjectId::new();
        let tid = TaskId::new();
        let rid = RelationId::new();
        let cid = ClaimId::new();
        let sid = SessionId::new();
        let kid = KnowledgeId::new();
        let tid2 = TaskId::new();
        let now = Utc::now();
        let identity = Identity {
            harness: "h".into(),
            agent_model: "m".into(),
            session_id: "s".into(),
            label: Some("lbl".into()),
        };

        // 1. ProjectCreated
        let p = DomainEvent::ProjectCreated {
            project_id: pid,
            name: "test-project".into(),
        }
        .to_payload();
        assert_eq!(p["project_id"], pid.to_string());
        assert_eq!(p["name"], "test-project");

        // 2. ProjectUpdated
        let p = DomainEvent::ProjectUpdated {
            project_id: pid,
            updated_fields: vec!["name".into(), "settings".into()],
        }
        .to_payload();
        assert_eq!(p["project_id"], pid.to_string());
        assert_eq!(p["updated_fields"][0], "name");
        assert_eq!(p["updated_fields"][1], "settings");

        // 3. TaskCreated — uses "type" not "task_type"
        let p = DomainEvent::TaskCreated {
            project_id: pid,
            task_id: tid,
            title: "do thing".into(),
            task_type: TaskType::Code,
            status: TaskStatus::Proposed,
        }
        .to_payload();
        assert_eq!(p["project_id"], pid.to_string());
        assert_eq!(p["task_id"], tid.to_string());
        assert_eq!(p["title"], "do thing");
        assert_eq!(p["type"], "code");
        assert_eq!(p["status"], "proposed");
        assert!(
            p.get("task_type").is_none(),
            "must use 'type' not 'task_type'"
        );

        // 4. TaskUpdated
        let p = DomainEvent::TaskUpdated {
            project_id: pid,
            task_id: tid,
            updated_fields: vec!["title".into()],
        }
        .to_payload();
        assert_eq!(p["project_id"], pid.to_string());
        assert_eq!(p["task_id"], tid.to_string());
        assert_eq!(p["updated_fields"][0], "title");

        // 5. TaskStatusChanged
        let p = DomainEvent::TaskStatusChanged {
            project_id: pid,
            task_id: tid,
            old_status: TaskStatus::Ready,
            new_status: TaskStatus::InProgress,
        }
        .to_payload();
        assert_eq!(p["old_status"], "ready");
        assert_eq!(p["new_status"], "in_progress");

        // 6. RelationAdded — uses "type" not "relation_type"
        let p = DomainEvent::RelationAdded {
            project_id: pid,
            relation_id: rid,
            relation_type: RelationType::DependsOn,
            source_task_id: tid,
            target_task_id: tid2,
        }
        .to_payload();
        assert_eq!(p["project_id"], pid.to_string());
        assert_eq!(p["relation_id"], rid.to_string());
        assert_eq!(p["type"], "depends_on");
        assert_eq!(p["source_task_id"], tid.to_string());
        assert_eq!(p["target_task_id"], tid2.to_string());
        assert!(
            p.get("relation_type").is_none(),
            "must use 'type' not 'relation_type'"
        );

        // 7. RelationRemoved
        let p = DomainEvent::RelationRemoved {
            project_id: pid,
            relation_id: rid,
        }
        .to_payload();
        assert_eq!(p["project_id"], pid.to_string());
        assert_eq!(p["relation_id"], rid.to_string());

        // 8. ClaimAcquired
        let p = DomainEvent::ClaimAcquired {
            project_id: pid,
            task_id: tid,
            claim_id: cid,
            identity: identity.clone(),
            expires_at: now,
        }
        .to_payload();
        assert_eq!(p["project_id"], pid.to_string());
        assert_eq!(p["task_id"], tid.to_string());
        assert_eq!(p["claim_id"], cid.to_string());
        assert_eq!(p["identity"]["harness"], "h");
        assert_eq!(p["identity"]["agent_model"], "m");
        assert_eq!(p["identity"]["session_id"], "s");
        assert_eq!(p["identity"]["label"], "lbl");
        assert!(p["expires_at"].is_string());

        // 9. ClaimRenewed
        let p = DomainEvent::ClaimRenewed {
            project_id: pid,
            task_id: tid,
            claim_id: cid,
            expires_at: now,
        }
        .to_payload();
        assert_eq!(p["project_id"], pid.to_string());
        assert_eq!(p["task_id"], tid.to_string());
        assert_eq!(p["claim_id"], cid.to_string());
        assert!(p["expires_at"].is_string());

        // 10. ClaimReleased
        let p = DomainEvent::ClaimReleased {
            project_id: pid,
            task_id: tid,
            claim_id: cid,
        }
        .to_payload();
        assert_eq!(p["project_id"], pid.to_string());
        assert_eq!(p["task_id"], tid.to_string());
        assert_eq!(p["claim_id"], cid.to_string());

        // 11. ClaimExpired — same schema as ClaimReleased
        let p = DomainEvent::ClaimExpired {
            project_id: pid,
            task_id: tid,
            claim_id: cid,
        }
        .to_payload();
        assert_eq!(p["project_id"], pid.to_string());
        assert_eq!(p["task_id"], tid.to_string());
        assert_eq!(p["claim_id"], cid.to_string());

        // 12. SessionRecorded
        let p = DomainEvent::SessionRecorded {
            project_id: pid,
            task_id: tid,
            session_id: sid,
            outcome: SessionOutcome::Failed,
        }
        .to_payload();
        assert_eq!(p["project_id"], pid.to_string());
        assert_eq!(p["task_id"], tid.to_string());
        assert_eq!(p["session_id"], sid.to_string());
        assert_eq!(p["outcome"], "failed");

        // 13. KnowledgeAdded — uses "type" not "knowledge_type"
        let p = DomainEvent::KnowledgeAdded {
            project_id: pid,
            knowledge_id: kid,
            knowledge_type: KnowledgeType::Decision,
            scope: KnowledgeScope::Task,
        }
        .to_payload();
        assert_eq!(p["project_id"], pid.to_string());
        assert_eq!(p["knowledge_id"], kid.to_string());
        assert_eq!(p["type"], "decision");
        assert_eq!(p["scope"], "task");
        assert!(
            p.get("knowledge_type").is_none(),
            "must use 'type' not 'knowledge_type'"
        );
    }

    #[test]
    fn event_bus_emit_with_no_receivers() {
        let bus = EventBus::new();
        // Must not panic.
        bus.emit(DomainEvent::ProjectCreated {
            project_id: ProjectId::new(),
            name: "test".into(),
        });
    }

    #[test]
    fn event_bus_subscribe_and_receive() {
        let bus = EventBus::new();
        let mut rx = bus.subscribe();

        let pid = ProjectId::new();
        bus.emit(DomainEvent::ProjectCreated {
            project_id: pid,
            name: "test".into(),
        });

        let event = rx.try_recv().unwrap();
        assert!(matches!(
            event,
            DomainEvent::ProjectCreated {
                project_id,
                ref name,
            } if project_id == pid && name == "test"
        ));
    }

    #[test]
    fn event_bus_multiple_subscribers() {
        let bus = EventBus::new();
        let mut rx1 = bus.subscribe();
        let mut rx2 = bus.subscribe();

        bus.emit(DomainEvent::ProjectCreated {
            project_id: ProjectId::new(),
            name: "test".into(),
        });

        assert!(rx1.try_recv().is_ok());
        assert!(rx2.try_recv().is_ok());
    }
}
