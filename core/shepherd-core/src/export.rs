//! Project export/import with versioned schema.
//!
//! Pure functions for building export documents and validating imports.
//! The store handles loading/saving; this module handles the format logic.

use std::collections::HashSet;

use chrono::Utc;

use crate::error::Error;
use crate::model::{
    ExportDocument, ImportResult, KnowledgeItem, Project, ProjectId, Relation, Session, Task,
};

/// Current export schema version (SemVer).
pub const EXPORT_VERSION: &str = "1.0.0";

/// Build an export document from loaded project data.
pub fn build_export(
    project: Project,
    tasks: Vec<Task>,
    relations: Vec<Relation>,
    sessions: Vec<Session>,
    knowledge_items: Vec<KnowledgeItem>,
) -> ExportDocument {
    ExportDocument {
        version: EXPORT_VERSION.to_string(),
        exported_at: Utc::now(),
        project,
        tasks,
        relations,
        sessions,
        knowledge_items,
    }
}

/// Validate an import document before inserting.
///
/// Checks:
/// - Schema version compatibility (must match major version).
/// - Internal referential integrity (task IDs referenced by relations,
///   sessions, and knowledge items must exist in the document).
pub fn validate_import(doc: &ExportDocument) -> Result<(), Error> {
    // Version check: major version must match.
    let doc_major = doc
        .version
        .split('.')
        .next()
        .and_then(|s| s.parse::<u32>().ok());
    let expected_major = EXPORT_VERSION
        .split('.')
        .next()
        .and_then(|s| s.parse::<u32>().ok());

    match (doc_major, expected_major) {
        (Some(d), Some(e)) if d == e => {}
        _ => {
            return Err(Error::ImportSchemaMismatch {
                detail: format!(
                    "export version {} is not compatible with expected {}",
                    doc.version, EXPORT_VERSION
                ),
            });
        }
    }

    // Referential integrity: collect valid task IDs.
    let task_ids: HashSet<_> = doc.tasks.iter().map(|t| t.id).collect();
    let session_ids: HashSet<_> = doc.sessions.iter().map(|s| s.id).collect();

    // Relations must reference tasks in the document.
    for rel in &doc.relations {
        if !task_ids.contains(&rel.source_task_id) {
            return Err(Error::ImportSchemaMismatch {
                detail: format!(
                    "relation {} references unknown source task {}",
                    rel.id, rel.source_task_id
                ),
            });
        }
        if !task_ids.contains(&rel.target_task_id) {
            return Err(Error::ImportSchemaMismatch {
                detail: format!(
                    "relation {} references unknown target task {}",
                    rel.id, rel.target_task_id
                ),
            });
        }
    }

    // Sessions must reference tasks in the document.
    for session in &doc.sessions {
        if !task_ids.contains(&session.task_id) {
            return Err(Error::ImportSchemaMismatch {
                detail: format!(
                    "session {} references unknown task {}",
                    session.id, session.task_id
                ),
            });
        }
    }

    // Knowledge items with task scope must reference valid tasks.
    for ki in &doc.knowledge_items {
        if let Some(tid) = ki.task_id
            && !task_ids.contains(&tid)
        {
            return Err(Error::ImportSchemaMismatch {
                detail: format!("knowledge item {} references unknown task {tid}", ki.id),
            });
        }
        if let Some(sid) = ki.session_id
            && !session_ids.contains(&sid)
        {
            return Err(Error::ImportSchemaMismatch {
                detail: format!("knowledge item {} references unknown session {sid}", ki.id),
            });
        }
    }

    Ok(())
}

/// Compute the import result summary for a set of imported entities.
pub fn import_summary(
    project_id: ProjectId,
    task_count: usize,
    relation_count: usize,
    session_count: usize,
    knowledge_count: usize,
) -> ImportResult {
    ImportResult {
        project_id,
        task_count,
        relation_count,
        session_count,
        knowledge_count,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;
    use chrono::Utc;

    fn sample_project() -> Project {
        Project {
            id: ProjectId::new(),
            name: "test".into(),
            description: "test project".into(),
            settings: ProjectSettings { review_gate: true },
            deleted_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn build_export_sets_version() {
        let doc = build_export(sample_project(), vec![], vec![], vec![], vec![]);
        assert_eq!(doc.version, EXPORT_VERSION);
    }

    #[test]
    fn validate_import_rejects_wrong_major_version() {
        let mut doc = build_export(sample_project(), vec![], vec![], vec![], vec![]);
        doc.version = "2.0.0".into();
        assert!(validate_import(&doc).is_err());
    }

    #[test]
    fn validate_import_accepts_compatible_minor() {
        let mut doc = build_export(sample_project(), vec![], vec![], vec![], vec![]);
        doc.version = "1.1.0".into();
        assert!(validate_import(&doc).is_ok());
    }

    #[test]
    fn validate_import_catches_broken_relation_ref() {
        let task = Task {
            id: TaskId::new(),
            project_id: ProjectId::new(),
            title: "t".into(),
            description: String::new(),
            task_type: TaskType::Code,
            status: TaskStatus::Proposed,
            metadata: serde_json::json!({}),
            assignee: None,
            graph_role: vec![],
            attempt_count: 0,
            blocked_from_status: None,
            block_reason: None,
            deleted_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let relation = Relation {
            id: RelationId::new(),
            relation_type: RelationType::DependsOn,
            source_task_id: task.id,
            target_task_id: TaskId::new(), // unknown
            created_at: Utc::now(),
        };
        let doc = build_export(sample_project(), vec![task], vec![relation], vec![], vec![]);
        assert!(validate_import(&doc).is_err());
    }

    #[test]
    fn validate_import_empty_project_ok() {
        let doc = build_export(sample_project(), vec![], vec![], vec![], vec![]);
        assert!(validate_import(&doc).is_ok());
    }
}
