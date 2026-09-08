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

// ── Error → ProblemDetail ───────────────────────────────────────────────

pub fn problem_detail(e: &core::Error) -> wire::ProblemDetail {
    wire::ProblemDetail {
        r#type: e.urn().to_string(),
        title: e.title().to_string(),
        status: e.status_code() as i32,
        detail: Some(e.to_string()),
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
