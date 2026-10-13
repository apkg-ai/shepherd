use std::sync::Arc;

use chrono::TimeDelta;

use super::*;
use crate::model::{
    Actor, ActorKind, Clock, CommandId, EpicCreate, EpicId, GoalCreate, ProjectCreate,
    ReviewPolicy, Task, TaskCreate, TestClock,
};
use crate::storage::open;
use crate::storage::rows::insert_actor;
use crate::storage::testing::{seed_claim, seed_submission, store_options, test_clock};

struct Fixture {
    _dir: tempfile::TempDir,
    clock: Arc<TestClock>,
    store: Store,
    owner: Actor,
    project: ProjectId,
    epic: EpicId,
    task: Task,
}

fn ctx(actor: &Actor, clock: &TestClock, revision: Option<i64>) -> CommandContext {
    CommandContext {
        actor: actor.clone(),
        command_id: CommandId::generate(clock.now()),
        idempotency_key: Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)),
        expected_revision: revision,
        now: clock.now(),
    }
}

async fn fixture(planning_required: bool) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let clock = test_clock();
    let store = open(store_options(dir.path(), "shepherd.db", clock.clone()))
        .await
        .unwrap();
    let owner = Actor {
        id: crate::model::ActorId::generate(clock.now()),
        kind: ActorKind::Human,
        label: "owner".to_string(),
        revoked: false,
        created_at: clock.now(),
    };
    let registered = owner.clone();
    store
        .command_transaction(|tx| Box::pin(async move { insert_actor(tx, &registered).await }))
        .await
        .unwrap();
    let project = store
        .create_project(
            ctx(&owner, &clock, None),
            ProjectCreate {
                name: "P".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value
        .id;
    let goal = store
        .create_goal(
            ctx(&owner, &clock, None),
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
    let epic = store
        .create_epic(
            ctx(&owner, &clock, None),
            project,
            goal,
            EpicCreate {
                title: "E".to_string(),
                description: None,
            },
        )
        .await
        .unwrap()
        .value
        .id;
    let task = store
        .create_task(
            ctx(&owner, &clock, None),
            project,
            epic,
            TaskCreate {
                title: "T".to_string(),
                type_key: "code".to_string(),
                planning_required: Some(planning_required),
                plan_review: Some(ReviewPolicy::None),
                work_review: Some(ReviewPolicy::None),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value;
    Fixture {
        _dir: dir,
        clock,
        store,
        owner,
        project,
        epic,
        task,
    }
}

fn input(owner: DocumentOwner, kind: DocumentKind) -> DocumentCreate {
    DocumentCreate {
        owner,
        kind,
        title: "Doc".to_string(),
        body: "# body".to_string(),
        links: Vec::new(),
    }
}

async fn doc(f: &Fixture, owner: DocumentOwner, kind: DocumentKind) -> Document {
    f.clock.advance(TimeDelta::milliseconds(2));
    f.store
        .create_document(
            ctx(&f.owner, &f.clock, None),
            f.project,
            input(owner, kind),
            "h",
        )
        .await
        .unwrap()
        .into_inner()
}

async fn append(
    f: &Fixture,
    document: &Document,
    revision: Option<i64>,
    body: &str,
) -> Result<DocumentRevision, DomainError> {
    f.clock.advance(TimeDelta::milliseconds(2));
    f.store
        .create_document_revision(
            ctx(&f.owner, &f.clock, revision),
            f.project,
            document.id,
            RevisionCreate {
                body: body.to_string(),
                links: Vec::new(),
            },
            "rev-h",
        )
        .await
        .map(Replay::into_inner)
}

async fn select(f: &Fixture, revision_id: Uuid) -> Result<Task, DomainError> {
    f.clock.advance(TimeDelta::milliseconds(2));
    let current: i64 = sqlx::query_scalar("SELECT revision FROM tasks WHERE id = ?1")
        .bind(f.task.id.to_string())
        .fetch_one(f.store.pool())
        .await
        .unwrap();
    f.store
        .select_task_plan(
            ctx(&f.owner, &f.clock, Some(current)),
            f.project,
            f.task.id,
            SelectPlanInput {
                document_revision_id: revision_id,
            },
        )
        .await
        .map(|result| result.value)
}

async fn plan_state(f: &Fixture) -> (Option<String>, Option<String>, String) {
    sqlx::query_as(
        "SELECT selected_plan_revision_id, accepted_plan_submission_id, phase \
             FROM tasks WHERE id = ?1",
    )
    .bind(f.task.id.to_string())
    .fetch_one(f.store.pool())
    .await
    .unwrap()
}

async fn output_gate(
    store: &Store,
    project: ProjectId,
    task: TaskId,
    ids: &[Uuid],
) -> Result<(), DomainError> {
    let ids = ids.to_vec();
    store
        .domain_transaction(move |tx| {
            Box::pin(async move {
                let kinds = task_owned_revision_kinds(tx, &project, &task, &ids).await?;
                require_output_kinds(&kinds)
            })
        })
        .await
}

#[tokio::test]
async fn documents_are_created_for_all_owner_kinds() {
    let f = fixture(false).await;
    for (owner, kind) in [
        (DocumentOwner::Project(f.project), DocumentKind::Brief),
        (DocumentOwner::Epic(f.epic), DocumentKind::Decision),
        (DocumentOwner::Task(f.task.id), DocumentKind::Plan),
    ] {
        let document = doc(&f, owner, kind).await;
        assert_eq!(document.revision, Revision::INITIAL);
        assert_eq!(document.owner_kind, owner.kind());
        assert_eq!(document.owner_id, owner.id());
        let revision = f
            .store
            .get_document_revision(&f.project, &document.latest_revision_id)
            .await
            .unwrap();
        assert_eq!(revision.number, 1);
    }
}

#[tokio::test]
async fn unknown_owners_are_not_found() {
    let f = fixture(false).await;
    let other_project = ProjectId::generate(f.clock.now());
    for owner in [
        DocumentOwner::Project(other_project),
        DocumentOwner::Epic(EpicId::generate(f.clock.now())),
        DocumentOwner::Task(TaskId::generate(f.clock.now())),
    ] {
        let err = f
            .store
            .create_document(
                ctx(&f.owner, &f.clock, None),
                f.project,
                input(owner, DocumentKind::Note),
                "h",
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::NotFound), "{owner:?}");
    }
}

#[tokio::test]
async fn invalid_content_is_rejected_by_the_command() {
    let f = fixture(false).await;
    let cases = [
        DocumentCreate {
            title: String::new(),
            ..input(DocumentOwner::Task(f.task.id), DocumentKind::Note)
        },
        DocumentCreate {
            body: "x".repeat(65_537),
            ..input(DocumentOwner::Task(f.task.id), DocumentKind::Note)
        },
        DocumentCreate {
            links: vec!["javascript:alert(1)".to_string()],
            ..input(DocumentOwner::Task(f.task.id), DocumentKind::Note)
        },
    ];
    for bad in cases {
        let err = f
            .store
            .create_document(ctx(&f.owner, &f.clock, None), f.project, bad, "h")
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::Validation { .. }));
    }
    let documents: i64 = sqlx::query_scalar("SELECT count(*) FROM documents")
        .fetch_one(f.store.pool())
        .await
        .unwrap();
    assert_eq!(documents, 0);
}

#[tokio::test]
async fn appends_bump_the_document_and_enforce_if_match() {
    let f = fixture(false).await;
    let document = doc(&f, DocumentOwner::Task(f.task.id), DocumentKind::Note).await;
    let second = append(&f, &document, Some(1), "v2").await.unwrap();
    assert_eq!(second.number, 2);
    let reloaded = f
        .store
        .get_document(&f.project, &document.id)
        .await
        .unwrap();
    assert_eq!(reloaded.revision.value(), 2);
    assert_eq!(reloaded.latest_revision_id, second.id);

    let err = append(&f, &document, Some(1), "stale").await.unwrap_err();
    assert!(matches!(
        err,
        DomainError::RevisionConflict {
            expected: 1,
            actual: 2
        }
    ));
    let err = append(&f, &document, None, "no precondition")
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::PreconditionRequired));
    let err = append(&f, &document, Some(2), "x".repeat(65_537).as_str())
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::Validation { field: "body", .. }));
}

#[tokio::test]
async fn terminal_owner_accepts_only_notes() {
    let f = fixture(false).await;
    let finding = doc(&f, DocumentOwner::Task(f.task.id), DocumentKind::Finding).await;
    f.clock.advance(TimeDelta::milliseconds(2));
    f.store
        .complete_task_for_test(ctx(&f.owner, &f.clock, Some(1)), f.project, f.task.id)
        .await
        .unwrap();

    let note = doc(&f, DocumentOwner::Task(f.task.id), DocumentKind::Note).await;
    assert!(append(&f, &note, Some(1), "still fine").await.is_ok());

    let err = f
        .store
        .create_document(
            ctx(&f.owner, &f.clock, None),
            f.project,
            input(DocumentOwner::Task(f.task.id), DocumentKind::Plan),
            "h",
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::TerminalScope));
    let err = append(&f, &finding, Some(1), "late").await.unwrap_err();
    assert!(matches!(err, DomainError::TerminalScope));
}

#[tokio::test]
async fn archived_scope_rejects_creates_appends_and_selection() {
    let f = fixture(true).await;
    let plan = doc(&f, DocumentOwner::Task(f.task.id), DocumentKind::Plan).await;
    f.clock.advance(TimeDelta::milliseconds(2));
    f.store
        .cancel_task(
            ctx(&f.owner, &f.clock, Some(1)),
            f.project,
            f.task.id,
            "obsolete".into(),
        )
        .await
        .unwrap();
    f.clock.advance(TimeDelta::milliseconds(2));
    f.store
        .archive_task(
            ctx(&f.owner, &f.clock, Some(2)),
            f.project,
            f.task.id,
            "cleanup".into(),
        )
        .await
        .unwrap();

    let err = f
        .store
        .create_document(
            ctx(&f.owner, &f.clock, None),
            f.project,
            input(DocumentOwner::Task(f.task.id), DocumentKind::Note),
            "h",
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::ArchivedScope));
    let err = append(&f, &plan, Some(1), "late").await.unwrap_err();
    assert!(matches!(err, DomainError::ArchivedScope));
    let err = select(&f, plan.latest_revision_id.as_uuid())
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::ArchivedScope));
}

#[tokio::test]
async fn archived_project_and_epic_owners_reject_mutations() {
    let f = fixture(false).await;
    let epic_doc = doc(&f, DocumentOwner::Epic(f.epic), DocumentKind::Brief).await;

    // A second, work-free project: archiving it must not touch the
    // fixture task, and archive_project refuses nonterminal work.
    f.clock.advance(TimeDelta::milliseconds(2));
    let empty = f
        .store
        .create_project(
            ctx(&f.owner, &f.clock, None),
            ProjectCreate {
                name: "Empty".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value;
    f.clock.advance(TimeDelta::milliseconds(2));
    let project_doc = f
        .store
        .create_document(
            ctx(&f.owner, &f.clock, None),
            empty.id,
            input(DocumentOwner::Project(empty.id), DocumentKind::Decision),
            "h",
        )
        .await
        .unwrap()
        .into_inner();

    f.clock.advance(TimeDelta::milliseconds(2));
    f.store
        .archive_project(
            ctx(&f.owner, &f.clock, Some(empty.revision.value())),
            empty.id,
            "cleanup".into(),
        )
        .await
        .unwrap();
    let err = f
        .store
        .create_document(
            ctx(&f.owner, &f.clock, None),
            empty.id,
            input(DocumentOwner::Project(empty.id), DocumentKind::Note),
            "h",
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::ArchivedScope));
    // append() is fixture-project bound; the project-owned document
    // lives in the second project.
    f.clock.advance(TimeDelta::milliseconds(2));
    let err = f
        .store
        .create_document_revision(
            ctx(&f.owner, &f.clock, Some(1)),
            empty.id,
            project_doc.id,
            RevisionCreate {
                body: "late".to_string(),
                links: Vec::new(),
            },
            "rev-h",
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::ArchivedScope));

    // The epic owner needs a terminal epic before it can archive.
    f.clock.advance(TimeDelta::milliseconds(2));
    let epic_revision: i64 = sqlx::query_scalar("SELECT revision FROM epics WHERE id = ?1")
        .bind(f.epic.to_string())
        .fetch_one(f.store.pool())
        .await
        .unwrap();
    f.store
        .cancel_epic(
            ctx(&f.owner, &f.clock, Some(epic_revision)),
            f.project,
            f.epic,
            "obsolete".into(),
        )
        .await
        .unwrap();
    f.clock.advance(TimeDelta::milliseconds(2));
    let epic_revision: i64 = sqlx::query_scalar("SELECT revision FROM epics WHERE id = ?1")
        .bind(f.epic.to_string())
        .fetch_one(f.store.pool())
        .await
        .unwrap();
    f.store
        .archive_epic(
            ctx(&f.owner, &f.clock, Some(epic_revision)),
            f.project,
            f.epic,
            "cleanup".into(),
        )
        .await
        .unwrap();
    let err = f
        .store
        .create_document(
            ctx(&f.owner, &f.clock, None),
            f.project,
            input(DocumentOwner::Epic(f.epic), DocumentKind::Note),
            "h",
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::ArchivedScope));
    let err = append(&f, &epic_doc, Some(1), "late").await.unwrap_err();
    assert!(matches!(err, DomainError::ArchivedScope));
}

#[tokio::test]
async fn execute_claims_block_selected_plan_appends_only() {
    let f = fixture(true).await;
    let plan = doc(&f, DocumentOwner::Task(f.task.id), DocumentKind::Plan).await;
    let notes = doc(&f, DocumentOwner::Task(f.task.id), DocumentKind::Note).await;
    select(&f, plan.latest_revision_id.as_uuid()).await.unwrap();

    let now = format_ts(&f.clock.now());
    let expires = format_ts(&(f.clock.now() + TimeDelta::seconds(300)));
    seed_claim(
        f.store.pool(),
        f.task.id,
        f.owner.id,
        "execute",
        "active",
        &now,
        &expires,
    )
    .await;

    let err = append(&f, &plan, Some(1), "v2").await.unwrap_err();
    assert!(matches!(err, DomainError::ActiveWork(_)));
    assert!(append(&f, &notes, Some(1), "fine").await.is_ok());

    // An expired lease no longer blocks.
    f.clock.advance(TimeDelta::seconds(1_000));
    assert!(append(&f, &plan, Some(1), "v2").await.is_ok());
}

#[tokio::test]
async fn selection_persists_clears_acceptance_and_noops_on_same_revision() {
    let f = fixture(true).await;
    let plan = doc(&f, DocumentOwner::Task(f.task.id), DocumentKind::Plan).await;
    let v1 = plan.latest_revision_id.as_uuid();
    let v2 = append(&f, &plan, Some(1), "v2").await.unwrap().id.as_uuid();

    let updated = select(&f, v1).await.unwrap();
    assert_eq!(updated.phase, crate::model::TaskPhase::Planning);
    let (selected, _, _) = plan_state(&f).await;
    assert_eq!(selected.as_deref(), Some(v1.to_string().as_str()));

    // Simulate an accepted plan, then change the selection: acceptance clears.
    let now = format_ts(&f.clock.now());
    let submission = seed_submission(f.store.pool(), f.task.id, f.owner.id, "accepted", &now).await;
    sqlx::query(
        "UPDATE tasks SET accepted_plan_submission_id = ?1, phase = 'execution' WHERE id = ?2",
    )
    .bind(&submission)
    .bind(f.task.id.to_string())
    .execute(f.store.pool())
    .await
    .unwrap();

    let updated = select(&f, v2).await.unwrap();
    assert_eq!(updated.phase, crate::model::TaskPhase::Planning);
    let (selected, accepted, phase) = plan_state(&f).await;
    assert_eq!(selected.as_deref(), Some(v2.to_string().as_str()));
    assert_eq!(accepted, None);
    assert_eq!(phase, "planning");

    // Re-selecting the same revision changes nothing and emits no event.
    let events_before: i64 = sqlx::query_scalar("SELECT count(*) FROM events")
        .fetch_one(f.store.pool())
        .await
        .unwrap();
    let revision_before = updated.revision;
    let noop = select(&f, v2).await.unwrap();
    assert_eq!(noop.revision, revision_before);
    let events_after: i64 = sqlx::query_scalar("SELECT count(*) FROM events")
        .fetch_one(f.store.pool())
        .await
        .unwrap();
    assert_eq!(events_after, events_before);
}

#[tokio::test]
async fn selection_guards_reject_invalid_targets_and_active_work() {
    let f = fixture(true).await;
    let plan = doc(&f, DocumentOwner::Task(f.task.id), DocumentKind::Plan).await;
    let note = doc(&f, DocumentOwner::Task(f.task.id), DocumentKind::Note).await;
    let v1 = plan.latest_revision_id.as_uuid();

    let err = select(&f, note.latest_revision_id.as_uuid())
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        DomainError::Validation {
            field: "document_revision_id",
            ..
        }
    ));
    let err = select(&f, Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)))
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        DomainError::Validation {
            field: "document_revision_id",
            ..
        }
    ));

    // Stale If-Match and missing precondition.
    f.clock.advance(TimeDelta::milliseconds(2));
    let err = f
        .store
        .select_task_plan(
            ctx(&f.owner, &f.clock, Some(99)),
            f.project,
            f.task.id,
            SelectPlanInput {
                document_revision_id: v1,
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::RevisionConflict { .. }));
    let err = f
        .store
        .select_task_plan(
            ctx(&f.owner, &f.clock, None),
            f.project,
            f.task.id,
            SelectPlanInput {
                document_revision_id: v1,
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::PreconditionRequired));

    // Unknown task is a 404.
    let err = f
        .store
        .select_task_plan(
            ctx(&f.owner, &f.clock, Some(1)),
            f.project,
            TaskId::generate(f.clock.now()),
            SelectPlanInput {
                document_revision_id: v1,
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::NotFound));

    // Pending submissions and review claims block selection.
    let now = format_ts(&f.clock.now());
    let submission = seed_submission(f.store.pool(), f.task.id, f.owner.id, "pending", &now).await;
    let err = select(&f, v1).await.unwrap_err();
    assert!(matches!(err, DomainError::ActiveWork(_)));
    sqlx::query("DELETE FROM submissions WHERE id = ?1")
        .bind(&submission)
        .execute(f.store.pool())
        .await
        .unwrap();
    let expires = format_ts(&(f.clock.now() + TimeDelta::seconds(300)));
    seed_claim(
        f.store.pool(),
        f.task.id,
        f.owner.id,
        "review",
        "active",
        &now,
        &expires,
    )
    .await;
    let err = select(&f, v1).await.unwrap_err();
    assert!(matches!(err, DomainError::ActiveWork(_)));
}

#[tokio::test]
async fn selection_requires_planning() {
    let f = fixture(false).await;
    let plan = doc(&f, DocumentOwner::Task(f.task.id), DocumentKind::Plan).await;
    let err = select(&f, plan.latest_revision_id.as_uuid())
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        DomainError::Validation {
            field: "planning_required",
            ..
        }
    ));
}

#[tokio::test]
async fn revoked_actors_cannot_select() {
    let f = fixture(true).await;
    let plan = doc(&f, DocumentOwner::Task(f.task.id), DocumentKind::Plan).await;
    sqlx::query("UPDATE actors SET revoked = 1 WHERE id = ?1")
        .bind(f.owner.id.to_string())
        .execute(f.store.pool())
        .await
        .unwrap();
    let err = select(&f, plan.latest_revision_id.as_uuid())
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::Forbidden(_)));
}

#[tokio::test]
async fn reads_are_project_scoped_and_lists_filter_and_paginate() {
    let f = fixture(false).await;
    let task_doc = doc(&f, DocumentOwner::Task(f.task.id), DocumentKind::Note).await;
    let epic_doc = doc(&f, DocumentOwner::Epic(f.epic), DocumentKind::Brief).await;
    let other_project = f
        .store
        .create_project(
            ctx(&f.owner, &f.clock, None),
            ProjectCreate {
                name: "Other".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value
        .id;

    assert!(matches!(
        f.store.get_document(&other_project, &task_doc.id).await,
        Err(DomainError::NotFound)
    ));
    assert!(matches!(
        f.store
            .get_document_revision(&other_project, &task_doc.latest_revision_id)
            .await,
        Err(DomainError::NotFound)
    ));
    assert!(matches!(
        f.store
            .list_document_revisions(&other_project, &task_doc.id, &ListParams::default())
            .await,
        Err(DomainError::NotFound)
    ));

    let by_owner = f
        .store
        .list_documents(
            &f.project,
            &DocumentListFilters {
                owner_kind: Some(DocumentOwnerKind::Epic),
                owner_id: Some(f.epic.as_uuid()),
                kind: Some(DocumentKind::Brief),
            },
            &ListParams::default(),
        )
        .await
        .unwrap();
    assert_eq!(by_owner.items.len(), 1);
    assert_eq!(by_owner.items[0].id, epic_doc.id);

    let page_one = f
        .store
        .list_documents(
            &f.project,
            &DocumentListFilters::default(),
            &ListParams {
                limit: Some(1),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(page_one.items.len(), 1);
    assert_eq!(page_one.items[0].id, task_doc.id, "created_at ascending");
    let cursor = page_one.next_cursor.expect("two documents exceed the page");
    let page_two = f
        .store
        .list_documents(
            &f.project,
            &DocumentListFilters::default(),
            &ListParams {
                limit: Some(1),
                cursor: Some(cursor.clone()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(page_two.items[0].id, epic_doc.id);
    let err = f
        .store
        .list_documents(
            &f.project,
            &DocumentListFilters {
                kind: Some(DocumentKind::Note),
                ..Default::default()
            },
            &ListParams {
                cursor: Some(cursor),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::InvalidCursor(_)));

    append(&f, &task_doc, Some(1), "v2").await.unwrap();
    append(&f, &task_doc, Some(2), "v3").await.unwrap();
    let newest_first = f
        .store
        .list_document_revisions(
            &f.project,
            &task_doc.id,
            &ListParams {
                limit: Some(2),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let numbers: Vec<i64> = newest_first.items.iter().map(|r| r.number).collect();
    assert_eq!(numbers, vec![3, 2]);
    let rest = f
        .store
        .list_document_revisions(
            &f.project,
            &task_doc.id,
            &ListParams {
                limit: Some(2),
                cursor: newest_first.next_cursor,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let numbers: Vec<i64> = rest.items.iter().map(|r| r.number).collect();
    assert_eq!(numbers, vec![1]);
}

#[tokio::test]
async fn duplicate_owned_revision_ids_validate() {
    let f = fixture(false).await;
    let document = doc(&f, DocumentOwner::Task(f.task.id), DocumentKind::Finding).await;
    let owned = document.latest_revision_id.as_uuid();
    let project = f.project;
    let task_id = f.task.id;
    f.store
        .domain_transaction(move |tx| {
            Box::pin(async move {
                task_owned_revision_kinds(tx, &project, &task_id, &[owned, owned])
                    .await
                    .map(|kinds| assert_eq!(kinds, vec![DocumentKind::Finding]))
            })
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn revoked_actors_get_forbidden_before_membership() {
    let f = fixture(true).await;
    sqlx::query("UPDATE actors SET revoked = 1 WHERE id = ?1")
        .bind(f.owner.id.to_string())
        .execute(f.store.pool())
        .await
        .unwrap();
    // A revoked actor must not learn whether a task id exists.
    let err = f
        .store
        .select_task_plan(
            ctx(&f.owner, &f.clock, Some(1)),
            f.project,
            TaskId::generate(f.clock.now()),
            SelectPlanInput {
                document_revision_id: Uuid::nil(),
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::Forbidden(_)));
}

#[tokio::test]
async fn unknown_project_lists_are_not_found_before_cursor_checks() {
    let f = fixture(false).await;
    let document = doc(&f, DocumentOwner::Task(f.task.id), DocumentKind::Note).await;
    let unknown = ProjectId::generate(f.clock.now());
    let garbage_cursor = ListParams {
        cursor: Some("not-a-cursor".into()),
        ..Default::default()
    };
    let err = f
        .store
        .list_documents(
            &unknown,
            &DocumentListFilters::default(),
            &ListParams::default(),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::NotFound));
    let err = f
        .store
        .list_documents(&unknown, &DocumentListFilters::default(), &garbage_cursor)
        .await
        .unwrap_err();
    assert!(
        matches!(err, DomainError::NotFound),
        "membership precedes cursor validity"
    );
    let err = f
        .store
        .list_document_revisions(&unknown, &document.id, &garbage_cursor)
        .await
        .unwrap_err();
    assert!(
        matches!(err, DomainError::NotFound),
        "membership precedes cursor validity"
    );
}

#[tokio::test]
async fn task_owned_revision_validation_reports_the_missing_id() {
    let f = fixture(false).await;
    let document = doc(&f, DocumentOwner::Task(f.task.id), DocumentKind::Finding).await;
    let owned = document.latest_revision_id.as_uuid();
    let foreign = Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext));
    let project = f.project;
    let task_id = f.task.id;
    let err = f
        .store
        .domain_transaction(move |tx| {
            Box::pin(async move {
                task_owned_revision_kinds(tx, &project, &task_id, &[owned, foreign]).await
            })
        })
        .await
        .unwrap_err();
    match err {
        DomainError::Validation { field, message } => {
            assert_eq!(field, "document_revision_ids");
            assert!(message.contains(&foreign.to_string()));
        }
        other => panic!("expected validation error, got {other:?}"),
    }
}

#[tokio::test]
async fn output_revision_gate_is_scoped_without_separate_ownership_checks() {
    let f = fixture(false).await;
    let finding = doc(&f, DocumentOwner::Task(f.task.id), DocumentKind::Finding).await;
    let note = doc(&f, DocumentOwner::Task(f.task.id), DocumentKind::Note).await;
    let finding_rev = finding.latest_revision_id.as_uuid();
    let note_rev = note.latest_revision_id.as_uuid();
    let project = f.project;
    let task_id = f.task.id;

    output_gate(&f.store, project, task_id, &[finding_rev])
        .await
        .unwrap();

    // A foreign task id must fail the gate on its own.
    let foreign_task = TaskId::generate(f.clock.now());
    let err = output_gate(&f.store, project, foreign_task, &[finding_rev])
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        DomainError::Validation {
            field: "document_revision_ids",
            ..
        }
    ));

    // A foreign project id must fail the gate on its own.
    let foreign_project = ProjectId::generate(f.clock.now());
    let err = output_gate(&f.store, foreign_project, task_id, &[finding_rev])
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        DomainError::Validation {
            field: "document_revision_ids",
            ..
        }
    ));

    // Task-owned but not an output kind, and the empty list.
    let err = output_gate(&f.store, project, task_id, &[note_rev])
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        DomainError::Validation {
            field: "document_revision_ids",
            ..
        }
    ));
    let err = output_gate(&f.store, project, task_id, &[])
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        DomainError::Validation {
            field: "document_revision_ids",
            ..
        }
    ));
}
