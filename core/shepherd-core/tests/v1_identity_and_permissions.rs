use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;

use shepherd_core::commands::{CommandContext, IdentityPaths};
use shepherd_core::error::DomainError;
use shepherd_core::model::{
    Actor, ActorKind, AgentCreate, Capability, Clock, CommandId, GoalCreate, ProjectCreate,
    ProjectId, ReviewPolicy, SecretString, TaskCreate, TaskId, TaskPatch, TaskTypePatch, TestClock,
    base_allow, require_capability,
};
use shepherd_core::storage::{Store, open, testing};
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{ConnectOptions, SqliteConnection};
use uuid::Uuid;

struct Fixture {
    dir: tempfile::TempDir,
    clock: Arc<TestClock>,
    store: Store,
}

impl Fixture {
    fn paths(&self) -> IdentityPaths {
        IdentityPaths::new(self.dir.path())
    }

    fn ctx(&self, actor: &Actor) -> CommandContext {
        let now = self.clock.now();
        CommandContext {
            actor: actor.clone(),
            command_id: CommandId::generate(now),
            idempotency_key: Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)),
            expected_revision: None,
            now,
        }
    }

    async fn owner(&self) -> Actor {
        self.store.ensure_owner(&self.paths()).await.unwrap().actor
    }

    async fn independent_connection(&self) -> SqliteConnection {
        SqliteConnectOptions::new()
            .filename(self.dir.path().join("shepherd.db"))
            .read_only(true)
            .connect()
            .await
            .unwrap()
    }
}

async fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let clock = testing::test_clock();
    let store = open(testing::store_options(
        dir.path(),
        "shepherd.db",
        clock.clone(),
    ))
    .await
    .unwrap();
    Fixture { dir, clock, store }
}

async fn count(conn: &mut SqliteConnection, table: &str) -> i64 {
    sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
        .fetch_one(conn)
        .await
        .unwrap()
}

async fn revision_of(conn: &mut SqliteConnection, table: &str, id: Uuid) -> i64 {
    sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT revision FROM {table} WHERE id = ?1"
    )))
    .bind(id.to_string())
    .fetch_one(conn)
    .await
    .unwrap()
}

fn file_mode(path: &Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

// Acceptance: agent cannot forge owner role in body. The context claims Human;
// the credential-backed row is an agent, and core rechecks the row.
#[tokio::test]
async fn agent_cannot_forge_owner_role_in_body() {
    let f = fixture().await;
    let owner = f.owner().await;
    let agent = f
        .store
        .create_agent(f.ctx(&owner), AgentCreate { label: "a".into() })
        .await
        .unwrap()
        .value
        .actor;
    let forged = Actor {
        kind: ActorKind::Human,
        label: "owner".to_string(),
        ..agent.clone()
    };
    let err = f
        .store
        .create_agent(f.ctx(&forged), AgentCreate { label: "b".into() })
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::Forbidden(_)));
    let mut conn = f.independent_connection().await;
    assert_eq!(count(&mut conn, "actors").await, 2);
    assert_eq!(count(&mut conn, "credentials").await, 2);
    let audit_targets: i64 =
        sqlx::query_scalar("SELECT count(*) FROM security_audit WHERE action = 'createAgent'")
            .fetch_one(&mut conn)
            .await
            .unwrap();
    assert_eq!(audit_targets, 1);
}

// Acceptance: agent cannot create agents (and cannot revoke them either).
#[tokio::test]
async fn agent_cannot_create_or_revoke_agents() {
    let f = fixture().await;
    let owner = f.owner().await;
    let agent = f
        .store
        .create_agent(f.ctx(&owner), AgentCreate { label: "a".into() })
        .await
        .unwrap()
        .value
        .actor;
    let other = f
        .store
        .create_agent(f.ctx(&owner), AgentCreate { label: "b".into() })
        .await
        .unwrap()
        .value
        .actor;
    let err = f
        .store
        .create_agent(f.ctx(&agent), AgentCreate { label: "c".into() })
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::Forbidden(_)));
    let err = f
        .store
        .revoke_agent(f.ctx(&agent), other.id, "coup".into(), "hash")
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::Forbidden(_)));
    let mut conn = f.independent_connection().await;
    assert_eq!(count(&mut conn, "actors").await, 3);
    let revoked: i64 = sqlx::query_scalar("SELECT count(*) FROM actors WHERE revoked = 1")
        .fetch_one(&mut conn)
        .await
        .unwrap();
    assert_eq!(revoked, 0);
    assert_eq!(count(&mut conn, "idempotency").await, 0);
}

// Acceptance: agent cannot weaken gates — neither per-task review policy nor
// the registry (task types, project defaults). Resource revisions stay intact.
#[tokio::test]
async fn agent_cannot_weaken_review_gates_via_registry() {
    let f = fixture().await;
    let owner = f.owner().await;
    let agent = f
        .store
        .create_agent(f.ctx(&owner), AgentCreate { label: "a".into() })
        .await
        .unwrap()
        .value
        .actor;
    let (project, task, type_id) = seed_task(&f, &owner).await;

    let err = f
        .store
        .update_task(
            f.ctx(&agent),
            project,
            task,
            TaskPatch {
                title: None,
                description: None,
                type_key: None,
                planning_required: None,
                plan_review: Some(ReviewPolicy::None),
                work_review: Some(ReviewPolicy::None),
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::Forbidden(_)));

    let err = f
        .store
        .update_task_type(
            f.ctx(&agent),
            project,
            type_id,
            TaskTypePatch {
                label: Some("weakened".into()),
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::Forbidden(_)));

    let mut conn = f.independent_connection().await;
    assert_eq!(revision_of(&mut conn, "tasks", task.as_uuid()).await, 1);
    assert_eq!(
        revision_of(&mut conn, "task_types", type_id.as_uuid()).await,
        1
    );
    assert_eq!(
        revision_of(&mut conn, "projects", project.as_uuid()).await,
        1
    );
}

// Acceptance: agent cannot approve human review. The review command lands in
// step 012; the capability matrix is the enforcement point that exists today.
#[test]
fn agent_is_denied_human_review_capability() {
    assert!(!base_allow(ActorKind::Agent, Capability::ReviewHumanPolicy));
    assert!(base_allow(ActorKind::Human, Capability::ReviewHumanPolicy));
    // And the independent-review rule cuts the other way for agent policy.
    assert!(!base_allow(ActorKind::Human, Capability::ReviewAgentPolicy));
    let now = "2026-09-14T00:00:00Z".parse().unwrap();
    let agent = Actor {
        id: shepherd_core::model::ActorId::generate(now),
        kind: ActorKind::Agent,
        label: "a".into(),
        revoked: false,
        created_at: now,
    };
    assert!(matches!(
        require_capability(&agent, Capability::ReviewHumanPolicy),
        Err(DomainError::Forbidden(_))
    ));
}

// Acceptance: same key / different actor cannot replay another response.
#[tokio::test]
async fn same_key_different_actor_does_not_replay_foreign_response() {
    let f = fixture().await;
    let owner = f.owner().await;
    let second_owner = testing::seed_actor(&f.store, ActorKind::Human, "second").await;
    let first_agent = f
        .store
        .create_agent(f.ctx(&owner), AgentCreate { label: "a".into() })
        .await
        .unwrap()
        .value
        .actor;
    let second_agent = f
        .store
        .create_agent(f.ctx(&owner), AgentCreate { label: "b".into() })
        .await
        .unwrap()
        .value
        .actor;
    let key = Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext));
    let mut ctx = f.ctx(&owner);
    ctx.idempotency_key = key;
    let first = f
        .store
        .revoke_agent(ctx, first_agent.id, "rogue".into(), "hash-first")
        .await
        .unwrap();
    assert!(!first.is_replay());

    // The other human reuses the same key for a different revocation: it
    // executes fresh instead of replaying the first actor's sealed response.
    let mut ctx = f.ctx(&second_owner);
    ctx.idempotency_key = key;
    let second = f
        .store
        .revoke_agent(ctx, second_agent.id, "rogue".into(), "hash-second")
        .await
        .unwrap();
    assert!(!second.is_replay());
    let mut conn = f.independent_connection().await;
    assert_eq!(count(&mut conn, "idempotency").await, 2);
    let revoked: i64 = sqlx::query_scalar("SELECT count(*) FROM actors WHERE revoked = 1")
        .fetch_one(&mut conn)
        .await
        .unwrap();
    assert_eq!(revoked, 2);
}

// Acceptance: revoked token fails before replay. The revoked credential is
// rejected everywhere, including a retry of its own stored idempotency key.
#[tokio::test]
async fn revoked_agent_token_is_unauthorized_everywhere() {
    let f = fixture().await;
    let owner = f.owner().await;
    let grant = f
        .store
        .create_agent(f.ctx(&owner), AgentCreate { label: "a".into() })
        .await
        .unwrap()
        .value;
    f.store
        .revoke_agent(f.ctx(&owner), grant.actor.id, "rogue".into(), "hash")
        .await
        .unwrap();
    assert!(matches!(
        f.store.authenticate_bearer(&grant.token).await.unwrap_err(),
        DomainError::Unauthenticated
    ));
    assert!(matches!(
        f.store
            .login_browser(&grant.token, f.clock.now())
            .await
            .unwrap_err(),
        DomainError::Unauthenticated
    ));
}

#[tokio::test]
async fn idempotency_conflict_on_same_key_different_request() {
    let f = fixture().await;
    let owner = f.owner().await;
    let first = f
        .store
        .create_agent(f.ctx(&owner), AgentCreate { label: "a".into() })
        .await
        .unwrap()
        .value
        .actor;
    let second = f
        .store
        .create_agent(f.ctx(&owner), AgentCreate { label: "b".into() })
        .await
        .unwrap()
        .value
        .actor;
    let key = Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext));
    let mut ctx = f.ctx(&owner);
    ctx.idempotency_key = key;
    f.store
        .revoke_agent(ctx, first.id, "rogue".into(), "hash-first")
        .await
        .unwrap();
    let mut ctx = f.ctx(&owner);
    ctx.idempotency_key = key;
    let err = f
        .store
        .revoke_agent(ctx, second.id, "rogue".into(), "hash-second")
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::IdempotencyConflict));
    let mut conn = f.independent_connection().await;
    let second_revoked: i64 = sqlx::query_scalar("SELECT revoked FROM actors WHERE id = ?1")
        .bind(second.id.to_string())
        .fetch_one(&mut conn)
        .await
        .unwrap();
    assert_eq!(second_revoked, 0);
}

#[tokio::test]
async fn revoking_a_revoked_agent_is_terminal() {
    let f = fixture().await;
    let owner = f.owner().await;
    let agent = f
        .store
        .create_agent(f.ctx(&owner), AgentCreate { label: "a".into() })
        .await
        .unwrap()
        .value
        .actor;
    f.store
        .revoke_agent(f.ctx(&owner), agent.id, "rogue".into(), "hash-1")
        .await
        .unwrap();
    let err = f
        .store
        .revoke_agent(f.ctx(&owner), agent.id, "again".into(), "hash-2")
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::TerminalScope));
    let mut conn = f.independent_connection().await;
    let audits: i64 =
        sqlx::query_scalar("SELECT count(*) FROM security_audit WHERE action = 'revokeAgent'")
            .fetch_one(&mut conn)
            .await
            .unwrap();
    assert_eq!(audits, 1);
}

// Acceptance: browser session expiry is fixed at login; GET does not slide it.
#[tokio::test]
async fn browser_session_expires_at_fixed_12h_without_sliding() {
    let f = fixture().await;
    f.owner().await;
    let token = SecretString::new(std::fs::read_to_string(f.paths().owner_token()).unwrap());
    let login = f.store.login_browser(&token, f.clock.now()).await.unwrap();
    let expiry = login.session.expires_at;
    assert_eq!(
        expiry,
        f.clock.now() + chrono::TimeDelta::hours(12),
        "fixed 12h TTL"
    );
    f.clock.advance(chrono::TimeDelta::hours(11));
    let session = f
        .store
        .browser_session(&login.session_token, f.clock.now())
        .await
        .unwrap();
    assert_eq!(session.expires_at, expiry);
    f.clock
        .advance(chrono::TimeDelta::hours(1) + chrono::TimeDelta::seconds(1));
    assert!(matches!(
        f.store
            .browser_session(&login.session_token, f.clock.now())
            .await
            .unwrap_err(),
        DomainError::Unauthenticated
    ));
}

// Acceptance: owner token file permissions are 0600 under a 0700 data dir
// (the replay key variant is covered beside its provider in storage tests).
#[tokio::test]
async fn owner_token_file_has_restrictive_permissions() {
    let f = fixture().await;
    f.owner().await;
    assert_eq!(file_mode(&f.paths().owner_token()), 0o600);
}

#[tokio::test]
async fn missing_owner_token_file_fails_loud_and_reissue_recovers() {
    let f = fixture().await;
    let owner = f.owner().await;
    std::fs::remove_file(f.paths().owner_token()).unwrap();
    let err = f.store.ensure_owner(&f.paths()).await.unwrap_err();
    assert!(err.to_string().contains("--reissue-owner-token"), "{err}");
    let recovered = f.store.reissue_owner_token(&f.paths()).await.unwrap();
    assert_eq!(recovered.actor.id, owner.id);
    f.store.ensure_owner(&f.paths()).await.unwrap();
    let token = SecretString::new(std::fs::read_to_string(f.paths().owner_token()).unwrap());
    assert_eq!(
        f.store.authenticate_bearer(&token).await.unwrap().id,
        owner.id
    );
}

async fn seed_task(
    f: &Fixture,
    owner: &Actor,
) -> (ProjectId, TaskId, shepherd_core::model::TaskTypeId) {
    let project = f
        .store
        .create_project(
            f.ctx(owner),
            ProjectCreate {
                name: "P".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value
        .id;
    let goal = f
        .store
        .create_goal(
            f.ctx(owner),
            project,
            GoalCreate {
                title: "G".to_string(),
                description: None,
            },
        )
        .await
        .unwrap()
        .value
        .id;
    let epic = f
        .store
        .create_epic(
            f.ctx(owner),
            project,
            goal,
            shepherd_core::model::EpicCreate {
                title: "E".to_string(),
                description: None,
            },
        )
        .await
        .unwrap()
        .value
        .id;
    let task = f
        .store
        .create_task(
            f.ctx(owner),
            project,
            epic,
            TaskCreate {
                title: "T".to_string(),
                description: None,
                type_key: "code".to_string(),
                planning_required: None,
                plan_review: None,
                work_review: None,
            },
        )
        .await
        .unwrap()
        .value
        .id;
    let types = f
        .store
        .list_task_types(&project, &Default::default())
        .await
        .unwrap()
        .items;
    let type_id = types
        .iter()
        .find(|t| t.key == "code")
        .expect("builtin code type")
        .id;
    (project, task, type_id)
}
