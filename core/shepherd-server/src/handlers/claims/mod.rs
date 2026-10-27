pub(crate) mod leases;
pub(crate) mod sessions;
pub(crate) mod work;

use shepherd_core::error::DomainError;
use shepherd_core::model::{Actor, ProjectId, TaskId};
use shepherd_core::storage::Store;

use crate::convert::{hierarchy, work as convert_work};
use crate::generated::types as wire;

/// Caller-scoped wire ContextBundle: the snapshot plus the task/epic
/// eligibility views the wire bodies require.
pub(crate) async fn context_body(
    store: &Store,
    project: ProjectId,
    task: TaskId,
    principal: &Actor,
) -> Result<wire::ContextBundle, DomainError> {
    let bundle = store.get_task_context(&project, &task).await?;
    let task_view = store.task_view(&project, &task, principal).await?;
    let epic = bundle.epic.id;
    let epic_view = store.epic_view(&project, &epic, principal).await?;
    Ok(convert_work::context_bundle(
        bundle,
        hierarchy::task(task_view),
        hierarchy::epic(epic_view),
    ))
}
