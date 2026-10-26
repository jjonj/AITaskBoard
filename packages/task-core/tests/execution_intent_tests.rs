use task_core::{ExecutionMode, FsTaskStore, TaskDraft, TaskStore, TaskStoreError};

#[test]
fn legacy_is_backlog_and_queue_does_not_enqueue_or_erase_unknown_frontmatter() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let store = FsTaskStore;
    store.ensure_board(root, "Queue test").unwrap();
    let draft: TaskDraft = serde_json::from_value(serde_json::json!({
        "title": "Legacy", "column": "ai-ready", "parent": null, "description": "Body"
    })).unwrap();
    let task = store.create(root, draft).unwrap();
    let initial = store.execution_intent(root, &task.id).unwrap();
    assert_eq!(initial.mode, ExecutionMode::Backlog);
    assert!(initial.start_now_request.is_none());
    let original = std::fs::read_to_string(&task.path).unwrap();
    let annotated = original.replacen("title: Legacy", "custom_extension: original\ntitle: Legacy", 1);
    std::fs::write(&task.path, annotated).unwrap();
    let current = store.execution_intent(root, &task.id).unwrap();
    let queued = store.set_execution_mode(root, &task.id, &current.revision, ExecutionMode::Queue).unwrap();
    assert_eq!(queued.mode, ExecutionMode::Queue);
    assert!(queued.start_now_request.is_none());
    assert!(std::fs::read_to_string(&task.path).unwrap().contains("custom_extension: original"));
    let loaded = store.load(root).unwrap().tasks.into_iter().find(|t| t.id == task.id).unwrap();
    store.update(root, loaded).unwrap();
    assert_eq!(store.execution_intent(root, &task.id).unwrap().mode, ExecutionMode::Queue);
    assert!(matches!(store.set_execution_mode(root, &task.id, &current.revision, ExecutionMode::Backlog), Err(TaskStoreError::Conflict(_))));
}

#[test]
fn start_now_request_is_one_shot_and_unknown_receipt_blocks_replay() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let store = FsTaskStore;
    store.ensure_board(root, "One shot").unwrap();
    let draft: TaskDraft = serde_json::from_value(serde_json::json!({
        "title": "Foreground", "column": "ai-ready", "parent": null
    })).unwrap();
    let task = store.create(root, draft).unwrap();
    let issued = store.request_start_now(root, &task.id, &task.revision).unwrap();
    let id = issued.start_now_request.as_deref().unwrap();
    assert!(matches!(store.request_start_now(root, &task.id, &issued.revision), Err(TaskStoreError::Conflict(_))));
    let acknowledged = store.receipt_start_now(root, &task.id, id, "provider-receipt").unwrap();
    assert_eq!(acknowledged.start_now_receipt.as_deref(), Some("provider-receipt"));
    assert_eq!(store.receipt_start_now(root, &task.id, id, "provider-receipt").unwrap().revision, acknowledged.revision);
    assert!(matches!(store.receipt_start_now(root, &task.id, id, "another"), Err(TaskStoreError::Conflict(_))));
    let next = store.request_start_now(root, &task.id, &acknowledged.revision).unwrap();
    assert_ne!(next.start_now_request, issued.start_now_request);
    assert!(next.start_now_receipt.is_none());
}
