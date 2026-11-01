#![forbid(unsafe_code)]

pub mod error;
pub mod streaming;
pub mod transport;

// The generated module trips these style lints by design.
#[allow(clippy::collapsible_if, clippy::double_must_use)]
mod generated;

use std::path::Path;

use reqwest::Method;
use serde::Serialize;
use serde::de::DeserializeOwned;
use uuid::Uuid;

pub use error::Error;
pub use generated::types;
pub use streaming::{EventStream, EventStreamItem};
pub use transport::{CallOptions, DEFAULT_BASE_URL, Secret, load_token_file};

use transport::{OpKind, Transport};

type Pair = (&'static str, String);

fn push(pairs: &mut Vec<Pair>, name: &'static str, value: Option<String>) {
    if let Some(value) = value {
        pairs.push((name, value));
    }
}

#[derive(Debug, Clone, Default)]
pub struct PageQuery {
    pub cursor: Option<String>,
    pub limit: Option<i32>,
}

impl PageQuery {
    fn pairs(&self) -> Vec<Pair> {
        let mut pairs = Vec::new();
        push(&mut pairs, "cursor", self.cursor.clone());
        push(&mut pairs, "limit", self.limit.map(|v| v.to_string()));
        pairs
    }
}

#[derive(Debug, Clone, Default)]
pub struct ListProjectsQuery {
    pub cursor: Option<String>,
    pub limit: Option<i32>,
    pub include_archived: Option<bool>,
}

impl ListProjectsQuery {
    fn pairs(&self) -> Vec<Pair> {
        let mut pairs = Vec::new();
        push(&mut pairs, "cursor", self.cursor.clone());
        push(&mut pairs, "limit", self.limit.map(|v| v.to_string()));
        push(
            &mut pairs,
            "include_archived",
            self.include_archived.map(|v| v.to_string()),
        );
        pairs
    }
}

pub type ListGoalsQuery = ListProjectsQuery;
pub type ListTaskTypesQuery = ListProjectsQuery;

#[derive(Debug, Clone, Default)]
pub struct ListEpicsQuery {
    pub cursor: Option<String>,
    pub limit: Option<i32>,
    pub goal_id: Option<Uuid>,
    pub status: Option<types::EntityStatus>,
    pub include_archived: Option<bool>,
}

impl ListEpicsQuery {
    fn pairs(&self) -> Vec<Pair> {
        let mut pairs = Vec::new();
        push(&mut pairs, "cursor", self.cursor.clone());
        push(&mut pairs, "limit", self.limit.map(|v| v.to_string()));
        push(&mut pairs, "goal_id", self.goal_id.map(|v| v.to_string()));
        push(
            &mut pairs,
            "status",
            self.status.as_ref().map(|v| v.as_str().into()),
        );
        push(
            &mut pairs,
            "include_archived",
            self.include_archived.map(|v| v.to_string()),
        );
        pairs
    }
}

#[derive(Debug, Clone, Default)]
pub struct ListTasksQuery {
    pub cursor: Option<String>,
    pub limit: Option<i32>,
    pub epic_id: Option<Uuid>,
    pub status: Option<types::EntityStatus>,
    pub phase: Option<types::TaskPhase>,
    pub type_key: Option<String>,
    pub include_archived: Option<bool>,
}

impl ListTasksQuery {
    fn pairs(&self) -> Vec<Pair> {
        let mut pairs = Vec::new();
        push(&mut pairs, "cursor", self.cursor.clone());
        push(&mut pairs, "limit", self.limit.map(|v| v.to_string()));
        push(&mut pairs, "epic_id", self.epic_id.map(|v| v.to_string()));
        push(
            &mut pairs,
            "status",
            self.status.as_ref().map(|v| v.as_str().into()),
        );
        push(
            &mut pairs,
            "phase",
            self.phase.as_ref().map(|v| v.as_str().into()),
        );
        push(&mut pairs, "type_key", self.type_key.clone());
        push(
            &mut pairs,
            "include_archived",
            self.include_archived.map(|v| v.to_string()),
        );
        pairs
    }
}

#[derive(Debug, Clone, Default)]
pub struct ListDependenciesQuery {
    pub cursor: Option<String>,
    pub limit: Option<i32>,
    pub level: Option<types::DependencyLevel>,
    pub scope_id: Option<Uuid>,
}

impl ListDependenciesQuery {
    fn pairs(&self) -> Vec<Pair> {
        let mut pairs = Vec::new();
        push(&mut pairs, "cursor", self.cursor.clone());
        push(&mut pairs, "limit", self.limit.map(|v| v.to_string()));
        push(
            &mut pairs,
            "level",
            self.level.as_ref().map(|v| v.as_str().into()),
        );
        push(&mut pairs, "scope_id", self.scope_id.map(|v| v.to_string()));
        pairs
    }
}

#[derive(Debug, Clone, Default)]
pub struct ListWorkQuery {
    pub cursor: Option<String>,
    pub limit: Option<i32>,
    pub epic_id: Option<Uuid>,
    pub type_key: Option<String>,
}

impl ListWorkQuery {
    fn pairs(&self, phase: types::ClaimInputPhase) -> Vec<Pair> {
        let mut pairs = vec![("phase", phase.as_str().to_owned())];
        push(&mut pairs, "cursor", self.cursor.clone());
        push(&mut pairs, "limit", self.limit.map(|v| v.to_string()));
        push(&mut pairs, "epic_id", self.epic_id.map(|v| v.to_string()));
        push(&mut pairs, "type_key", self.type_key.clone());
        pairs
    }
}

#[derive(Debug, Clone, Default)]
pub struct ListClaimsQuery {
    pub cursor: Option<String>,
    pub limit: Option<i32>,
    pub task_id: Option<Uuid>,
    pub active: Option<bool>,
}

impl ListClaimsQuery {
    fn pairs(&self) -> Vec<Pair> {
        let mut pairs = Vec::new();
        push(&mut pairs, "cursor", self.cursor.clone());
        push(&mut pairs, "limit", self.limit.map(|v| v.to_string()));
        push(&mut pairs, "task_id", self.task_id.map(|v| v.to_string()));
        push(&mut pairs, "active", self.active.map(|v| v.to_string()));
        pairs
    }
}

#[derive(Debug, Clone, Default)]
pub struct ListDocumentsQuery {
    pub cursor: Option<String>,
    pub limit: Option<i32>,
    pub owner_kind: Option<types::DocumentOwnerKind>,
    pub owner_id: Option<Uuid>,
    pub kind: Option<types::DocumentKind>,
}

impl ListDocumentsQuery {
    fn pairs(&self) -> Vec<Pair> {
        let mut pairs = Vec::new();
        push(&mut pairs, "cursor", self.cursor.clone());
        push(&mut pairs, "limit", self.limit.map(|v| v.to_string()));
        push(
            &mut pairs,
            "owner_kind",
            self.owner_kind.as_ref().map(|v| v.as_str().into()),
        );
        push(&mut pairs, "owner_id", self.owner_id.map(|v| v.to_string()));
        push(
            &mut pairs,
            "kind",
            self.kind.as_ref().map(|v| v.as_str().into()),
        );
        pairs
    }
}

#[derive(Debug, Clone, Default)]
pub struct ListSubmissionsQuery {
    pub cursor: Option<String>,
    pub limit: Option<i32>,
    pub task_id: Option<Uuid>,
    pub status: Option<types::SubmissionStatus>,
    pub kind: Option<types::SubmissionKind>,
    pub policy: Option<types::ReviewPolicy>,
}

impl ListSubmissionsQuery {
    fn pairs(&self) -> Vec<Pair> {
        let mut pairs = Vec::new();
        push(&mut pairs, "cursor", self.cursor.clone());
        push(&mut pairs, "limit", self.limit.map(|v| v.to_string()));
        push(&mut pairs, "task_id", self.task_id.map(|v| v.to_string()));
        push(
            &mut pairs,
            "status",
            self.status.as_ref().map(|v| v.as_str().into()),
        );
        push(
            &mut pairs,
            "kind",
            self.kind.as_ref().map(|v| v.as_str().into()),
        );
        push(
            &mut pairs,
            "policy",
            self.policy.as_ref().map(|v| v.as_str().into()),
        );
        pairs
    }
}

#[derive(Debug, Clone, Default)]
pub struct ListHistoryQuery {
    pub cursor: Option<String>,
    pub limit: Option<i32>,
    pub resource_id: Option<Uuid>,
}

impl ListHistoryQuery {
    fn pairs(&self) -> Vec<Pair> {
        let mut pairs = Vec::new();
        push(&mut pairs, "cursor", self.cursor.clone());
        push(&mut pairs, "limit", self.limit.map(|v| v.to_string()));
        push(
            &mut pairs,
            "resource_id",
            self.resource_id.map(|v| v.to_string()),
        );
        pairs
    }
}

/// Typed client for the shepherd daemon REST contract (`openapi/shepherd.yaml`).
///
/// One method per operationId. The client never generates Idempotency-Key,
/// If-Match or lease values — callers supply them via [`CallOptions`], and
/// retries reuse them verbatim. exportProject/importProject are absent from
/// the implemented contract (issue #97) and therefore from this client.
#[derive(Debug, Clone)]
pub struct Client {
    transport: Transport,
}

impl Client {
    pub fn new(base_url: &str, token: Secret) -> Result<Self, Error> {
        Ok(Self {
            transport: Transport::new(base_url, token)?,
        })
    }

    pub fn from_token_file(base_url: &str, token_file: &Path) -> Result<Self, Error> {
        Self::new(base_url, load_token_file(token_file)?)
    }

    async fn read<T: DeserializeOwned>(&self, path: String, query: Vec<Pair>) -> Result<T, Error> {
        self.transport
            .request::<T, ()>(
                Method::GET,
                &path,
                &query,
                None,
                &CallOptions::new(),
                OpKind::Read,
            )
            .await
    }

    async fn command<T, B>(
        &self,
        method: Method,
        path: String,
        body: &B,
        opts: &CallOptions,
    ) -> Result<T, Error>
    where
        T: DeserializeOwned,
        B: Serialize,
    {
        self.transport
            .request(method, &path, &[], Some(body), opts, OpKind::Mutation)
            .await
    }

    async fn command_bodyless<T: DeserializeOwned>(
        &self,
        method: Method,
        path: String,
        opts: &CallOptions,
    ) -> Result<T, Error> {
        self.transport
            .request::<T, ()>(method, &path, &[], None, opts, OpKind::Mutation)
            .await
    }

    pub async fn get_health(&self) -> Result<types::Health, Error> {
        self.read("/health".into(), Vec::new()).await
    }

    pub async fn get_principal(&self) -> Result<types::Actor, Error> {
        self.read("/api/v1/principal".into(), Vec::new()).await
    }

    pub async fn create_browser_session(
        &self,
        body: &types::SessionLogin,
    ) -> Result<types::BrowserSession, Error> {
        self.command(
            Method::POST,
            "/api/v1/session".into(),
            body,
            &CallOptions::new(),
        )
        .await
    }

    pub async fn get_browser_session(&self) -> Result<types::BrowserSession, Error> {
        self.read("/api/v1/session".into(), Vec::new()).await
    }

    pub async fn delete_browser_session(&self) -> Result<types::Ack, Error> {
        self.command_bodyless(
            Method::DELETE,
            "/api/v1/session".into(),
            &CallOptions::new(),
        )
        .await
    }

    pub async fn list_agents(&self, query: &PageQuery) -> Result<types::PageActor, Error> {
        self.read("/api/v1/agents".into(), query.pairs()).await
    }

    /// One-time token issuance; never idempotency-keyed, never retried.
    pub async fn create_agent(
        &self,
        body: &types::AgentCreate,
    ) -> Result<types::AgentToken, Error> {
        self.command(
            Method::POST,
            "/api/v1/agents".into(),
            body,
            &CallOptions::new(),
        )
        .await
    }

    pub async fn revoke_agent(
        &self,
        agent_id: Uuid,
        body: &types::ReasonInput,
        opts: &CallOptions,
    ) -> Result<types::Ack, Error> {
        self.command(
            Method::POST,
            format!("/api/v1/agents/{agent_id}/revoke"),
            body,
            opts,
        )
        .await
    }

    pub async fn list_projects(
        &self,
        query: &ListProjectsQuery,
    ) -> Result<types::PageProject, Error> {
        self.read("/api/v1/projects".into(), query.pairs()).await
    }

    pub async fn create_project(
        &self,
        body: &types::ProjectCreate,
        opts: &CallOptions,
    ) -> Result<types::Project, Error> {
        self.command(Method::POST, "/api/v1/projects".into(), body, opts)
            .await
    }

    pub async fn get_project(&self, project_id: Uuid) -> Result<types::Project, Error> {
        self.read(format!("/api/v1/projects/{project_id}"), Vec::new())
            .await
    }

    pub async fn update_project(
        &self,
        project_id: Uuid,
        body: &types::ProjectPatch,
        opts: &CallOptions,
    ) -> Result<types::Project, Error> {
        self.command(
            Method::PATCH,
            format!("/api/v1/projects/{project_id}"),
            body,
            opts,
        )
        .await
    }

    pub async fn archive_project(
        &self,
        project_id: Uuid,
        body: &types::ReasonInput,
        opts: &CallOptions,
    ) -> Result<types::Project, Error> {
        self.command(
            Method::POST,
            format!("/api/v1/projects/{project_id}/archive"),
            body,
            opts,
        )
        .await
    }

    pub async fn list_goals(
        &self,
        project_id: Uuid,
        query: &ListGoalsQuery,
    ) -> Result<types::PageGoal, Error> {
        self.read(
            format!("/api/v1/projects/{project_id}/goals"),
            query.pairs(),
        )
        .await
    }

    pub async fn create_goal(
        &self,
        project_id: Uuid,
        body: &types::GoalCreate,
        opts: &CallOptions,
    ) -> Result<types::Goal, Error> {
        self.command(
            Method::POST,
            format!("/api/v1/projects/{project_id}/goals"),
            body,
            opts,
        )
        .await
    }

    pub async fn get_goal(&self, project_id: Uuid, goal_id: Uuid) -> Result<types::Goal, Error> {
        self.read(
            format!("/api/v1/projects/{project_id}/goals/{goal_id}"),
            Vec::new(),
        )
        .await
    }

    pub async fn update_goal(
        &self,
        project_id: Uuid,
        goal_id: Uuid,
        body: &types::TextPatch,
        opts: &CallOptions,
    ) -> Result<types::Goal, Error> {
        self.command(
            Method::PATCH,
            format!("/api/v1/projects/{project_id}/goals/{goal_id}"),
            body,
            opts,
        )
        .await
    }

    pub async fn archive_goal(
        &self,
        project_id: Uuid,
        goal_id: Uuid,
        body: &types::ReasonInput,
        opts: &CallOptions,
    ) -> Result<types::Goal, Error> {
        self.command(
            Method::POST,
            format!("/api/v1/projects/{project_id}/goals/{goal_id}/archive"),
            body,
            opts,
        )
        .await
    }

    pub async fn get_goal_graph(
        &self,
        project_id: Uuid,
        goal_id: Uuid,
    ) -> Result<types::Graph, Error> {
        self.read(
            format!("/api/v1/projects/{project_id}/goals/{goal_id}/graph"),
            Vec::new(),
        )
        .await
    }

    pub async fn list_epics(
        &self,
        project_id: Uuid,
        query: &ListEpicsQuery,
    ) -> Result<types::PageEpic, Error> {
        self.read(
            format!("/api/v1/projects/{project_id}/epics"),
            query.pairs(),
        )
        .await
    }

    pub async fn create_epic(
        &self,
        project_id: Uuid,
        body: &types::EpicCreate,
        opts: &CallOptions,
    ) -> Result<types::Epic, Error> {
        self.command(
            Method::POST,
            format!("/api/v1/projects/{project_id}/epics"),
            body,
            opts,
        )
        .await
    }

    pub async fn get_epic(&self, project_id: Uuid, epic_id: Uuid) -> Result<types::Epic, Error> {
        self.read(
            format!("/api/v1/projects/{project_id}/epics/{epic_id}"),
            Vec::new(),
        )
        .await
    }

    pub async fn update_epic(
        &self,
        project_id: Uuid,
        epic_id: Uuid,
        body: &types::TextPatch,
        opts: &CallOptions,
    ) -> Result<types::Epic, Error> {
        self.command(
            Method::PATCH,
            format!("/api/v1/projects/{project_id}/epics/{epic_id}"),
            body,
            opts,
        )
        .await
    }

    pub async fn archive_epic(
        &self,
        project_id: Uuid,
        epic_id: Uuid,
        body: &types::ReasonInput,
        opts: &CallOptions,
    ) -> Result<types::Epic, Error> {
        self.epic_action(project_id, epic_id, "archive", body, opts)
            .await
    }

    pub async fn accept_epic(
        &self,
        project_id: Uuid,
        epic_id: Uuid,
        opts: &CallOptions,
    ) -> Result<types::Epic, Error> {
        self.epic_action(project_id, epic_id, "accept", &types::EmptyInput {}, opts)
            .await
    }

    pub async fn block_epic(
        &self,
        project_id: Uuid,
        epic_id: Uuid,
        body: &types::ReasonInput,
        opts: &CallOptions,
    ) -> Result<types::Epic, Error> {
        self.epic_action(project_id, epic_id, "block", body, opts)
            .await
    }

    pub async fn unblock_epic(
        &self,
        project_id: Uuid,
        epic_id: Uuid,
        opts: &CallOptions,
    ) -> Result<types::Epic, Error> {
        self.epic_action(project_id, epic_id, "unblock", &types::EmptyInput {}, opts)
            .await
    }

    pub async fn cancel_epic(
        &self,
        project_id: Uuid,
        epic_id: Uuid,
        body: &types::ReasonInput,
        opts: &CallOptions,
    ) -> Result<types::Epic, Error> {
        self.epic_action(project_id, epic_id, "cancel", body, opts)
            .await
    }

    pub async fn complete_epic(
        &self,
        project_id: Uuid,
        epic_id: Uuid,
        body: &types::ReasonInput,
        opts: &CallOptions,
    ) -> Result<types::Epic, Error> {
        self.epic_action(project_id, epic_id, "complete", body, opts)
            .await
    }

    async fn epic_action<B: Serialize>(
        &self,
        project_id: Uuid,
        epic_id: Uuid,
        action: &str,
        body: &B,
        opts: &CallOptions,
    ) -> Result<types::Epic, Error> {
        self.command(
            Method::POST,
            format!("/api/v1/projects/{project_id}/epics/{epic_id}/{action}"),
            body,
            opts,
        )
        .await
    }

    pub async fn get_epic_graph(
        &self,
        project_id: Uuid,
        epic_id: Uuid,
    ) -> Result<types::Graph, Error> {
        self.read(
            format!("/api/v1/projects/{project_id}/epics/{epic_id}/graph"),
            Vec::new(),
        )
        .await
    }

    pub async fn list_tasks(
        &self,
        project_id: Uuid,
        query: &ListTasksQuery,
    ) -> Result<types::PageTask, Error> {
        self.read(
            format!("/api/v1/projects/{project_id}/tasks"),
            query.pairs(),
        )
        .await
    }

    pub async fn create_task(
        &self,
        project_id: Uuid,
        body: &types::TaskCreate,
        opts: &CallOptions,
    ) -> Result<types::Task, Error> {
        self.command(
            Method::POST,
            format!("/api/v1/projects/{project_id}/tasks"),
            body,
            opts,
        )
        .await
    }

    pub async fn get_task(&self, project_id: Uuid, task_id: Uuid) -> Result<types::Task, Error> {
        self.read(
            format!("/api/v1/projects/{project_id}/tasks/{task_id}"),
            Vec::new(),
        )
        .await
    }

    pub async fn update_task(
        &self,
        project_id: Uuid,
        task_id: Uuid,
        body: &types::TaskPatch,
        opts: &CallOptions,
    ) -> Result<types::Task, Error> {
        self.command(
            Method::PATCH,
            format!("/api/v1/projects/{project_id}/tasks/{task_id}"),
            body,
            opts,
        )
        .await
    }

    pub async fn archive_task(
        &self,
        project_id: Uuid,
        task_id: Uuid,
        body: &types::ReasonInput,
        opts: &CallOptions,
    ) -> Result<types::Task, Error> {
        self.task_action(project_id, task_id, "archive", body, opts)
            .await
    }

    pub async fn accept_task(
        &self,
        project_id: Uuid,
        task_id: Uuid,
        opts: &CallOptions,
    ) -> Result<types::Task, Error> {
        self.task_action(project_id, task_id, "accept", &types::EmptyInput {}, opts)
            .await
    }

    pub async fn block_task(
        &self,
        project_id: Uuid,
        task_id: Uuid,
        body: &types::ReasonInput,
        opts: &CallOptions,
    ) -> Result<types::Task, Error> {
        self.task_action(project_id, task_id, "block", body, opts)
            .await
    }

    pub async fn unblock_task(
        &self,
        project_id: Uuid,
        task_id: Uuid,
        opts: &CallOptions,
    ) -> Result<types::Task, Error> {
        self.task_action(project_id, task_id, "unblock", &types::EmptyInput {}, opts)
            .await
    }

    pub async fn cancel_task(
        &self,
        project_id: Uuid,
        task_id: Uuid,
        body: &types::ReasonInput,
        opts: &CallOptions,
    ) -> Result<types::Task, Error> {
        self.task_action(project_id, task_id, "cancel", body, opts)
            .await
    }

    pub async fn waive_task(
        &self,
        project_id: Uuid,
        task_id: Uuid,
        body: &types::ReasonInput,
        opts: &CallOptions,
    ) -> Result<types::Task, Error> {
        self.task_action(project_id, task_id, "waive", body, opts)
            .await
    }

    async fn task_action<B: Serialize>(
        &self,
        project_id: Uuid,
        task_id: Uuid,
        action: &str,
        body: &B,
        opts: &CallOptions,
    ) -> Result<types::Task, Error> {
        self.command(
            Method::POST,
            format!("/api/v1/projects/{project_id}/tasks/{task_id}/{action}"),
            body,
            opts,
        )
        .await
    }

    pub async fn list_task_types(
        &self,
        project_id: Uuid,
        query: &ListTaskTypesQuery,
    ) -> Result<types::PageTaskType, Error> {
        self.read(
            format!("/api/v1/projects/{project_id}/task-types"),
            query.pairs(),
        )
        .await
    }

    pub async fn create_task_type(
        &self,
        project_id: Uuid,
        body: &types::TaskTypeCreate,
        opts: &CallOptions,
    ) -> Result<types::TaskType, Error> {
        self.command(
            Method::POST,
            format!("/api/v1/projects/{project_id}/task-types"),
            body,
            opts,
        )
        .await
    }

    pub async fn update_task_type(
        &self,
        project_id: Uuid,
        type_id: Uuid,
        body: &types::TaskTypePatch,
        opts: &CallOptions,
    ) -> Result<types::TaskType, Error> {
        self.command(
            Method::PATCH,
            format!("/api/v1/projects/{project_id}/task-types/{type_id}"),
            body,
            opts,
        )
        .await
    }

    pub async fn list_dependencies(
        &self,
        project_id: Uuid,
        query: &ListDependenciesQuery,
    ) -> Result<types::PageDependency, Error> {
        self.read(
            format!("/api/v1/projects/{project_id}/dependencies"),
            query.pairs(),
        )
        .await
    }

    pub async fn create_dependency(
        &self,
        project_id: Uuid,
        body: &types::DependencyCreate,
        opts: &CallOptions,
    ) -> Result<types::Dependency, Error> {
        self.command(
            Method::POST,
            format!("/api/v1/projects/{project_id}/dependencies"),
            body,
            opts,
        )
        .await
    }

    pub async fn delete_dependency(
        &self,
        project_id: Uuid,
        dependency_id: Uuid,
        opts: &CallOptions,
    ) -> Result<types::Ack, Error> {
        self.command_bodyless(
            Method::DELETE,
            format!("/api/v1/projects/{project_id}/dependencies/{dependency_id}"),
            opts,
        )
        .await
    }

    pub async fn list_work(
        &self,
        project_id: Uuid,
        phase: types::ClaimInputPhase,
        query: &ListWorkQuery,
    ) -> Result<types::PageTask, Error> {
        self.read(
            format!("/api/v1/projects/{project_id}/work"),
            query.pairs(phase),
        )
        .await
    }

    pub async fn get_task_context(
        &self,
        project_id: Uuid,
        task_id: Uuid,
    ) -> Result<types::ContextBundle, Error> {
        self.read(
            format!("/api/v1/projects/{project_id}/tasks/{task_id}/context"),
            Vec::new(),
        )
        .await
    }

    /// The returned [`types::ClaimGrant::lease_token`] authorizes renew,
    /// release and report via [`CallOptions::lease_token`]; keep it out of logs.
    pub async fn claim_task(
        &self,
        project_id: Uuid,
        task_id: Uuid,
        body: &types::ClaimInput,
        opts: &CallOptions,
    ) -> Result<types::ClaimGrant, Error> {
        self.command(
            Method::POST,
            format!("/api/v1/projects/{project_id}/tasks/{task_id}/claims"),
            body,
            opts,
        )
        .await
    }

    pub async fn list_claims(
        &self,
        project_id: Uuid,
        query: &ListClaimsQuery,
    ) -> Result<types::PageClaim, Error> {
        self.read(
            format!("/api/v1/projects/{project_id}/claims"),
            query.pairs(),
        )
        .await
    }

    pub async fn renew_claim(
        &self,
        project_id: Uuid,
        claim_id: Uuid,
        body: &types::RenewInput,
        opts: &CallOptions,
    ) -> Result<types::Claim, Error> {
        self.command(
            Method::POST,
            format!("/api/v1/projects/{project_id}/claims/{claim_id}/renew"),
            body,
            opts,
        )
        .await
    }

    pub async fn release_claim(
        &self,
        project_id: Uuid,
        claim_id: Uuid,
        opts: &CallOptions,
    ) -> Result<types::Claim, Error> {
        self.command(
            Method::POST,
            format!("/api/v1/projects/{project_id}/claims/{claim_id}/release"),
            &types::EmptyInput {},
            opts,
        )
        .await
    }

    pub async fn report_claim(
        &self,
        project_id: Uuid,
        claim_id: Uuid,
        body: &types::ReportInput,
        opts: &CallOptions,
    ) -> Result<types::Session, Error> {
        self.command(
            Method::POST,
            format!("/api/v1/projects/{project_id}/claims/{claim_id}/report"),
            body,
            opts,
        )
        .await
    }

    pub async fn list_task_sessions(
        &self,
        project_id: Uuid,
        task_id: Uuid,
        query: &PageQuery,
    ) -> Result<types::PageSession, Error> {
        self.read(
            format!("/api/v1/projects/{project_id}/tasks/{task_id}/sessions"),
            query.pairs(),
        )
        .await
    }

    pub async fn list_documents(
        &self,
        project_id: Uuid,
        query: &ListDocumentsQuery,
    ) -> Result<types::PageDocument, Error> {
        self.read(
            format!("/api/v1/projects/{project_id}/documents"),
            query.pairs(),
        )
        .await
    }

    pub async fn create_document(
        &self,
        project_id: Uuid,
        body: &types::DocumentCreate,
        opts: &CallOptions,
    ) -> Result<types::Document, Error> {
        self.command(
            Method::POST,
            format!("/api/v1/projects/{project_id}/documents"),
            body,
            opts,
        )
        .await
    }

    pub async fn get_document(
        &self,
        project_id: Uuid,
        document_id: Uuid,
    ) -> Result<types::Document, Error> {
        self.read(
            format!("/api/v1/projects/{project_id}/documents/{document_id}"),
            Vec::new(),
        )
        .await
    }

    pub async fn list_document_revisions(
        &self,
        project_id: Uuid,
        document_id: Uuid,
        query: &PageQuery,
    ) -> Result<types::PageDocumentRevision, Error> {
        self.read(
            format!("/api/v1/projects/{project_id}/documents/{document_id}/revisions"),
            query.pairs(),
        )
        .await
    }

    pub async fn create_document_revision(
        &self,
        project_id: Uuid,
        document_id: Uuid,
        body: &types::RevisionCreate,
        opts: &CallOptions,
    ) -> Result<types::DocumentRevision, Error> {
        self.command(
            Method::POST,
            format!("/api/v1/projects/{project_id}/documents/{document_id}/revisions"),
            body,
            opts,
        )
        .await
    }

    pub async fn get_document_revision(
        &self,
        project_id: Uuid,
        revision_id: Uuid,
    ) -> Result<types::DocumentRevision, Error> {
        self.read(
            format!("/api/v1/projects/{project_id}/document-revisions/{revision_id}"),
            Vec::new(),
        )
        .await
    }

    pub async fn select_task_plan(
        &self,
        project_id: Uuid,
        task_id: Uuid,
        body: &types::SelectPlanInput,
        opts: &CallOptions,
    ) -> Result<types::Task, Error> {
        self.command(
            Method::POST,
            format!("/api/v1/projects/{project_id}/tasks/{task_id}/plan"),
            body,
            opts,
        )
        .await
    }

    pub async fn list_submissions(
        &self,
        project_id: Uuid,
        query: &ListSubmissionsQuery,
    ) -> Result<types::PageSubmission, Error> {
        self.read(
            format!("/api/v1/projects/{project_id}/submissions"),
            query.pairs(),
        )
        .await
    }

    pub async fn get_submission(
        &self,
        project_id: Uuid,
        submission_id: Uuid,
    ) -> Result<types::Submission, Error> {
        self.read(
            format!("/api/v1/projects/{project_id}/submissions/{submission_id}"),
            Vec::new(),
        )
        .await
    }

    pub async fn withdraw_submission(
        &self,
        project_id: Uuid,
        submission_id: Uuid,
        body: &types::ReasonInput,
        opts: &CallOptions,
    ) -> Result<types::Submission, Error> {
        self.command(
            Method::POST,
            format!("/api/v1/projects/{project_id}/submissions/{submission_id}/withdraw"),
            body,
            opts,
        )
        .await
    }

    /// Agent reviewers must supply their lease via [`CallOptions::lease_token`];
    /// human reviewers hold no lease and omit it.
    pub async fn review_submission(
        &self,
        project_id: Uuid,
        submission_id: Uuid,
        body: &types::ReviewInput,
        opts: &CallOptions,
    ) -> Result<types::Review, Error> {
        self.command(
            Method::POST,
            format!("/api/v1/projects/{project_id}/submissions/{submission_id}/reviews"),
            body,
            opts,
        )
        .await
    }

    pub async fn list_submission_reviews(
        &self,
        project_id: Uuid,
        submission_id: Uuid,
        query: &PageQuery,
    ) -> Result<types::PageReview, Error> {
        self.read(
            format!("/api/v1/projects/{project_id}/submissions/{submission_id}/reviews"),
            query.pairs(),
        )
        .await
    }

    pub async fn list_history(
        &self,
        project_id: Uuid,
        query: &ListHistoryQuery,
    ) -> Result<types::PageEvent, Error> {
        self.read(
            format!("/api/v1/projects/{project_id}/history"),
            query.pairs(),
        )
        .await
    }

    /// Opens the resumable SSE event stream; see [`EventStream`].
    pub async fn get_events(
        &self,
        project_id: Uuid,
        after: Option<String>,
    ) -> Result<EventStream, Error> {
        EventStream::connect(
            self.transport.clone(),
            format!("/api/v1/projects/{project_id}/events"),
            after,
        )
        .await
    }
}
