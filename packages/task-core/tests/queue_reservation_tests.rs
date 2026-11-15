use task_core::{ExecutionMode, FsTaskStore, QueueState, TaskStore, TaskStoreError};
use std::sync::{Arc, Barrier};

fn queued_task(root: &std::path::Path) -> (FsTaskStore, task_core::Task, String) {
    let store = FsTaskStore;
    store.ensure_board(root, "T3 dispatcher reservations").unwrap();
    let task = store.create(root, serde_json::from_value(serde_json::json!({
        "title": "Reserved queue work", "column": "ai-ready", "parent": null
    })).unwrap()).unwrap();
    let intent = store.set_execution_mode(root, &task.id, &task.revision, ExecutionMode::Queue).unwrap();
    (store, task, intent.queue_request.unwrap())
}

#[test]
fn concurrent_reservations_have_exactly_one_winner() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let (store, task, queue_request) = queued_task(&root);
    let ids = [uuid::Uuid::new_v4().to_string(), uuid::Uuid::new_v4().to_string()];
    let barrier = Arc::new(Barrier::new(2));
    let handles = ids.into_iter().map(|request_id| {
        let root = root.clone();
        let barrier = barrier.clone();
        let id = task.id.clone();
        let queue_request = queue_request.clone();
        std::thread::spawn(move || {
            let store = FsTaskStore;
            barrier.wait();
            store.reserve_queue_task(&root, &id, &queue_request, &request_id, 180)
                .map(|_| request_id)
        })
    }).collect::<Vec<_>>();
    let outcomes = handles.into_iter().map(|handle| handle.join().unwrap()).collect::<Vec<_>>();
    assert_eq!(outcomes.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(outcomes.iter().filter(|result| matches!(result, Err(TaskStoreError::Conflict(_)))).count(), 1);
    let current = store.execution_intent(&root, &task.id).unwrap();
    assert_eq!(current.queue_state, Some(QueueState::Reserved));
    assert_eq!(current.queue_reservation_id.as_deref(), outcomes.iter()
        .find_map(|result| result.as_ref().ok().map(String::as_str)));
}

#[test]
fn reserved_starting_and_working_lifecycle_is_idempotent_and_claim_is_atomic() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let (store, task, queue_request) = queued_task(root);
    let reservation = uuid::Uuid::new_v4().to_string();
    let session = uuid::Uuid::new_v4().to_string();

    let reserved = store.reserve_queue_task(root, &task.id, &queue_request, &reservation, 180).unwrap();
    assert_eq!(reserved.queue_state, Some(QueueState::Reserved));
    assert_eq!(reserved.queue_reservation_id.as_deref(), Some(reservation.as_str()));
    assert!(reserved.queue_reservation_expires_at.is_some());
    assert!(matches!(store.reserve_queue_task(root, &task.id, &queue_request,
        &uuid::Uuid::new_v4().to_string(), 180), Err(TaskStoreError::Conflict(_))));
    assert!(matches!(store.claim(root, &task.id, "manual-agent"),
        Err(TaskStoreError::Conflict(_))));

    let starting = store.start_queue_task(root, &task.id, &reservation).unwrap();
    assert_eq!(starting.queue_state, Some(QueueState::Starting));
    assert!(starting.queue_reservation_expires_at.is_none());
    assert!(matches!(store.reserve_queue_task(root, &task.id, &queue_request,
        &uuid::Uuid::new_v4().to_string(), 180), Err(TaskStoreError::Conflict(_))));
    assert!(matches!(store.claim(root, &task.id, "manual-agent"),
        Err(TaskStoreError::Conflict(_))));

    let working = store.claim_queue_working(root, &task.id, &queue_request, &reservation, &session).unwrap();
    assert_eq!(working.queue_state, Some(QueueState::Working));
    assert_eq!(working.queue_session_id.as_deref(), Some(session.as_str()));
    let claimed = store.load(root).unwrap().tasks.into_iter().find(|row| row.id == task.id).unwrap();
    assert_eq!(claimed.claimed_by.as_deref(), Some(format!("t3:{session}").as_str()));
    assert_eq!(claimed.column, "in-progress");
    assert_eq!(store.claim_queue_working(root, &task.id, &queue_request, &reservation, &session).unwrap().queue_state,
        Some(QueueState::Working));
    assert!(store.claim_queue_working(root, &task.id, &queue_request, &reservation,
        &uuid::Uuid::new_v4().to_string()).is_err());

    // A normal claim cannot steal a reserved/starting item; after a legitimate
    // worker release the durable queue intent becomes selectable again.
    store.release_claim(root, &task.id, &format!("t3:{session}")).unwrap();
    let released = store.execution_intent(root, &task.id).unwrap();
    assert_eq!(released.queue_state, Some(QueueState::Queued));
    assert!(released.queue_reservation_id.is_none());
    assert!(store.reserve_queue_task(root, &task.id, &queue_request,
        &uuid::Uuid::new_v4().to_string(), 180).is_ok());
}

#[test]
fn expired_reserved_lease_can_be_replaced_but_starting_never_expires() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let (store, task, queue_request) = queued_task(root);
    let first = uuid::Uuid::new_v4().to_string();
    let reserved = store.reserve_queue_task(root, &task.id, &queue_request, &first, 30).unwrap();
    let path = std::path::Path::new(&task.path);
    let text = std::fs::read_to_string(path).unwrap();
    let expired = text.replace(
        reserved.queue_reservation_expires_at.as_deref().unwrap(),
        "2000-01-01T00:00:00.000Z",
    );
    std::fs::write(path, expired).unwrap();

    let second = uuid::Uuid::new_v4().to_string();
    let replaced = store.reserve_queue_task(root, &task.id, &queue_request, &second, 180).unwrap();
    assert_eq!(replaced.queue_reservation_id.as_deref(), Some(second.as_str()));
    store.start_queue_task(root, &task.id, &second).unwrap();
    assert!(matches!(store.reserve_queue_task(root, &task.id, &queue_request,
        &uuid::Uuid::new_v4().to_string(), 180), Err(TaskStoreError::Conflict(_))));
}

#[test]
fn definitive_failed_start_requeues_with_a_fresh_reservation_identity() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let (store, task, queue_request) = queued_task(root);
    let first = uuid::Uuid::new_v4().to_string();
    store.reserve_queue_task(root, &task.id, &queue_request, &first, 180).unwrap();
    store.start_queue_task(root, &task.id, &first).unwrap();
    let failed = store.requeue_failed_start(root, &task.id, &queue_request, &first, "T3 rejected before creation").unwrap();
    assert_eq!(failed.queue_state, Some(QueueState::Queued));
    assert!(failed.queue_reservation_id.is_none());
    assert_eq!(failed.queue_error.as_deref(), Some("T3 rejected before creation"));

    let second = uuid::Uuid::new_v4().to_string();
    let retried = store.reserve_queue_task(root, &task.id, &queue_request, &second, 180).unwrap();
    assert_eq!(retried.queue_reservation_id.as_deref(), Some(second.as_str()));
}

#[test]
fn legacy_openrig_attempt_cannot_be_reserved_by_t3() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let (store, task, queue_request) = queued_task(root);
    let path = std::path::Path::new(&task.path);
    let text = std::fs::read_to_string(path).unwrap();
    let updated = text.replacen(
        &format!("queue_request: {queue_request}"),
        &format!("queue_request: {queue_request}\nqueue_registration_attempt: {queue_request}"),
        1,
    );
    assert_ne!(updated, text);
    std::fs::write(path, updated).unwrap();

    assert!(matches!(store.reserve_queue_task(root, &task.id, &queue_request,
        &uuid::Uuid::new_v4().to_string(), 180), Err(TaskStoreError::Conflict(_))));
}

#[test]
fn a_reconciled_legacy_openrig_item_can_be_reserved_by_t3() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let (store, task, queue_request) = queued_task(root);
    let path = std::path::Path::new(&task.path);
    let text = std::fs::read_to_string(path).unwrap();
    let qitem = format!("qitem-taskboard-{queue_request}");
    let updated = text.replacen(
        &format!("queue_request: {queue_request}"),
        &format!("queue_request: {queue_request}\nqueue_registration_attempt: {queue_request}\nqueue_receipt: {qitem}\nqueue_retired_receipt: {qitem}"),
        1,
    );
    assert_ne!(updated, text);
    std::fs::write(path, updated).unwrap();

    let reservation = uuid::Uuid::new_v4().to_string();
    let result = store.reserve_queue_task(root, &task.id, &queue_request, &reservation, 180).unwrap();
    assert_eq!(result.queue_state, Some(QueueState::Reserved));
}

#[test]
fn manual_start_preempts_only_a_not_yet_started_queue_reservation() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let (store, task, queue_request) = queued_task(root);
    let reserved_id = uuid::Uuid::new_v4().to_string();
    store.reserve_queue_task(root, &task.id, &queue_request, &reserved_id, 180).unwrap();
    store.prepare_manual_start(root, &task.id).unwrap();
    let intent = store.execution_intent(root, &task.id).unwrap();
    assert_eq!(intent.mode, ExecutionMode::Backlog);
    assert!(intent.queue_state.is_none());
    assert!(intent.queue_reservation_id.is_none());
    assert!(store.start_queue_task(root, &task.id, &reserved_id).is_err());

    let (store, task, queue_request) = queued_task(root);
    let starting_id = uuid::Uuid::new_v4().to_string();
    store.reserve_queue_task(root, &task.id, &queue_request, &starting_id, 180).unwrap();
    store.start_queue_task(root, &task.id, &starting_id).unwrap();
    assert!(matches!(store.prepare_manual_start(root, &task.id), Err(TaskStoreError::Conflict(_))));
}

#[test]
fn working_queue_claim_is_requeued_when_managed_worker_releases_it() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let (store, task, queue_request) = queued_task(root);
    let reservation = uuid::Uuid::new_v4().to_string();
    let session = uuid::Uuid::new_v4().to_string();
    store.reserve_queue_task(root, &task.id, &queue_request, &reservation, 180).unwrap();
    store.start_queue_task(root, &task.id, &reservation).unwrap();
    let working = store.claim_queue_working(root, &task.id, &queue_request, &reservation, &session).unwrap();
    assert_eq!(working.queue_state, Some(QueueState::Working));
    assert!(store.release_claim(root, &task.id, &format!("t3:{session}")).is_ok());
    assert_eq!(store.execution_intent(root, &task.id).unwrap().queue_state, Some(QueueState::Queued));
}

#[test]
fn managed_queue_worker_release_and_completion_update_the_lifecycle() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let (store, task, _) = queued_task(root);
    let first = store.claim_managed(root, &task.id, "t3:manual-session").unwrap();
    store.release_managed(root, &task.id, "t3:manual-session", &first.claim_token).unwrap();
    assert_eq!(store.execution_intent(root, &task.id).unwrap().queue_state, Some(QueueState::Queued));

    let second = store.claim_managed(root, &task.id, "t3:manual-session-2").unwrap();
    store.complete_managed(root, &task.id, "t3:manual-session-2", &second.claim_token, &second.task.revision).unwrap();
    assert_eq!(store.execution_intent(root, &task.id).unwrap().queue_state, Some(QueueState::Completed));
}

#[test]
fn queued_task_completion_is_persisted_in_lifecycle() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let (store, task, _) = queued_task(root);
    let mut done = store.load(root).unwrap().tasks.into_iter().find(|row| row.id == task.id).unwrap();
    done.column = "done".into();
    store.update(root, done).unwrap();
    let completed = store.execution_intent(root, &task.id).unwrap();
    assert_eq!(completed.queue_state, Some(QueueState::Completed));

    // Done -> AI Ready restores Queue with a fresh, non-replayed request.
    let mut reopened = store.load(root).unwrap().tasks.into_iter().find(|row| row.id == task.id).unwrap();
    reopened.column = "ai-ready".into();
    store.update_as(root, reopened, task_core::MutationActor::Human).unwrap();
    let first = store.execution_intent(root, &task.id).unwrap();
    assert_eq!(first.queue_state, Some(QueueState::Queued));
    assert_ne!(first.queue_request, completed.queue_request);
    assert!(first.queue_session_id.is_none());
    assert!(first.queue_reservation_id.is_none());
    assert!(store.load(root).unwrap().tasks.iter().find(|row| row.id == task.id).unwrap().completed.is_none());
    assert_eq!(store.execution_intent(root, &task.id).unwrap().queue_request, first.queue_request);

    // Done -> Draft also restores Queue intent, while readiness blocks dispatch.
    let mut done = store.load(root).unwrap().tasks.into_iter().find(|row| row.id == task.id).unwrap();
    done.column = "done".into();
    store.update(root, done).unwrap();
    let mut draft = store.load(root).unwrap().tasks.into_iter().find(|row| row.id == task.id).unwrap();
    draft.column = "draft".into();
    store.update_as(root, draft, task_core::MutationActor::Human).unwrap();
    let second = store.execution_intent(root, &task.id).unwrap();
    assert_eq!(second.queue_state, Some(QueueState::Queued));
    assert_ne!(second.queue_request, first.queue_request);

    // Out-of-band Markdown edits also repair a previously Completed Queue.
    let mut done = store.load(root).unwrap().tasks.into_iter().find(|row| row.id == task.id).unwrap();
    done.column = "done".into();
    store.update(root, done).unwrap();
    let before = store.execution_intent(root, &task.id).unwrap();
    let path = std::path::Path::new(&task.path);
    let raw = std::fs::read_to_string(path).unwrap();
    let moved = raw.replacen("column: done", "column: ai-ready", 1);
    assert_ne!(raw, moved);
    std::fs::write(path, moved).unwrap();
    let after = store.load(root).unwrap();
    assert!(after.tasks.iter().find(|row| row.id == task.id).unwrap().completed.is_none());
    let repaired = store.execution_intent(root, &task.id).unwrap();
    assert_eq!(repaired.queue_state, Some(QueueState::Queued));
    assert_ne!(repaired.queue_request, before.queue_request);
    assert_eq!(store.execution_intent(root, &task.id).unwrap().queue_request, repaired.queue_request);
}

#[test]
fn recovery_watchdog_expires_reserved_and_escalates_unknown_start_without_replay() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let (store, task, queue_request) = queued_task(root);
    let request = uuid::Uuid::new_v4().to_string();
    let reserved = store.reserve_queue_task(root, &task.id, &queue_request, &request, 30).unwrap();
    assert!(reserved.queue_state_since.is_some());

    let file = std::path::Path::new(&task.path);
    let text = std::fs::read_to_string(file).unwrap();
    let expired = text.replace(
        reserved.queue_reservation_expires_at.as_deref().unwrap(),
        "2000-01-01T00:00:00.000Z",
    );
    std::fs::write(file, expired).unwrap();
    let restored = store.expire_reserved_queue_task(root, &task.id, &queue_request, &request).unwrap();
    assert_eq!(restored.queue_state, Some(QueueState::Queued));
    assert!(restored.queue_reservation_id.is_none());

    let fresh = uuid::Uuid::new_v4().to_string();
    store.reserve_queue_task(root, &task.id, &queue_request, &fresh, 180).unwrap();
    let started = store.start_queue_task(root, &task.id, &fresh).unwrap();
    assert_eq!(started.queue_state, Some(QueueState::Starting));
    assert!(started.queue_state_since.is_some());
    let incident = store.flag_queue_recovery_required(root, &task.id, &queue_request,
        QueueState::Starting, &fresh, "Unknown prompt delivery after maximum watchdog deadline").unwrap();
    assert_eq!(incident.queue_state, Some(QueueState::RecoveryRequired));
    assert!(incident.queue_error.as_deref().unwrap().contains("Unknown prompt"));
    assert_eq!(incident.queue_reservation_id.as_deref(), Some(fresh.as_str()));
    let task = store.load(root).unwrap().tasks.into_iter().find(|t| t.id == task.id).unwrap();
    assert!(task.attention.contains(&task_core::TaskAttention::NeedsUser));
    // A recovery incident is never silently eligible for a second worker.
    assert!(store.reserve_queue_task(root, &task.id, &queue_request,
        &uuid::Uuid::new_v4().to_string(), 180).is_err());
    assert!(store.flag_queue_recovery_required(root, &task.id, &queue_request,
        QueueState::Starting, &fresh, "another incident").is_err());

    // A deliberate operator override is explicit and rotates request identity;
    // an old T3 response can never claim the new attempt.
    let retried = store.retry_queue_incident(root, &task.id, &queue_request,
        &fresh, "inspected previous guest").unwrap();
    assert_eq!(retried.queue_state, Some(QueueState::Queued));
    assert_ne!(retried.queue_request.as_deref(), Some(queue_request.as_str()));
    assert!(retried.queue_reservation_id.is_none());
    let after = store.load(root).unwrap().tasks.into_iter().find(|t| t.id == task.id).unwrap();
    assert!(!after.attention.contains(&task_core::TaskAttention::NeedsUser));
    assert!(store.reserve_queue_task(root, &task.id, &queue_request,
        &uuid::Uuid::new_v4().to_string(), 180).is_err());
}

#[test]
fn human_inbox_recovery_decisions_are_actionable_and_cannot_silently_requeue() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let (store, task, queue_request) = queued_task(root);
    let request = uuid::Uuid::new_v4().to_string();
    store.reserve_queue_task(root, &task.id, &queue_request, &request, 180).unwrap();
    store.start_queue_task(root, &task.id, &request).unwrap();
    store.flag_queue_recovery_required(root, &task.id, &queue_request,
        QueueState::Starting, &request, "Unknown first prompt").unwrap();
    let mut decision = store.load(root).unwrap().tasks.into_iter()
        .find(|t| t.id == task.id).unwrap();
    assert_eq!(decision.question.as_ref().unwrap().request_type.as_deref(), Some("queue_recovery"));
    decision.question.as_mut().unwrap().answer = Some("Keep paused".into());
    decision.column = "ai-ready".into();
    decision.attention.clear(); // Inbox can clear flags; backend must restore incident.
    store.update_as(root, decision, task_core::MutationActor::Human).unwrap();
    let paused = store.load(root).unwrap().tasks.into_iter()
        .find(|t| t.id == task.id).unwrap();
    assert_eq!(paused.column, "blocked");
    assert!(paused.attention.contains(&task_core::TaskAttention::NeedsUser));
    assert_eq!(store.execution_intent(root, &task.id).unwrap().queue_state,
        Some(QueueState::RecoveryRequired));

    // Resolve the parked incident explicitly through the owning CLI method.
    let retry = store.retry_queue_incident(root, &task.id, &queue_request,
        &request, "operator approved").unwrap();
    assert_eq!(retry.queue_state, Some(QueueState::Queued));
    assert_ne!(retry.queue_request.as_deref(), Some(queue_request.as_str()));

    // The original Inbox retry route is also a one-shot, exact human action.
    let new_request = retry.queue_request.unwrap();
    let reservation = uuid::Uuid::new_v4().to_string();
    let mut ready = store.load(root).unwrap().tasks.into_iter()
        .find(|t| t.id == task.id).unwrap();
    ready.column = "ai-ready".into();
    store.update_as(root, ready, task_core::MutationActor::Human).unwrap();
    store.reserve_queue_task(root, &task.id, &new_request, &reservation, 180).unwrap();
    store.start_queue_task(root, &task.id, &reservation).unwrap();
    store.flag_queue_recovery_required(root, &task.id, &new_request,
        QueueState::Starting, &reservation, "Another unknown").unwrap();
    let mut response = store.load(root).unwrap().tasks.into_iter()
        .find(|t| t.id == task.id).unwrap();
    response.question.as_mut().unwrap().answer =
        Some("Retry anyway (may duplicate work)".into());
    response.column = "ai-ready".into();
    response.attention.clear();
    store.update_as(root, response, task_core::MutationActor::Human).unwrap();
    let recovered = store.execution_intent(root, &task.id).unwrap();
    assert_eq!(recovered.queue_state, Some(QueueState::Queued));
    assert_ne!(recovered.queue_request.as_deref(), Some(new_request.as_str()));
    assert!(recovered.queue_reservation_id.is_none());
    let card = store.load(root).unwrap().tasks.into_iter()
        .find(|t| t.id == task.id).unwrap();
    assert!(!card.attention.contains(&task_core::TaskAttention::NeedsUser));
    assert!(!card.attention.contains(&task_core::TaskAttention::Question));
}
