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
    assert!(queued.queue_request.is_some());
    assert!(queued.queue_receipt.is_none());
    assert!(queued.queue_attempt.is_none());
    assert!(queued.start_now_request.is_none());
    assert!(std::fs::read_to_string(&task.path).unwrap().contains("custom_extension: original"));
    let loaded = store.load(root).unwrap().tasks.into_iter().find(|t| t.id == task.id).unwrap();
    store.update(root, loaded).unwrap();
    assert_eq!(store.execution_intent(root, &task.id).unwrap().queue_request, queued.queue_request);
    assert_eq!(store.set_execution_mode(root, &task.id, &store.execution_intent(root, &task.id).unwrap().revision, ExecutionMode::Queue).unwrap().queue_request, queued.queue_request);
    assert!(matches!(store.set_execution_mode(root, &task.id, &current.revision, ExecutionMode::Backlog), Err(TaskStoreError::Conflict(_))));
}

#[test]
fn queue_request_is_immutable_until_requeued_and_receipt_is_compare_and_set() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let store = FsTaskStore;
    store.ensure_board(root, "Registration").unwrap();
    let task = store.create(root, serde_json::from_value(serde_json::json!({
        "title": "Queue", "column": "ai-ready", "parent": null
    })).unwrap()).unwrap();
    let first = store.set_execution_mode(root, &task.id, &task.revision, ExecutionMode::Queue).unwrap();
    let request = first.queue_request.as_ref().unwrap();
    let qitem = format!("qitem-taskboard-{request}");
    assert!(matches!(store.receipt_queue(root, &task.id, request, &task.revision, &qitem), Err(TaskStoreError::Conflict(_))));
    let attempted = store.begin_queue_registration(root, &task.id, request, &first.revision).unwrap();
    assert_eq!(attempted.queue_attempt.as_deref(), Some(request.as_str()));
    assert!(matches!(store.begin_queue_registration(root, &task.id, request, &attempted.revision), Err(TaskStoreError::Conflict(_))));
    assert!(matches!(store.receipt_queue(root, &task.id, request, &first.revision, &qitem), Err(TaskStoreError::Conflict(_))));
    let receipt = store.receipt_queue(root, &task.id, request, &attempted.revision, &qitem).unwrap();
    assert_eq!(receipt.queue_receipt.as_deref(), Some(qitem.as_str()));
    assert_eq!(store.receipt_queue(root, &task.id, request, &first.revision, &qitem).unwrap().revision, receipt.revision);
    assert!(matches!(store.receipt_queue(root, &task.id, request, &receipt.revision, "qitem-2"), Err(TaskStoreError::Conflict(_))));
    let backlog = store.set_execution_mode(root, &task.id, &receipt.revision, ExecutionMode::Backlog).unwrap();
    assert_eq!(backlog.queue_receipt.as_deref(), Some(qitem.as_str()));
    assert!(matches!(store.receipt_queue(root, &task.id, request, &backlog.revision, &qitem), Err(TaskStoreError::Conflict(_))));
    assert!(matches!(
        store.set_execution_mode(root, &task.id, &backlog.revision, ExecutionMode::Queue),
        Err(TaskStoreError::Conflict(_))
    ));
    let retired = store.retire_queue(root, &task.id, request, &backlog.revision, &qitem).unwrap();
    assert_eq!(retired.queue_retired_receipt.as_deref(), Some(qitem.as_str()));
    let requeued = store.set_execution_mode(root, &task.id, &retired.revision, ExecutionMode::Queue).unwrap();
    assert_ne!(requeued.queue_request, first.queue_request);
    assert!(requeued.queue_receipt.is_none());
    assert!(requeued.queue_attempt.is_none());
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

#[test]
fn column_auto_queue_applies_to_future_ready_tasks_and_keeps_manual_backlog() {
    use task_core::queue_task_ready;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let store = FsTaskStore;
    store.ensure_board(root, "Column Queue").unwrap();
    let mut board = store.load(root).unwrap().config;
    board.columns.iter_mut().find(|c| c.id == "ai-ready").unwrap().auto_queue = true;
    store.save_config(root, board).unwrap();
    let created = store.create(root, serde_json::from_value(serde_json::json!({
        "title":"New ready", "column":"ai-ready", "parent":null
    })).unwrap()).unwrap();
    let queued = store.execution_intent(root, &created.id).unwrap();
    assert_eq!(queued.mode, ExecutionMode::Queue);
    let same = store.load(root).unwrap();
    assert!(queue_task_ready(&same, same.tasks.iter().find(|t| t.id == created.id).unwrap()));
    assert_eq!(store.execution_intent(root, &created.id).unwrap().queue_request, queued.queue_request);
    let manual = store.set_execution_mode(root, &created.id, &queued.revision, ExecutionMode::Backlog).unwrap();
    assert_eq!(manual.mode, ExecutionMode::Backlog);
    assert_eq!(store.load(root).unwrap().tasks.len(), 1);
    assert_eq!(store.execution_intent(root, &created.id).unwrap().mode, ExecutionMode::Backlog);
}

#[test]
fn moving_a_task_adopts_column_queue_policy_for_human_and_agent_edits() {
    use task_core::MutationActor;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let store = FsTaskStore;
    store.ensure_board(root, "Move Queue").unwrap();
    let mut config = store.load(root).unwrap().config;
    let mut auto = config.columns.iter().find(|c| c.id == "ai-ready").unwrap().clone();
    auto.id = "auto-queued".into();
    auto.name = "Auto queued".into();
    auto.auto_queue = true;
    auto.order += 1;
    config.columns.push(auto);
    store.save_config(root, config).unwrap();

    let created = store.create_as(root, serde_json::from_value(serde_json::json!({
        "title":"New queued", "column":"auto-queued", "parent":null
    })).unwrap(), MutationActor::Human).unwrap();
    assert_eq!(store.execution_intent(root, &created.id).unwrap().mode, ExecutionMode::Queue);
    let mut task = created;
    task.column = "ai-ready".into();
    let moved = store.update_as(root, task, MutationActor::Agent).unwrap();
    assert_eq!(store.execution_intent(root, &moved.id).unwrap().mode, ExecutionMode::Backlog);

    // The destination policy overrides even a prior explicit per-card preference.
    let manually_queued = store.set_execution_mode(root, &moved.id, &moved.revision, ExecutionMode::Queue).unwrap();
    let mut task = store.load(root).unwrap().tasks.into_iter().find(|t| t.id == moved.id).unwrap();
    task.column = "auto-queued".into();
    let moved = store.update_as(root, task, MutationActor::Human).unwrap();
    assert_eq!(store.execution_intent(root, &moved.id).unwrap().mode, ExecutionMode::Queue);
    assert_eq!(store.execution_intent(root, &moved.id).unwrap().queue_request, manually_queued.queue_request);

    let manual = store.set_execution_mode(root, &moved.id, &moved.revision, ExecutionMode::Backlog).unwrap();
    let mut task = store.load(root).unwrap().tasks.into_iter().find(|t| t.id == moved.id).unwrap();
    task.column = "ai-ready".into();
    let moved = store.update_as(root, task, MutationActor::Agent).unwrap();
    assert_eq!(store.execution_intent(root, &moved.id).unwrap().mode, ExecutionMode::Backlog);
    let mut task = moved;
    task.column = "auto-queued".into();
    let moved = store.update_as(root, task, MutationActor::Human).unwrap();
    let next = store.execution_intent(root, &moved.id).unwrap();
    assert_eq!(next.mode, ExecutionMode::Queue);
    assert_ne!(next.queue_request, manual.queue_request);
}

#[test]
fn newly_marked_priority_auto_queues_but_creation_follows_column_default() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let store = FsTaskStore;
    store.ensure_board(root, "Priority Queue").unwrap();
    let old = store.create(root, serde_json::from_value(serde_json::json!({
        "title":"Already priority", "column":"ai-ready", "parent":null,
        "attention":["priority"]
    })).unwrap()).unwrap();
    assert_eq!(store.execution_intent(root, &old.id).unwrap().mode, ExecutionMode::Backlog);
    let original = store.create(root, serde_json::from_value(serde_json::json!({
        "title":"New priority", "column":"ai-ready", "parent":null
    })).unwrap()).unwrap();
    let mut task = original;
    task.attention.push(task_core::TaskAttention::Priority);
    let updated = store.update(root, task).unwrap();
    assert_eq!(store.execution_intent(root, &updated.id).unwrap().mode, ExecutionMode::Queue);
    assert_eq!(store.execution_intent(root, &old.id).unwrap().mode, ExecutionMode::Backlog);
}

#[test]
fn waiting_dependencies_do_not_dispatch_until_prerequisite_completes() {
    use task_core::queue_task_ready;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let store = FsTaskStore;
    store.ensure_board(root, "Waiting").unwrap();
    let prerequisite = store.create(root, serde_json::from_value(serde_json::json!({
        "title":"Requirement", "column":"ai-ready", "parent":null
    })).unwrap()).unwrap();
    let mut config = store.load(root).unwrap().config;
    config.columns.iter_mut().find(|c| c.id == "ai-ready").unwrap().auto_queue = true;
    store.save_config(root, config).unwrap();
    let dependent = store.create(root, serde_json::from_value(serde_json::json!({
        "title":"Dependent", "column":"ai-ready", "parent":null, "depends_on":[prerequisite.id]
    })).unwrap()).unwrap();
    // Queue intent follows the column immediately; dispatch still waits for dependencies.
    assert_eq!(store.execution_intent(root, &dependent.id).unwrap().mode, ExecutionMode::Queue);
    let board = store.load(root).unwrap();
    assert!(!queue_task_ready(&board, board.tasks.iter().find(|t| t.id==dependent.id).unwrap()));
    let mut required = board.tasks.iter().find(|t| t.id == prerequisite.id).unwrap().clone();
    required.column = "done".into();
    store.update(root, required).unwrap();
    store.load(root).unwrap();
    assert_eq!(store.execution_intent(root, &dependent.id).unwrap().mode, ExecutionMode::Queue);
}

#[test]
fn foreground_request_rejects_registered_queue_until_terminal_reconciled() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let store = FsTaskStore;
    store.ensure_board(root, "No duplicate foreground").unwrap();
    let task = store.create(root, serde_json::from_value(serde_json::json!({
        "title":"Foreground", "column":"ai-ready", "parent":null
    })).unwrap()).unwrap();
    let queued = store.set_execution_mode(root, &task.id, &task.revision, ExecutionMode::Queue).unwrap();
    let request = queued.queue_request.as_deref().unwrap();
    let in_flight = store.begin_queue_registration(root, &task.id, request, &queued.revision).unwrap();
    assert!(matches!(store.request_start_now(root, &task.id, &in_flight.revision), Err(TaskStoreError::Conflict(_))));
    assert_eq!(store.execution_intent(root, &task.id).unwrap().start_now_request,None);
}

#[test]
fn priority_set_in_draft_adopts_destination_column_instead_of_queuing() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let store = FsTaskStore;
    store.ensure_board(root, "Priority draft").unwrap();
    let mut task = store.create(root, serde_json::from_value(serde_json::json!({
        "title":"Planned priority", "column":"draft", "parent":null, "attention":["priority"]
    })).unwrap()).unwrap();
    assert_eq!(store.execution_intent(root, &task.id).unwrap().mode, ExecutionMode::Backlog);
    task.column = "ai-ready".into();
    let updated = store.update(root, task).unwrap();
    assert_eq!(updated.column, "ai-ready");
    assert_eq!(store.execution_intent(root, &updated.id).unwrap().mode, ExecutionMode::Backlog);
}

#[test]
fn archive_revokes_queue_intent_but_retains_exact_receipt_for_cancellation() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let store = FsTaskStore;
    store.ensure_board(root, "Archive queue").unwrap();
    let task = store.create(root, serde_json::from_value(serde_json::json!({
        "title":"Archive me", "column":"ai-ready", "parent":null
    })).unwrap()).unwrap();
    let queued = store.set_execution_mode(root,&task.id,&task.revision,ExecutionMode::Queue).unwrap();
    let archived = store.archive(root,&task.id).unwrap();
    assert!(archived.archived);
    assert!(archived.claimed_by.is_none());
    let intent = store.execution_intent(root,&task.id).unwrap();
    assert_eq!(intent.mode, ExecutionMode::Backlog);
    assert_eq!(intent.queue_request, queued.queue_request);
}
