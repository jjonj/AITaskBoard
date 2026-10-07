use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};
use task_core::{FsTaskStore, TaskDraft, TaskStore, TaskStoreError};

fn temp_board(name: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("taskboard-{name}-{}-{stamp}", std::process::id()))
}

fn draft(title: &str, column: &str, parent: Option<&str>) -> TaskDraft {
    TaskDraft {
        title: title.into(),
        column: column.into(),
        parent: parent.map(str::to_string),
        category: None,
        attention: Vec::new(),
        question: None,
        description: format!("Description for {title}"),
        acceptance_criteria: vec!["Works".into()],
        ai_notes: String::new(),
        user_notes: String::new(),
        notes: String::new(),
    }
}

#[test]
fn creates_loads_updates_and_archives_tasks() {
    let root = temp_board("roundtrip");
    let store = FsTaskStore::default();
    store.ensure_board(&root, "Test").unwrap();

    let created = store
        .create(&root, draft("Parent", "ai-ready", None))
        .unwrap();
    let child = store
        .create(&root, draft("Child", "draft", Some(&created.id)))
        .unwrap();

    let snapshot = store.load(&root).unwrap();
    assert_eq!(snapshot.tasks.len(), 2);
    assert_eq!(child.parent.as_deref(), Some(created.id.as_str()));

    let mut updated = child.clone();
    updated.column = "in-progress".into();
    let updated = store.update(&root, updated).unwrap();
    assert_eq!(updated.column, "in-progress");

    let archived = store.archive(&root, &updated.id).unwrap();
    assert!(archived.archived);
    assert!(archived.path.contains("Archive"));
    assert!(!root.join(".taskboard-write.lock").exists());

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn stale_task_update_is_rejected() {
    let root = temp_board("conflict");
    let store = FsTaskStore::default();
    store.ensure_board(&root, "Test").unwrap();

    let original = store.create(&root, draft("One", "ai-ready", None)).unwrap();
    let mut first = original.clone();
    first.title = "First writer".into();
    store.update(&root, first).unwrap();

    let mut stale = original;
    stale.title = "Stale writer".into();
    let error = store.update(&root, stale).unwrap_err();
    assert!(matches!(error, TaskStoreError::Conflict(_)));

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejects_parent_cycles() {
    let root = temp_board("cycle");
    let store = FsTaskStore::default();
    store.ensure_board(&root, "Test").unwrap();

    let parent = store
        .create(&root, draft("Parent", "ai-ready", None))
        .unwrap();
    let child = store
        .create(&root, draft("Child", "ai-ready", Some(&parent.id)))
        .unwrap();

    let mut parent_update = parent;
    parent_update.parent = Some(child.id);
    let error = store.update(&root, parent_update).unwrap_err();
    assert!(matches!(error, TaskStoreError::Invalid(_)));

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn reads_manual_crlf_edits() {
    let root = temp_board("crlf");
    let store = FsTaskStore::default();
    store.ensure_board(&root, "Test").unwrap();
    let created = store
        .create(&root, draft("CRLF", "ai-ready", None))
        .unwrap();

    let path = PathBuf::from(&created.path);
    let text = fs::read_to_string(&path).unwrap().replace("\n", "\r\n");
    fs::write(&path, text).unwrap();

    let snapshot = store.load(&root).unwrap();
    assert_eq!(snapshot.tasks.len(), 1);
    assert_eq!(snapshot.tasks[0].title, "CRLF");

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn saves_dynamic_columns() {
    let root = temp_board("columns");
    let store = FsTaskStore::default();
    store.ensure_board(&root, "Test").unwrap();

    let mut config = store.load(&root).unwrap().config;
    let next_order = config.columns.len() as i32;
    config.columns.push(task_core::Column {
        id: "review".into(),
        name: "Review".into(),
        order: next_order,
        terminal: false,
        hidden: false,
        ai_ready: false,
        color: "#777777".into(),
    });
    let config = store.save_config(&root, config).unwrap();
    assert!(config.columns.iter().any(|column| column.id == "review"));

    let task = store
        .create(&root, draft("Review me", "review", None))
        .unwrap();
    assert_eq!(task.column, "review");

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn preserves_unknown_frontmatter_fields_on_update() {
    let root = temp_board("unknown-fields");
    let store = FsTaskStore::default();
    store.ensure_board(&root, "Test").unwrap();

    let created = store
        .create(&root, draft("Custom", "ai-ready", None))
        .unwrap();
    let path = PathBuf::from(&created.path);
    let text = fs::read_to_string(&path).unwrap();
    let text = text.replacen("created:", "custom_agent_hint: keep-me\ncreated:", 1)
        + "\n## Agent Scratch\n\nDo not delete this custom section.\n";
    fs::write(&path, text).unwrap();

    let mut loaded = store
        .load(&root)
        .unwrap()
        .tasks
        .into_iter()
        .find(|task| task.id == created.id)
        .unwrap();
    loaded.title = "Edited by desktop".into();
    store.update(&root, loaded).unwrap();

    let saved = fs::read_to_string(path).unwrap();
    assert!(saved.contains("custom_agent_hint: keep-me"));
    assert!(saved.contains("## Agent Scratch"));
    assert!(saved.contains("Do not delete this custom section."));

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn claims_are_exclusive_and_release_is_owner_only() {
    let root = temp_board("claims");
    let store = FsTaskStore::default();
    store.ensure_board(&root, "Test").unwrap();

    let created = store
        .create(&root, draft("Claim me", "draft", None))
        .unwrap();

    let claimed = store.claim(&root, &created.id, "kiwi").unwrap();
    assert_eq!(claimed.claimed_by.as_deref(), Some("kiwi"));

    let same_agent = store.claim(&root, &created.id, "kiwi").unwrap();
    assert_eq!(same_agent.claimed_by.as_deref(), Some("kiwi"));

    let error = store.claim(&root, &created.id, "otter").unwrap_err();
    assert!(matches!(
        error,
        TaskStoreError::AlreadyClaimed { claimed_by, .. } if claimed_by == "kiwi"
    ));

    let error = store
        .release_claim(&root, &created.id, "otter")
        .unwrap_err();
    assert!(matches!(
        error,
        TaskStoreError::AlreadyClaimed { claimed_by, .. } if claimed_by == "kiwi"
    ));

    let released = store.release_claim(&root, &created.id, "kiwi").unwrap();
    assert_eq!(released.claimed_by, None);

    let released_again = store.release_claim(&root, &created.id, "kiwi").unwrap();
    assert_eq!(released_again.claimed_by, None);

    let saved = fs::read_to_string(released.path).unwrap();
    assert!(!saved.contains("claimed_by:"));

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn preserves_unknown_board_metadata_and_body() {
    let root = temp_board("board-roundtrip");
    let store = FsTaskStore::default();
    store.ensure_board(&root, "Test").unwrap();

    let path = root.join("_board.md");
    let text = fs::read_to_string(&path).unwrap();
    let text = text.replacen("name:", "custom_board_hint: keep-board-meta\nname:", 1)
        + "\n## Custom Board Notes\n\nKeep this text too.\n";
    fs::write(&path, text).unwrap();

    let mut config = store.load(&root).unwrap().config;
    config.name = "Renamed Test".into();
    store.save_config(&root, config).unwrap();

    let saved = fs::read_to_string(path).unwrap();
    assert!(saved.contains("custom_board_hint: keep-board-meta"));
    assert!(saved.contains("## Custom Board Notes"));
    assert!(saved.contains("Keep this text too."));

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn concurrent_creates_get_unique_ids() {
    let root = temp_board("concurrent-create");
    FsTaskStore::default().ensure_board(&root, "Test").unwrap();

    let root_a = root.clone();
    let root_b = root.clone();
    let a = std::thread::spawn(move || {
        FsTaskStore::default().create(&root_a, draft("A", "ai-ready", None))
    });
    let b = std::thread::spawn(move || {
        FsTaskStore::default().create(&root_b, draft("B", "ai-ready", None))
    });

    let a = a.join().unwrap().unwrap();
    let b = b.join().unwrap().unwrap();
    assert_ne!(a.id, b.id);

    let snapshot = FsTaskStore::default().load(&root).unwrap();
    assert_eq!(snapshot.tasks.len(), 2);

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn default_board_uses_ai_ready_and_draft_semantics() {
    let config = task_core::default_board_config("Test");
    assert!(config
        .columns
        .iter()
        .any(|column| column.id == task_core::AI_READY_COLUMN_ID));
    assert!(config
        .columns
        .iter()
        .any(|column| column.id == task_core::DRAFT_COLUMN_ID));
    assert!(config.columns.iter().all(|column| !column.color.is_empty()));
}

#[test]
fn new_task_supports_category_without_notes_section() {
    let root = temp_board("category");
    let store = FsTaskStore::default();
    store.ensure_board(&root, "Test").unwrap();

    let mut task = draft("Lab task", "ai-ready", None);
    task.category = Some("Feature Lab".into());
    let created = store.create(&root, task).unwrap();

    assert_eq!(created.category.as_deref(), Some("Feature Lab"));
    let saved = fs::read_to_string(&created.path).unwrap();
    assert!(
        saved.contains("category: Feature Lab")
            || saved.contains("category: 'Feature Lab'")
            || saved.contains("category: \"Feature Lab\"")
    );
    assert!(!saved.contains("## Notes"));

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn question_attention_and_ai_notes_round_trip_and_answer_cleanly() {
    let root = temp_board("question");
    let store = FsTaskStore::default();
    store.ensure_board(&root, "Test").unwrap();

    let mut d = draft("Need a decision", "ai-ready", None);
    d.ai_notes = "Two implementations are viable; user preference decides the UX.".into();
    d.attention = vec![task_core::TaskAttention::Question];
    d.question = Some(task_core::TaskQuestion {
        prompt: "Which interaction should we use?".into(),
        options: vec!["Radial menu".into(), "Context menu".into()],
        answer: None,
    });

    let created = store.create(&root, d).unwrap();
    let markdown = fs::read_to_string(&created.path).unwrap();
    assert!(markdown.contains("## AI Notes"));
    assert!(markdown.contains("Two implementations are viable"));
    assert!(markdown.contains("attention:"));
    assert!(markdown.contains("question:"));

    let loaded = store.load(&root).unwrap();
    let mut task = loaded
        .tasks
        .into_iter()
        .find(|task| task.id == created.id)
        .unwrap();
    assert_eq!(
        task.ai_notes,
        "Two implementations are viable; user preference decides the UX."
    );
    assert!(task.attention.contains(&task_core::TaskAttention::Question));

    task.attention
        .retain(|flag| flag != &task_core::TaskAttention::Question);
    task.question.as_mut().unwrap().answer = Some("Context menu".into());
    let saved = store.update(&root, task).unwrap();

    let reloaded = store.load(&root).unwrap();
    let answered = reloaded
        .tasks
        .into_iter()
        .find(|task| task.id == saved.id)
        .unwrap();
    assert!(!answered
        .attention
        .contains(&task_core::TaskAttention::Question));
    assert_eq!(
        answered.question.unwrap().answer.as_deref(),
        Some("Context menu")
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn unresolved_question_requires_question_attention() {
    let root = temp_board("question-validation");
    let store = FsTaskStore::default();
    store.ensure_board(&root, "Test").unwrap();

    let mut d = draft("Bad question", "ai-ready", None);
    d.question = Some(task_core::TaskQuestion {
        prompt: "Pick one?".into(),
        options: vec!["A".into(), "B".into()],
        answer: None,
    });

    let error = store.create(&root, d).unwrap_err();
    assert!(matches!(error, TaskStoreError::Invalid(_)));

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn user_notes_round_trip_and_user_note_attention_can_be_cleared_without_erasing_note() {
    let root = temp_board("user-note");
    let store = FsTaskStore::default();
    store.ensure_board(&root, "Test").unwrap();

    let mut d = draft("Clarify me", "ai-ready", None);
    d.user_notes = "I don't understand what this task means; make it clearer.".into();
    d.attention = vec![task_core::TaskAttention::UserNote];

    let created = store.create(&root, d).unwrap();
    let markdown = fs::read_to_string(&created.path).unwrap();
    assert!(markdown.contains("## User Notes"));
    assert!(markdown.contains("make it clearer"));
    assert!(markdown.contains("user-note"));

    let loaded = store.load(&root).unwrap();
    let mut task = loaded
        .tasks
        .into_iter()
        .find(|task| task.id == created.id)
        .unwrap();
    assert_eq!(
        task.user_notes,
        "I don't understand what this task means; make it clearer."
    );
    assert!(task.attention.contains(&task_core::TaskAttention::UserNote));

    task.attention
        .retain(|flag| flag != &task_core::TaskAttention::UserNote);
    let saved = store.update(&root, task).unwrap();

    let reloaded = store.load(&root).unwrap();
    let handled = reloaded
        .tasks
        .into_iter()
        .find(|task| task.id == saved.id)
        .unwrap();
    assert_eq!(
        handled.user_notes,
        "I don't understand what this task means; make it clearer."
    );
    assert!(!handled
        .attention
        .contains(&task_core::TaskAttention::UserNote));

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn revisions_are_serialized_for_frontend_conflict_detection() {
    let root = temp_board("serialized-revisions");
    let store = FsTaskStore::default();
    store.ensure_board(&root, "Test").unwrap();
    store.create(&root, draft("One", "ai-ready", None)).unwrap();

    let snapshot = store.load(&root).unwrap();
    assert!(!snapshot.config.revision.is_empty());
    assert!(!snapshot.tasks[0].revision.is_empty());

    let json = serde_json::to_value(&snapshot).unwrap();
    assert!(json["config"]["revision"]
        .as_str()
        .is_some_and(|value| !value.is_empty()));
    assert!(json["tasks"][0]["revision"]
        .as_str()
        .is_some_and(|value| !value.is_empty()));

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn snapshot_undo_restores_task_and_rejects_later_external_edits() {
    let root = temp_board("undo-task");
    let store = FsTaskStore::default();
    store.ensure_board(&root, "Test").unwrap();
    let created = store
        .create(&root, draft("Move me", "ai-ready", None))
        .unwrap();

    let before = store.load(&root).unwrap();
    let mut moved = before
        .tasks
        .iter()
        .find(|task| task.id == created.id)
        .unwrap()
        .clone();
    moved.column = "in-progress".into();
    store.update(&root, moved).unwrap();
    let after = store.load(&root).unwrap();

    let restored = store
        .restore_snapshot(&root, before.clone(), after)
        .unwrap();
    assert_eq!(
        restored
            .tasks
            .iter()
            .find(|task| task.id == created.id)
            .unwrap()
            .column,
        "ai-ready"
    );

    let before_again = store.load(&root).unwrap();
    let mut moved_again = before_again
        .tasks
        .iter()
        .find(|task| task.id == created.id)
        .unwrap()
        .clone();
    moved_again.column = "in-progress".into();
    store.update(&root, moved_again).unwrap();
    let after_again = store.load(&root).unwrap();

    let path = PathBuf::from(
        &after_again
            .tasks
            .iter()
            .find(|task| task.id == created.id)
            .unwrap()
            .path,
    );
    let text = fs::read_to_string(&path).unwrap();
    fs::write(
        &path,
        text.replace("title: Move me", "title: External edit"),
    )
    .unwrap();

    let error = store
        .restore_snapshot(&root, before_again, after_again)
        .unwrap_err();
    assert!(matches!(error, TaskStoreError::Conflict(_)));

    let current = store.load(&root).unwrap();
    assert_eq!(
        current
            .tasks
            .iter()
            .find(|task| task.id == created.id)
            .unwrap()
            .title,
        "External edit"
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn snapshot_undo_removes_created_task_and_restores_board_config() {
    let root = temp_board("undo-create-config");
    let store = FsTaskStore::default();
    store.ensure_board(&root, "Test").unwrap();

    let before_create = store.load(&root).unwrap();
    store
        .create(&root, draft("Temporary", "ai-ready", None))
        .unwrap();
    let after_create = store.load(&root).unwrap();
    let restored = store
        .restore_snapshot(&root, before_create, after_create)
        .unwrap();
    assert!(restored.tasks.is_empty());

    let before_config = store.load(&root).unwrap();
    let mut changed = before_config.config.clone();
    changed
        .columns
        .iter_mut()
        .find(|column| column.id == "in-progress")
        .unwrap()
        .ai_ready = true;
    store.save_config(&root, changed).unwrap();
    let after_config = store.load(&root).unwrap();
    assert!(
        after_config
            .config
            .columns
            .iter()
            .find(|column| column.id == "in-progress")
            .unwrap()
            .ai_ready
    );

    let restored = store
        .restore_snapshot(&root, before_config, after_config)
        .unwrap();
    assert!(
        !restored
            .config
            .columns
            .iter()
            .find(|column| column.id == "in-progress")
            .unwrap()
            .ai_ready
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn canonical_ai_ready_column_cannot_be_disabled() {
    let root = temp_board("canonical-ready");
    let store = FsTaskStore::default();
    store.ensure_board(&root, "Test").unwrap();

    let mut config = store.load(&root).unwrap().config;
    config
        .columns
        .iter_mut()
        .find(|column| column.id == task_core::AI_READY_COLUMN_ID)
        .unwrap()
        .ai_ready = false;

    let error = store.save_config(&root, config).unwrap_err();
    assert!(matches!(error, TaskStoreError::Invalid(_)));

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn terminal_tasks_get_completion_date_and_reopening_clears_it() {
    let root = temp_board("completion-state");
    let store = FsTaskStore::default();
    store.ensure_board(&root, "Test").unwrap();

    let created = store
        .create(&root, draft("Finish me", "ai-ready", None))
        .unwrap();
    assert_eq!(created.completed, None);

    let mut completed = created;
    completed.column = "done".into();
    let completed = store.update(&root, completed).unwrap();
    assert!(completed.completed.is_some());

    let mut reopened = completed;
    reopened.column = "in-progress".into();
    let reopened = store.update(&root, reopened).unwrap();
    assert_eq!(reopened.completed, None);

    let saved = fs::read_to_string(&reopened.path).unwrap();
    assert!(!saved.contains("completed:"));

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn legacy_terminal_task_gets_full_retention_period_on_first_load() {
    let root = temp_board("legacy-completed");
    let store = FsTaskStore::default();
    store.ensure_board(&root, "Test").unwrap();

    let created = store
        .create(&root, draft("Legacy done", "done", None))
        .unwrap();
    let path = PathBuf::from(&created.path);
    let text = fs::read_to_string(&path).unwrap();
    let without_completed = text
        .lines()
        .filter(|line| !line.starts_with("completed:"))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    fs::write(&path, without_completed).unwrap();

    let snapshot = store.load(&root).unwrap();
    let task = snapshot
        .tasks
        .iter()
        .find(|task| task.id == created.id)
        .unwrap();
    assert!(!task.archived);
    assert!(task.completed.is_some());
    assert!(PathBuf::from(&task.path).starts_with(root.join("Active")));

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn terminal_task_is_auto_archived_after_retention_period() {
    let root = temp_board("auto-archive");
    let store = FsTaskStore::default();
    store.ensure_board(&root, "Test").unwrap();

    let created = store
        .create(&root, draft("Old done", "done", None))
        .unwrap();
    let path = PathBuf::from(&created.path);
    let text = fs::read_to_string(&path).unwrap();
    let current_completed = created.completed.as_deref().unwrap();
    fs::write(
        &path,
        text.replace(
            &format!("completed: {current_completed}"),
            "completed: 2000-01-01",
        ),
    )
    .unwrap();

    let snapshot = store.load(&root).unwrap();
    let task = snapshot
        .tasks
        .iter()
        .find(|task| task.id == created.id)
        .unwrap();
    assert!(task.archived);
    assert!(PathBuf::from(&task.path).starts_with(root.join("Archive")));

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn auto_archive_keeps_completed_parent_while_it_has_active_child() {
    let root = temp_board("auto-archive-parent");
    let store = FsTaskStore::default();
    store.ensure_board(&root, "Test").unwrap();

    let parent = store
        .create(&root, draft("Parent done", "done", None))
        .unwrap();
    let child = store
        .create(
            &root,
            draft("Still working", "in-progress", Some(&parent.id)),
        )
        .unwrap();

    let parent_path = PathBuf::from(&parent.path);
    let text = fs::read_to_string(&parent_path).unwrap();
    let current_completed = parent.completed.as_deref().unwrap();
    fs::write(
        &parent_path,
        text.replace(
            &format!("completed: {current_completed}"),
            "completed: 2000-01-01",
        ),
    )
    .unwrap();

    let snapshot = store.load(&root).unwrap();
    let parent = snapshot
        .tasks
        .iter()
        .find(|task| task.id == parent.id)
        .unwrap();
    let child = snapshot
        .tasks
        .iter()
        .find(|task| task.id == child.id)
        .unwrap();
    assert!(!parent.archived);
    assert!(!child.archived);

    fs::remove_dir_all(root).unwrap();
}
