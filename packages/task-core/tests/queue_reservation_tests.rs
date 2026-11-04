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
    assert_eq!(store.execution_intent(root, &task.id).unwrap().queue_state, Some(QueueState::Completed));
}
