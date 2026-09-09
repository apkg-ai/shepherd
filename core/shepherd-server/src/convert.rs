//! Conversions between `shepherd_core` domain types and the generated
//! OpenAPI wire types in `gen::types`.

use shepherd_core as core;

use crate::generated::types as wire;

// ── Domain → Wire (responses) ───────────────────────────────────────────

impl From<core::Project> for wire::Project {
    fn from(p: core::Project) -> Self {
        Self {
            id: p.id.0,
            name: p.name,
            description: p.description,
            settings: wire::ProjectSettings {
                review_gate: p.settings.review_gate,
            },
            created_at: p.created_at,
            updated_at: p.updated_at,
        }
    }
}

impl From<core::Task> for wire::Task {
    fn from(t: core::Task) -> Self {
        Self {
            id: t.id.0,
            project_id: t.project_id.0,
            title: t.title,
            description: t.description,
            r#type: match t.task_type {
                core::TaskType::Code => wire::TaskType::Code,
                core::TaskType::Question => wire::TaskType::Question,
                core::TaskType::Refactor => wire::TaskType::Refactor,
                core::TaskType::Review => wire::TaskType::Review,
                core::TaskType::Research => wire::TaskType::Research,
            },
            status: match t.status {
                core::TaskStatus::Proposed => wire::TaskStatus::Proposed,
                core::TaskStatus::Approved => wire::TaskStatus::Approved,
                core::TaskStatus::Ready => wire::TaskStatus::Ready,
                core::TaskStatus::InProgress => wire::TaskStatus::InProgress,
                core::TaskStatus::InReview => wire::TaskStatus::InReview,
                core::TaskStatus::Done => wire::TaskStatus::Done,
                core::TaskStatus::Blocked => wire::TaskStatus::Blocked,
                core::TaskStatus::Cancelled => wire::TaskStatus::Cancelled,
            },
            metadata: match t.metadata {
                serde_json::Value::Object(map) => wire::TaskMetadata {
                    additional_properties: map.into_iter().collect(),
                },
                _ => wire::TaskMetadata::default(),
            },
            assignee: t.assignee.map(|a| wire::Identity {
                harness: a.harness,
                agent_model: a.agent_model,
                session_id: a.session_id,
                label: a.label,
            }),
            graph_role: t
                .graph_role
                .into_iter()
                .map(|r| match r {
                    core::GraphRole::Start => wire::TaskGraphRoleItem::Start,
                    core::GraphRole::End => wire::TaskGraphRoleItem::End,
                    core::GraphRole::Milestone => wire::TaskGraphRoleItem::Milestone,
                })
                .collect(),
            attempt_count: t.attempt_count,
            created_at: t.created_at,
            updated_at: t.updated_at,
        }
    }
}

impl From<core::Relation> for wire::Relation {
    fn from(r: core::Relation) -> Self {
        Self {
            id: r.id.0,
            r#type: match r.relation_type {
                core::RelationType::Decomposition => wire::RelationType::Decomposition,
                core::RelationType::DependsOn => wire::RelationType::DependsOn,
            },
            source_task_id: r.source_task_id.0,
            target_task_id: r.target_task_id.0,
            created_at: r.created_at,
        }
    }
}

impl From<core::Page<core::Project>> for wire::ProjectList {
    fn from(page: core::Page<core::Project>) -> Self {
        Self {
            items: page.items.into_iter().map(wire::Project::from).collect(),
            has_more: page.has_more,
            next_cursor: page.next_cursor.map(Some),
        }
    }
}

impl From<core::Page<core::Task>> for wire::TaskList {
    fn from(page: core::Page<core::Task>) -> Self {
        Self {
            items: page.items.into_iter().map(wire::Task::from).collect(),
            has_more: page.has_more,
            next_cursor: page.next_cursor.map(Some),
        }
    }
}

impl From<core::Identity> for wire::Identity {
    fn from(i: core::Identity) -> Self {
        Self {
            harness: i.harness,
            agent_model: i.agent_model,
            session_id: i.session_id,
            label: i.label,
        }
    }
}

impl From<core::Claim> for wire::Claim {
    fn from(c: core::Claim) -> Self {
        Self {
            id: c.id.0,
            task_id: c.task_id.0,
            identity: c.identity.into(),
            ttl_seconds: c.ttl_seconds,
            lease_id: c.lease_id,
            acquired_at: c.acquired_at,
            expires_at: c.expires_at,
            renewed_at: c.renewed_at.map(Some),
        }
    }
}

impl From<core::Session> for wire::Session {
    fn from(s: core::Session) -> Self {
        Self {
            id: s.id.0,
            task_id: s.task_id.0,
            identity: s.identity.into(),
            started_at: s.started_at,
            ended_at: s.ended_at,
            outcome: match s.outcome {
                core::SessionOutcome::Succeeded => wire::SessionOutcome::Succeeded,
                core::SessionOutcome::Failed => wire::SessionOutcome::Failed,
            },
            failure_reason: s.failure_reason.map(Some),
            summary: s.summary,
            decisions: s.decisions,
            knowledge_items: s
                .knowledge_items
                .into_iter()
                .map(wire::KnowledgeItem::from)
                .collect(),
            artifacts: s.artifacts,
            created_at: s.created_at,
        }
    }
}

impl From<core::KnowledgeItem> for wire::KnowledgeItem {
    fn from(k: core::KnowledgeItem) -> Self {
        Self {
            id: k.id.0,
            r#type: match k.knowledge_type {
                core::KnowledgeType::Link => wire::KnowledgeItemType::Link,
                core::KnowledgeType::Transcript => wire::KnowledgeItemType::Transcript,
                core::KnowledgeType::Decision => wire::KnowledgeItemType::Decision,
                core::KnowledgeType::Note => wire::KnowledgeItemType::Note,
            },
            title: k.title,
            content: k.content,
            scope: match k.scope {
                core::KnowledgeScope::Task => wire::KnowledgeItemScope::Task,
                core::KnowledgeScope::Session => wire::KnowledgeItemScope::Session,
                core::KnowledgeScope::Project => wire::KnowledgeItemScope::Project,
            },
            task_id: k.task_id.map(|t| Some(t.0)),
            session_id: k.session_id.map(|s| Some(s.0)),
            project_id: k.project_id.0,
            created_at: k.created_at,
        }
    }
}

impl From<core::Page<core::Session>> for wire::SessionList {
    fn from(page: core::Page<core::Session>) -> Self {
        Self {
            items: page.items.into_iter().map(wire::Session::from).collect(),
            has_more: page.has_more,
            next_cursor: page.next_cursor.map(Some),
        }
    }
}

impl From<core::Page<core::KnowledgeItem>> for wire::KnowledgeItemList {
    fn from(page: core::Page<core::KnowledgeItem>) -> Self {
        Self {
            items: page
                .items
                .into_iter()
                .map(wire::KnowledgeItem::from)
                .collect(),
            has_more: page.has_more,
            next_cursor: page.next_cursor.map(Some),
        }
    }
}

impl From<core::ContextBundle> for wire::ContextBundle {
    fn from(b: core::ContextBundle) -> Self {
        Self {
            task: b.task.into(),
            ancestor_summaries: b
                .ancestor_summaries
                .into_iter()
                .map(|a| wire::AncestorSummary {
                    task_id: a.task_id.0,
                    title: a.title,
                    status: match a.status {
                        core::TaskStatus::Proposed => wire::AncestorSummaryStatus::Proposed,
                        core::TaskStatus::Approved => wire::AncestorSummaryStatus::Approved,
                        core::TaskStatus::Ready => wire::AncestorSummaryStatus::Ready,
                        core::TaskStatus::InProgress => wire::AncestorSummaryStatus::InProgress,
                        core::TaskStatus::InReview => wire::AncestorSummaryStatus::InReview,
                        core::TaskStatus::Done => wire::AncestorSummaryStatus::Done,
                        core::TaskStatus::Blocked => wire::AncestorSummaryStatus::Blocked,
                        core::TaskStatus::Cancelled => wire::AncestorSummaryStatus::Cancelled,
                    },
                    summary: a.summary,
                    decisions: a.decisions,
                })
                .collect(),
            project_knowledge: b
                .project_knowledge
                .into_iter()
                .map(wire::KnowledgeItem::from)
                .collect(),
            artifacts: b.artifacts,
            // The wire schema requires claimed_by; a sibling without an
            // active claimant carries no in-flight signal, so skip it.
            sibling_tasks: b
                .sibling_tasks
                .into_iter()
                .filter_map(|s| {
                    let claimed_by = s.claimed_by?;
                    Some(wire::SiblingTask {
                        task_id: s.task_id.0,
                        title: s.title,
                        status: match s.status {
                            core::TaskStatus::Proposed => wire::SiblingTaskStatus::Proposed,
                            core::TaskStatus::Approved => wire::SiblingTaskStatus::Approved,
                            core::TaskStatus::Ready => wire::SiblingTaskStatus::Ready,
                            core::TaskStatus::InProgress => wire::SiblingTaskStatus::InProgress,
                            core::TaskStatus::InReview => wire::SiblingTaskStatus::InReview,
                            core::TaskStatus::Done => wire::SiblingTaskStatus::Done,
                            core::TaskStatus::Blocked => wire::SiblingTaskStatus::Blocked,
                            core::TaskStatus::Cancelled => wire::SiblingTaskStatus::Cancelled,
                        },
                        claimed_by: claimed_by.into(),
                    })
                })
                .collect(),
        }
    }
}

impl From<core::ExportDocument> for wire::ExportDocument {
    fn from(d: core::ExportDocument) -> Self {
        Self {
            version: d.version,
            project: d.project.into(),
            tasks: d.tasks.into_iter().map(wire::Task::from).collect(),
            relations: d.relations.into_iter().map(wire::Relation::from).collect(),
            sessions: d.sessions.into_iter().map(wire::Session::from).collect(),
            knowledge: d
                .knowledge_items
                .into_iter()
                .map(wire::KnowledgeItem::from)
                .collect(),
        }
    }
}

impl From<core::ImportResult> for wire::ImportResult {
    fn from(r: core::ImportResult) -> Self {
        Self {
            project_id: r.project_id.0,
            task_count: r.task_count as i32,
            relation_count: r.relation_count as i32,
            session_count: r.session_count as i32,
            knowledge_count: r.knowledge_count as i32,
        }
    }
}

// ── Wire → Domain (requests) ────────────────────────────────────────────

impl From<wire::ProjectCreate> for core::ProjectCreate {
    fn from(w: wire::ProjectCreate) -> Self {
        Self {
            name: w.name,
            description: w.description,
            settings: w.settings.map(|s| core::ProjectSettings {
                review_gate: s.review_gate,
            }),
        }
    }
}

impl From<wire::ProjectUpdate> for core::ProjectUpdate {
    fn from(w: wire::ProjectUpdate) -> Self {
        Self {
            name: w.name,
            description: w.description,
            settings: w.settings.map(|s| core::ProjectSettings {
                review_gate: s.review_gate,
            }),
        }
    }
}

impl From<wire::TaskCreate> for core::TaskCreate {
    fn from(w: wire::TaskCreate) -> Self {
        Self {
            title: w.title,
            description: w.description,
            task_type: match w.r#type {
                wire::TaskCreateType::Code => core::TaskType::Code,
                wire::TaskCreateType::Question => core::TaskType::Question,
                wire::TaskCreateType::Refactor => core::TaskType::Refactor,
                wire::TaskCreateType::Review => core::TaskType::Review,
                wire::TaskCreateType::Research => core::TaskType::Research,
            },
            status: w.status.map(|s| match s {
                wire::TaskCreateStatus::Proposed => core::TaskStatus::Proposed,
                wire::TaskCreateStatus::Approved => core::TaskStatus::Approved,
            }),
            metadata: w
                .metadata
                .map(|m| serde_json::Value::Object(m.additional_properties.into_iter().collect())),
            assignee: w.assignee.map(|a| core::Identity {
                harness: a.harness,
                agent_model: a.agent_model,
                session_id: a.session_id,
                label: a.label,
            }),
            graph_role: w.graph_role.map(|roles| {
                roles
                    .into_iter()
                    .map(|r| match r {
                        wire::TaskCreateGraphRoleItem::Start => core::GraphRole::Start,
                        wire::TaskCreateGraphRoleItem::End => core::GraphRole::End,
                        wire::TaskCreateGraphRoleItem::Milestone => core::GraphRole::Milestone,
                    })
                    .collect()
            }),
        }
    }
}

impl From<wire::TaskUpdate> for core::TaskUpdate {
    fn from(w: wire::TaskUpdate) -> Self {
        Self {
            title: w.title,
            description: w.description,
            task_type: w.r#type.map(|t| match t {
                wire::TaskUpdateType::Code => core::TaskType::Code,
                wire::TaskUpdateType::Question => core::TaskType::Question,
                wire::TaskUpdateType::Refactor => core::TaskType::Refactor,
                wire::TaskUpdateType::Review => core::TaskType::Review,
                wire::TaskUpdateType::Research => core::TaskType::Research,
            }),
            metadata: w
                .metadata
                .map(|m| serde_json::Value::Object(m.additional_properties.into_iter().collect())),
            assignee: w.assignee.map(|opt| {
                opt.map(|a| core::Identity {
                    harness: a.harness,
                    agent_model: a.agent_model,
                    session_id: a.session_id,
                    label: a.label,
                })
            }),
            graph_role: w.graph_role.map(|roles| {
                roles
                    .into_iter()
                    .map(|r| match r {
                        wire::TaskUpdateGraphRoleItem::Start => core::GraphRole::Start,
                        wire::TaskUpdateGraphRoleItem::End => core::GraphRole::End,
                        wire::TaskUpdateGraphRoleItem::Milestone => core::GraphRole::Milestone,
                    })
                    .collect()
            }),
        }
    }
}

impl From<wire::RelationCreate> for core::RelationCreate {
    fn from(w: wire::RelationCreate) -> Self {
        Self {
            relation_type: match w.r#type {
                wire::RelationCreateType::Decomposition => core::RelationType::Decomposition,
                wire::RelationCreateType::DependsOn => core::RelationType::DependsOn,
            },
            target_task_id: core::TaskId::from_uuid(w.target_task_id),
        }
    }
}

impl From<wire::Identity> for core::Identity {
    fn from(w: wire::Identity) -> Self {
        Self {
            harness: w.harness,
            agent_model: w.agent_model,
            session_id: w.session_id,
            label: w.label,
        }
    }
}

impl From<wire::ClaimRequest> for core::ClaimRequest {
    fn from(w: wire::ClaimRequest) -> Self {
        Self {
            identity: w.identity.into(),
            ttl_seconds: w.ttl_seconds,
        }
    }
}

impl From<wire::ClaimRenewal> for core::ClaimRenewal {
    fn from(w: wire::ClaimRenewal) -> Self {
        Self {
            identity: w.identity.into(),
            ttl_seconds: w.ttl_seconds,
        }
    }
}

impl From<wire::ClaimRelease> for core::ClaimRelease {
    fn from(w: wire::ClaimRelease) -> Self {
        Self {
            identity: w.identity.into(),
        }
    }
}

impl From<wire::KnowledgeItemCreate> for core::KnowledgeItemCreate {
    fn from(w: wire::KnowledgeItemCreate) -> Self {
        Self {
            knowledge_type: match w.r#type {
                wire::KnowledgeItemCreateType::Link => core::KnowledgeType::Link,
                wire::KnowledgeItemCreateType::Transcript => core::KnowledgeType::Transcript,
                wire::KnowledgeItemCreateType::Decision => core::KnowledgeType::Decision,
                wire::KnowledgeItemCreateType::Note => core::KnowledgeType::Note,
            },
            title: w.title,
            content: w.content,
            scope: w
                .scope
                .map(|s| match s {
                    wire::KnowledgeItemCreateScope::Task => core::KnowledgeScope::Task,
                    wire::KnowledgeItemCreateScope::Session => core::KnowledgeScope::Session,
                    wire::KnowledgeItemCreateScope::Project => core::KnowledgeScope::Project,
                })
                .unwrap_or(core::KnowledgeScope::Project),
            task_id: w.task_id.map(core::TaskId::from_uuid),
            session_id: w.session_id.map(core::SessionId::from_uuid),
        }
    }
}

impl From<wire::SessionReport> for core::SessionReport {
    fn from(w: wire::SessionReport) -> Self {
        Self {
            identity: w.identity.into(),
            started_at: w.started_at,
            ended_at: w.ended_at,
            outcome: match w.outcome {
                wire::SessionReportOutcome::Succeeded => core::SessionOutcome::Succeeded,
                wire::SessionReportOutcome::Failed => core::SessionOutcome::Failed,
            },
            failure_reason: w.failure_reason,
            summary: Some(w.summary),
            decisions: w.decisions,
            knowledge_items: w
                .knowledge_items
                .map(|items| items.into_iter().map(Into::into).collect()),
            artifacts: w.artifacts,
        }
    }
}

// ── Wire → Domain (import document) ─────────────────────────────────────
//
// Import receives full resource representations. The wire schema omits
// server-internal fields (deleted_at, blocked_from_status, block_reason,
// exported_at); they reset to safe defaults on import.

impl From<wire::Project> for core::Project {
    fn from(w: wire::Project) -> Self {
        Self {
            id: core::ProjectId::from_uuid(w.id),
            name: w.name,
            description: w.description,
            settings: core::ProjectSettings {
                review_gate: w.settings.review_gate,
            },
            deleted_at: None,
            created_at: w.created_at,
            updated_at: w.updated_at,
        }
    }
}

impl From<wire::Task> for core::Task {
    fn from(w: wire::Task) -> Self {
        Self {
            id: core::TaskId::from_uuid(w.id),
            project_id: core::ProjectId::from_uuid(w.project_id),
            title: w.title,
            description: w.description,
            task_type: match w.r#type {
                wire::TaskType::Code => core::TaskType::Code,
                wire::TaskType::Question => core::TaskType::Question,
                wire::TaskType::Refactor => core::TaskType::Refactor,
                wire::TaskType::Review => core::TaskType::Review,
                wire::TaskType::Research => core::TaskType::Research,
            },
            status: match w.status {
                wire::TaskStatus::Proposed => core::TaskStatus::Proposed,
                wire::TaskStatus::Approved => core::TaskStatus::Approved,
                wire::TaskStatus::Ready => core::TaskStatus::Ready,
                wire::TaskStatus::InProgress => core::TaskStatus::InProgress,
                wire::TaskStatus::InReview => core::TaskStatus::InReview,
                wire::TaskStatus::Done => core::TaskStatus::Done,
                wire::TaskStatus::Blocked => core::TaskStatus::Blocked,
                wire::TaskStatus::Cancelled => core::TaskStatus::Cancelled,
            },
            metadata: match w.metadata {
                m if m.additional_properties.is_empty() => serde_json::json!({}),
                m => serde_json::Value::Object(m.additional_properties.into_iter().collect()),
            },
            assignee: w.assignee.map(|a| core::Identity {
                harness: a.harness,
                agent_model: a.agent_model,
                session_id: a.session_id,
                label: a.label,
            }),
            graph_role: w
                .graph_role
                .into_iter()
                .map(|r| match r {
                    wire::TaskGraphRoleItem::Start => core::GraphRole::Start,
                    wire::TaskGraphRoleItem::End => core::GraphRole::End,
                    wire::TaskGraphRoleItem::Milestone => core::GraphRole::Milestone,
                })
                .collect(),
            attempt_count: w.attempt_count,
            blocked_from_status: None,
            block_reason: None,
            deleted_at: None,
            created_at: w.created_at,
            updated_at: w.updated_at,
        }
    }
}

impl From<wire::Relation> for core::Relation {
    fn from(w: wire::Relation) -> Self {
        Self {
            id: core::RelationId::from_uuid(w.id),
            relation_type: match w.r#type {
                wire::RelationType::Decomposition => core::RelationType::Decomposition,
                wire::RelationType::DependsOn => core::RelationType::DependsOn,
            },
            source_task_id: core::TaskId::from_uuid(w.source_task_id),
            target_task_id: core::TaskId::from_uuid(w.target_task_id),
            created_at: w.created_at,
        }
    }
}

impl From<wire::KnowledgeItem> for core::KnowledgeItem {
    fn from(w: wire::KnowledgeItem) -> Self {
        Self {
            id: core::KnowledgeId::from_uuid(w.id),
            knowledge_type: match w.r#type {
                wire::KnowledgeItemType::Link => core::KnowledgeType::Link,
                wire::KnowledgeItemType::Transcript => core::KnowledgeType::Transcript,
                wire::KnowledgeItemType::Decision => core::KnowledgeType::Decision,
                wire::KnowledgeItemType::Note => core::KnowledgeType::Note,
            },
            title: w.title,
            content: w.content,
            scope: match w.scope {
                wire::KnowledgeItemScope::Task => core::KnowledgeScope::Task,
                wire::KnowledgeItemScope::Session => core::KnowledgeScope::Session,
                wire::KnowledgeItemScope::Project => core::KnowledgeScope::Project,
            },
            task_id: w.task_id.flatten().map(core::TaskId::from_uuid),
            session_id: w.session_id.flatten().map(core::SessionId::from_uuid),
            project_id: core::ProjectId::from_uuid(w.project_id),
            created_at: w.created_at,
        }
    }
}

impl From<wire::Session> for core::Session {
    fn from(w: wire::Session) -> Self {
        Self {
            id: core::SessionId::from_uuid(w.id),
            task_id: core::TaskId::from_uuid(w.task_id),
            identity: w.identity.into(),
            started_at: w.started_at,
            ended_at: w.ended_at,
            outcome: match w.outcome {
                wire::SessionOutcome::Succeeded => core::SessionOutcome::Succeeded,
                wire::SessionOutcome::Failed => core::SessionOutcome::Failed,
            },
            failure_reason: w.failure_reason.flatten(),
            summary: w.summary,
            decisions: w.decisions,
            knowledge_items: w.knowledge_items.into_iter().map(Into::into).collect(),
            artifacts: w.artifacts,
            created_at: w.created_at,
        }
    }
}

impl From<wire::ExportDocument> for core::ExportDocument {
    fn from(w: wire::ExportDocument) -> Self {
        Self {
            version: w.version,
            exported_at: chrono::Utc::now(),
            project: w.project.into(),
            tasks: w.tasks.into_iter().map(Into::into).collect(),
            relations: w.relations.into_iter().map(Into::into).collect(),
            sessions: w.sessions.into_iter().map(Into::into).collect(),
            knowledge_items: w.knowledge.into_iter().map(Into::into).collect(),
        }
    }
}

// ── Error → ProblemDetail ───────────────────────────────────────────────

pub fn problem_detail(e: &core::Error) -> wire::ProblemDetail {
    // Database errors can embed SQL fragments; keep them out of response
    // bodies and log server-side instead.
    let detail = match e {
        core::Error::Database(err) => {
            eprintln!("database error: {err}");
            "internal database error".to_string()
        }
        other => other.to_string(),
    };
    wire::ProblemDetail {
        r#type: e.urn().to_string(),
        title: e.title().to_string(),
        status: e.status_code() as i32,
        detail: Some(detail),
        instance: None,
        errors: match e {
            core::Error::ValidationError { errors, .. } if !errors.is_empty() => Some(
                errors
                    .iter()
                    .map(|ve| wire::ValidationErrorDetail {
                        field: ve.field.clone(),
                        message: ve.message.clone(),
                        code: ve.code.clone(),
                    })
                    .collect(),
            ),
            _ => None,
        },
    }
}

// ── ID parsing helpers ──────────────────────────────────────────────────
//
// The generated router validates path parameters against the spec's UUID
// pattern before handlers run, so the error branches here are unreachable
// over HTTP today. They remain as defense-in-depth: the handlers receive
// `String` and must convert to typed IDs for the store.

pub fn parse_project_id(s: &str) -> Result<core::ProjectId, core::Error> {
    s.parse()
        .map(core::ProjectId::from_uuid)
        .map_err(|_| core::Error::ValidationError {
            detail: format!("invalid project_id: {s}"),
            errors: vec![],
        })
}

pub fn parse_task_id(s: &str) -> Result<core::TaskId, core::Error> {
    s.parse()
        .map(core::TaskId::from_uuid)
        .map_err(|_| core::Error::ValidationError {
            detail: format!("invalid task_id: {s}"),
            errors: vec![],
        })
}

pub fn parse_relation_id(s: &str) -> Result<core::RelationId, core::Error> {
    s.parse()
        .map(core::RelationId::from_uuid)
        .map_err(|_| core::Error::ValidationError {
            detail: format!("invalid relation_id: {s}"),
            errors: vec![],
        })
}

pub fn parse_session_id(s: &str) -> Result<core::SessionId, core::Error> {
    s.parse()
        .map(core::SessionId::from_uuid)
        .map_err(|_| core::Error::ValidationError {
            detail: format!("invalid session_id: {s}"),
            errors: vec![],
        })
}

pub fn parse_knowledge_id(s: &str) -> Result<core::KnowledgeId, core::Error> {
    s.parse()
        .map(core::KnowledgeId::from_uuid)
        .map_err(|_| core::Error::ValidationError {
            detail: format!("invalid knowledge_id: {s}"),
            errors: vec![],
        })
}
