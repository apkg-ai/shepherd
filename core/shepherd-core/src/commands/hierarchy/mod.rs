use super::{
    CommandContext, CommandResult, PendingEvent, append_events, epic_scope, live_actor,
    missing_after_write, require_revision, task_has_active_work, task_scope,
};
use crate::error::DomainError;
use crate::model::{
    BUILTIN_TASK_TYPES, Capability, DESCRIPTION_MAX_CHARS, Epic, EpicCreate, EpicId, EpicStatus,
    Goal, GoalCreate, GoalId, NAME_MAX_CHARS, Project, ProjectCreate, ProjectId, ProjectPatch,
    ProjectSettings, REASON_MAX_CHARS, Revision, Task, TaskCreate, TaskId, TaskPatch, TaskStatus,
    TaskType, TaskTypeCreate, TaskTypeId, TaskTypePatch, TextPatch, require_capability,
    validate_long_text, validate_required_text, validate_type_key, validate_type_label,
};
use crate::queries::hierarchy::{
    epic_row, find_epic, find_goal, find_project, find_task, find_task_type, goal_row,
    project_archived, project_row, task_row,
};
use crate::storage::rows::format_ts;
use crate::storage::{StorageError, Store};
use crate::workflow::{self, AffectedScope, policy};

mod epics;
mod goals;
mod projects;
mod task_types;
mod tasks;

#[cfg(test)]
mod test_support;
