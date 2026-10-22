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
        kind: task_core::TaskKind::Task,
        depends_on: Vec::new(),
        related_to: Vec::new(),
        column: column.into(),
        parent: parent.map(str::to_string),
        category: None,
        icon: None,
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

    let mut epic = draft("Parent", "ai-ready", None);
    epic.kind = task_core::TaskKind::Epic;
    epic.acceptance_criteria.clear();
    let created = store.create(&root, epic).unwrap();
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
fn task_icons_are_optional_and_roundtrip_in_frontmatter() {
    let root = temp_board("icons");
    let store = FsTaskStore::default();
    store.ensure_board(&root, "Test").unwrap();

    let plain = store.create(&root, draft("No icon", "ai-ready", None)).unwrap();
    assert_eq!(plain.icon, None);
    assert!(!fs::read_to_string(&plain.path).unwrap().contains("icon:"));

    let mut typed = draft("Fix something", "ai-ready", None);
    typed.icon = Some("bug".into());
    let created = store.create(&root, typed).unwrap();
    assert_eq!(created.icon.as_deref(), Some("bug"));
    assert!(fs::read_to_string(&created.path).unwrap().contains("icon: bug"));

    let mut loaded = store.load(&root).unwrap().tasks.into_iter()
        .find(|task| task.id == created.id).unwrap();
    assert_eq!(loaded.icon.as_deref(), Some("bug"));
    loaded.icon = Some("validation".into());
    let updated = store.update(&root, loaded).unwrap();
    assert_eq!(updated.icon.as_deref(), Some("validation"));

    let mut updated = updated;
    for icon in ["design", "visual-work"] {
        updated.icon = Some(icon.into());
        updated = store.update(&root, updated).unwrap();
        assert_eq!(updated.icon.as_deref(), Some(icon));
        assert!(fs::read_to_string(&updated.path).unwrap().contains(&format!("icon: {icon}")));
    }

    let mut cleared = updated;
    cleared.icon = None;
    let cleared = store.update(&root, cleared).unwrap();
    assert_eq!(cleared.icon, None);
    assert!(!fs::read_to_string(&cleared.path).unwrap().contains("icon:"));

    let mut invalid = draft("Invalid icon", "ai-ready", None);
    invalid.icon = Some("nonexistent".into());
    assert!(matches!(store.create(&root, invalid), Err(TaskStoreError::Invalid(_))));
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

    let mut epic = draft("Parent", "ai-ready", None);
    epic.kind = task_core::TaskKind::Epic;
    epic.acceptance_criteria.clear();
    let parent = store.create(&root, epic).unwrap();
    let child = store.create(&root, draft("Child", "ai-ready", Some(&parent.id))).unwrap();

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
fn archive_all_column_is_revision_checked_and_preserves_epic_children() {
    let root = temp_board("archive-all");
    let store = FsTaskStore::default();
    store.ensure_board(&root, "Test").unwrap();

    let mut draft_epic = draft("Parent", "draft", None);
    draft_epic.kind = task_core::TaskKind::Epic;
    draft_epic.acceptance_criteria.clear();
    let parent = store.create(&root, draft_epic).unwrap();
    let child = store.create(&root, draft("Child", "done", Some(&parent.id))).unwrap();
    let outside = store.create(&root, draft("Outside", "ai-ready", None)).unwrap();
    // The parent follows its only descendant and is now automatically Done.
    let snapshot = store.load(&root).unwrap();
    let parent = snapshot.tasks.iter().find(|t| t.id == parent.id).unwrap().clone();
    assert_eq!(parent.column, "done");

    let expected = vec![
        (parent.id.clone(), parent.revision.clone()),
        (child.id.clone(), child.revision.clone()),
    ];
    let mut modified = child.clone();
    modified.title = "Externally changed".into();
    let modified = store.update(&root, modified).unwrap();
    let error = store.archive_column(&root, "done", &expected).unwrap_err();
    assert!(matches!(error, TaskStoreError::Conflict(_)));
    assert!(store.load(&root).unwrap().tasks.iter().all(|task| !task.archived));

    let expected = vec![
        (parent.id.clone(), parent.revision.clone()),
        (modified.id.clone(), modified.revision.clone()),
    ];
    let before_archiving = store.load(&root).unwrap();
    let archived = store.archive_column(&root, "done", &expected).unwrap();
    assert!(archived.tasks.iter().find(|task| task.id == parent.id).unwrap().archived);
    assert!(archived.tasks.iter().find(|task| task.id == child.id).unwrap().archived);
    assert!(!archived.tasks.iter().find(|task| task.id == outside.id).unwrap().archived);
    let restored = store.restore_snapshot(&root, before_archiving, archived).unwrap();
    assert!(!restored.tasks.iter().find(|task| task.id == parent.id).unwrap().archived);
    assert!(!restored.tasks.iter().find(|task| task.id == child.id).unwrap().archived);

    // An epic containing an unfinished child cannot be marked Done.
    let mut active_epic = draft("Active Epic", "draft", None);
    active_epic.kind = task_core::TaskKind::Epic;
    active_epic.acceptance_criteria.clear();
    let active_parent = store.create(&root, active_epic).unwrap();
    let child_outside = store.create(&root, draft("ChildOutside", "ai-ready", Some(&active_parent.id))).unwrap();
    let snapshot = store.load(&root).unwrap();
    assert_eq!(snapshot.tasks.iter().find(|t| t.id == active_parent.id).unwrap().column, "ai-ready");
    assert_eq!(snapshot.tasks.iter().find(|t| t.id == child_outside.id).unwrap().parent.as_deref(), Some(active_parent.id.as_str()));
    let expected = snapshot.tasks.iter()
        .filter(|task| !task.archived && task.column == "done")
        .map(|task| (task.id.clone(), task.revision.clone())).collect::<Vec<_>>();
    store.archive_column(&root, "done", &expected).unwrap();
    assert!(!store.load(&root).unwrap().tasks.iter().find(|t| t.id == active_parent.id).unwrap().archived);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn completed_cards_have_stable_high_resolution_completion_order() {
    let root = temp_board("completed-at");
    let store = FsTaskStore::default();
    store.ensure_board(&root, "Test").unwrap();

    let first = store.create(&root, draft("First", "ai-ready", None)).unwrap();
    let second = store.create(&root, draft("Second", "ai-ready", None)).unwrap();

    let mut complete_first = first;
    complete_first.column = "done".into();
    let done_first = store.update(&root, complete_first).unwrap();
    let first_timestamp = done_first.completed_at.clone().expect("completion timestamp");
    assert!(chrono::DateTime::parse_from_rfc3339(&first_timestamp).is_ok());
    assert!(fs::read_to_string(&done_first.path).unwrap().contains("completed_at:"));

    std::thread::sleep(std::time::Duration::from_millis(12));
    let mut complete_second = second;
    complete_second.column = "done".into();
    let done_second = store.update(&root, complete_second).unwrap();
    assert!(done_second.completed_at.as_deref().unwrap() > first_timestamp.as_str());

    let mut edit_first = done_first;
    edit_first.title = "First edited".into();
    let edit_first = store.update(&root, edit_first).unwrap();
    assert_eq!(edit_first.completed_at.as_deref(), Some(first_timestamp.as_str()));

    let mut reopen = edit_first;
    reopen.column = "ai-ready".into();
    let reopened = store.update(&root, reopen).unwrap();
    assert!(reopened.completed.is_none());
    assert!(reopened.completed_at.is_none());
    let mut recomplete = reopened;
    recomplete.column = "done".into();
    let recompleted = store.update(&root, recomplete).unwrap();
    assert!(recompleted.completed_at.as_deref().unwrap() > first_timestamp.as_str());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn expired_snoozes_wake_and_show_temporary_awake_indicator() {
    let root = temp_board("column-snooze");
    let store = FsTaskStore::default();
    store.ensure_board(&root, "Test").unwrap();

    let mut config = store.load(&root).unwrap().config;
    let column = config.columns.iter_mut().find(|column| column.id == "draft").unwrap();
    column.hidden = true;
    column.snoozed_until = Some((chrono::Utc::now() - chrono::Duration::minutes(2)).to_rfc3339());
    store.save_config(&root, config).unwrap();

    let woke = store.load(&root).unwrap();
    let column = woke.config.columns.iter().find(|column| column.id == "draft").unwrap();
    assert!(!column.hidden);
    assert!(column.snoozed_until.is_none());
    assert!(column.awake_until.is_some());
    assert!(fs::read_to_string(root.join("_board.md")).unwrap().contains("awake_until:"));

    let mut config = woke.config;
    let column = config.columns.iter_mut().find(|column| column.id == "draft").unwrap();
    column.awake_until = Some((chrono::Utc::now() - chrono::Duration::hours(1)).to_rfc3339());
    store.save_config(&root, config).unwrap();
    let expired = store.load(&root).unwrap();
    let column = expired.config.columns.iter().find(|column| column.id == "draft").unwrap();
    assert!(column.awake_until.is_none());

    let mut config = expired.config;
    let column = config.columns.iter_mut().find(|column| column.id == "draft").unwrap();
    column.hidden = true;
    column.snoozed_until = Some((chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339());
    store.save_config(&root, config).unwrap();
    let still_snoozed = store.load(&root).unwrap();
    let column = still_snoozed.config.columns.iter().find(|column| column.id == "draft").unwrap();
    assert!(column.hidden);
    assert!(column.snoozed_until.is_some());
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
        snoozed_until: None,
        awake_until: None,
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
fn auto_archive_keeps_epic_while_it_has_active_child() {
    let root = temp_board("auto-archive-parent");
    let store = FsTaskStore::default();
    store.ensure_board(&root, "Test").unwrap();
    let mut epic_draft = draft("Parent milestone", "draft", None);
    epic_draft.kind = task_core::TaskKind::Epic;
    epic_draft.acceptance_criteria.clear();
    let parent = store.create(&root, epic_draft).unwrap();
    let child = store.create(&root, draft("Still working", "in-progress", Some(&parent.id))).unwrap();
    let parent_path = PathBuf::from(&parent.path);
    let text = fs::read_to_string(&parent_path).unwrap();
    fs::write(&parent_path, text.replacen("created:", "completed: 2000-01-01\ncreated:", 1)).unwrap();
    let snapshot = store.load(&root).unwrap();
    let parent = snapshot.tasks.iter().find(|t| t.id == parent.id).unwrap();
    let child = snapshot.tasks.iter().find(|t| t.id == child.id).unwrap();
    assert!(!parent.archived);
    assert!(!child.archived);
    assert_eq!(parent.column, "in-progress");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn epic_automatically_follows_descendants_and_refuses_agent_execution() {
    use task_core::TaskKind;
    let root = temp_board("epic-auto");
    let store = FsTaskStore::default();
    store.ensure_board(&root, "Test").unwrap();
    let mut epic_draft = draft("Big combat work", "ai-ready", None);
    epic_draft.kind = TaskKind::Epic;
    epic_draft.acceptance_criteria.clear();
    let epic = store.create(&root, epic_draft).unwrap();
    assert!(epic.id.starts_with("WTE-"));
    assert_eq!(epic.column, "draft");
    assert!(matches!(store.claim(&root, &epic.id, "agent"), Err(TaskStoreError::Invalid(_))));
    let mut child = store.create(&root, draft("Executable", "ai-ready", Some(&epic.id))).unwrap();
    let snapshot = store.load(&root).unwrap();
    let epic = snapshot.tasks.iter().find(|t| t.id == epic.id).unwrap();
    assert_eq!(epic.column, "ai-ready");
    let mut illegally_moved = epic.clone();
    illegally_moved.column = "in-progress".into();
    assert!(matches!(store.update(&root, illegally_moved), Err(TaskStoreError::Invalid(_))));

    child.column = "in-progress".into();
    child = store.update(&root, child).unwrap();
    assert_eq!(store.load(&root).unwrap().tasks.iter().find(|t| t.id == epic.id).unwrap().column, "in-progress");
    child.column = "done".into();
    store.update(&root, child).unwrap();
    let latest = store.load(&root).unwrap();
    let done_epic = latest.tasks.iter().find(|t| t.id == epic.id).unwrap();
    assert_eq!(done_epic.column, "done");
    assert!(done_epic.completed.is_some());
    assert_eq!(done_epic.claimed_by, None);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn dependencies_restrict_claim_and_start_prevent_cycles_and_support_soft_links() {
    let root = temp_board("hard-deps");
    let store = FsTaskStore::default();
    store.ensure_board(&root, "Test").unwrap();
    let first = store.create(&root, draft("Prerequisite", "ai-ready", None)).unwrap();
    let mut dependent_draft = draft("Depends", "ai-ready", None);
    dependent_draft.depends_on = vec![first.id.clone()];
    dependent_draft.related_to = vec![first.id.clone()];
    let dependent = store.create(&root, dependent_draft).unwrap();
    assert!(matches!(store.claim(&root, &dependent.id, "agent"), Err(TaskStoreError::Invalid(_))));
    let mut moved = dependent.clone();
    moved.column = "in-progress".into();
    assert!(matches!(store.update(&root, moved), Err(TaskStoreError::Invalid(_))));
    let mut cyc = first.clone();
    cyc.depends_on = vec![dependent.id.clone()];
    assert!(matches!(store.update(&root, cyc), Err(TaskStoreError::Invalid(_))));
    let mut bad = draft("Unknown link", "ai-ready", None);
    bad.depends_on = vec!["WT-999999".into()];
    assert!(matches!(store.create(&root, bad), Err(TaskStoreError::Invalid(_))));
    let mut completed_first = first;
    completed_first.column = "done".into();
    store.update(&root, completed_first).unwrap();
    let claimed = store.claim(&root, &dependent.id, "agent").unwrap();
    assert_eq!(claimed.claimed_by.as_deref(), Some("agent"));
    let mut moved = claimed;
    moved.column = "in-progress".into();
    store.update(&root, moved).unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn for_user_task_is_persisted_not_agent_claimable_and_can_be_human_completed() {
    use task_core::TaskKind;
    let root = temp_board("for-user");
    let store = FsTaskStore::default();
    store.ensure_board(&root, "Test").unwrap();

    let mut human_draft = draft("Approve reference animations", "in-progress", None);
    human_draft.kind = TaskKind::ForUser;
    let human = store.create(&root, human_draft).unwrap();
    assert!(human.id.starts_with("WT-"));
    assert!(fs::read_to_string(&human.path).unwrap().contains("kind: for_user"));
    assert_eq!(store.load(&root).unwrap().tasks.iter().find(|t| t.id == human.id).unwrap().kind, TaskKind::ForUser);
    assert!(matches!(
        store.claim(&root, &human.id, "agent"),
        Err(TaskStoreError::Invalid(message)) if message.contains("FOR USER")
    ));
    // The regular UI update is the human pathway, including marking Done.
    let mut user_finished = human.clone();
    user_finished.column = "done".into();
    let completed = store.update(&root, user_finished).unwrap();
    assert_eq!(completed.completed.is_some(), true);
    assert_eq!(completed.kind, TaskKind::Task); // Done clears human-only status.

    // A dependent AI task can be started only when the human card is Done.
    let mut dependent_draft = draft("Finalize animations", "ai-ready", None);
    dependent_draft.depends_on = vec![completed.id.clone()];
    let dependent = store.create(&root, dependent_draft).unwrap();
    assert_eq!(store.claim(&root, &dependent.id, "agent").unwrap().claimed_by.as_deref(), Some("agent"));

    assert_eq!(store.load(&root).unwrap().tasks.iter().find(|t| t.id == completed.id).unwrap().kind, TaskKind::Task);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn for_user_task_satisfies_dependency_only_after_human_completion() {
    use task_core::TaskKind;
    let root = temp_board("for-user-gates");
    let store = FsTaskStore::default();
    store.ensure_board(&root, "Test").unwrap();
    let mut draft_human = draft("Make decision", "in-progress", None);
    draft_human.kind = TaskKind::ForUser;
    let human = store.create(&root, draft_human).unwrap();
    let mut agent_draft = draft("Implement decision", "ai-ready", None);
    agent_draft.depends_on = vec![human.id.clone()];
    let agent = store.create(&root, agent_draft).unwrap();
    assert!(matches!(store.claim(&root, &agent.id, "ai"), Err(TaskStoreError::Invalid(_))));

    let mut finished = human.clone();
    finished.column = "done".into();
    let finished = store.update(&root, finished).unwrap();
    store.claim(&root, &agent.id, "ai").unwrap();
    // Reopening a user task reinstates the agent's dependency gate.
    let mut reopened = finished;
    reopened.column = "in-progress".into();
    store.update(&root, reopened).unwrap();
    let mut new_task = draft("Follow-up", "ai-ready", None);
    new_task.depends_on = vec![human.id.clone()];
    let created = store.create(&root, new_task).unwrap();
    assert!(matches!(store.claim(&root, &created.id, "ai"), Err(TaskStoreError::Invalid(_))));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn only_epics_are_allowed_as_parents_and_epics_are_roots() {
    use task_core::TaskKind;
    let root = temp_board("strict-epic-hierarchy");
    let store = FsTaskStore::default();
    store.ensure_board(&root, "Test").unwrap();
    let normal = store.create(&root, draft("Normal", "ai-ready", None)).unwrap();
    let mut under_task = draft("Child", "ai-ready", Some(&normal.id));
    assert!(matches!(store.create(&root, under_task.clone()),
        Err(TaskStoreError::Invalid(message)) if message.contains("not an epic")));
    let mut epic_draft = draft("Epic", "draft", None);
    epic_draft.kind = TaskKind::Epic;
    epic_draft.acceptance_criteria.clear();
    let epic = store.create(&root, epic_draft).unwrap();
    under_task.parent = Some(epic.id.clone());
    let child = store.create(&root, under_task).unwrap();
    assert_eq!(child.parent.as_deref(), Some(epic.id.as_str()));
    let mut bad_update = child.clone();
    bad_update.parent = Some(normal.id.clone());
    assert!(matches!(store.update(&root, bad_update), Err(TaskStoreError::Invalid(_))));
    let mut bad_epic = draft("Nested epic", "draft", Some(&epic.id));
    bad_epic.kind = TaskKind::Epic;
    bad_epic.acceptance_criteria.clear();
    assert!(matches!(store.create(&root, bad_epic), Err(TaskStoreError::Invalid(_))));
    let mut child_epic = epic.clone();
    child_epic.parent = Some(normal.id);
    assert!(matches!(store.update(&root, child_epic), Err(TaskStoreError::Invalid(_))));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn epics_reject_icons_attention_questions_criteria_and_ai_notes() {
    use task_core::{TaskKind, TaskAttention, TaskQuestion};
    let root = temp_board("epic-field-contract");
    let store = FsTaskStore::default();
    store.ensure_board(&root, "Test").unwrap();
    let mut base = draft("Epic", "draft", None);
    base.kind = TaskKind::Epic;
    base.acceptance_criteria.clear();
    base.user_notes = "Keep coordination notes".into();
    let epic = store.create(&root, base.clone()).unwrap();
    assert_eq!(epic.user_notes, "Keep coordination notes");
    for field in 0..4 {
        let mut invalid = base.clone();
        match field {
            0 => invalid.icon = Some("design".into()),
            1 => invalid.attention.push(TaskAttention::NeedsUser),
            2 => invalid.acceptance_criteria.push("Implementation criterion".into()),
            3 => invalid.ai_notes = "Agent handoff".into(),
            _ => unreachable!()
        }
        assert!(matches!(store.create(&root, invalid), Err(TaskStoreError::Invalid(_))));
    }
    let mut question = base.clone();
    question.attention.push(TaskAttention::Question);
    question.question = Some(TaskQuestion { prompt: "Choose".into(), options: vec![], answer: None });
    assert!(matches!(store.create(&root, question), Err(TaskStoreError::Invalid(_))));
    let mut changed = epic.clone();
    changed.ai_notes = "Should be in Notes instead".into();
    assert!(matches!(store.update(&root, changed), Err(TaskStoreError::Invalid(_))));
    let mut retyped = epic;
    retyped.kind = TaskKind::Task;
    let child = store.create(&root, draft("Child", "ai-ready", Some(&retyped.id))).unwrap();
    assert!(matches!(store.update(&root, retyped), Err(TaskStoreError::Invalid(_))));
    assert!(!child.archived);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn drop_position_metadata_roundtrips_and_is_revision_checked() {
    let root = temp_board("manual-positions");
    let store = FsTaskStore::default();
    store.ensure_board(&root, "Test").unwrap();
    let anchor = store.create(&root, draft("Anchor", "ai-ready", None)).unwrap();
    let moved = store.create(&root, draft("Reposition", "ai-ready", None)).unwrap();
    let mut placing = moved.clone();
    placing.position_after = Some(anchor.id.clone());
    placing.positioned_at = Some("2026-10-15T12:00:00Z".into());
    let placed = store.update(&root, placing).unwrap();
    assert_eq!(placed.position_after.as_deref(), Some(anchor.id.as_str()));
    assert!(fs::read_to_string(&placed.path).unwrap().contains("position_after:"));
    let loaded = store.load(&root).unwrap();
    assert_eq!(loaded.tasks.iter().find(|t| t.id == placed.id).unwrap().positioned_at.as_deref(), Some("2026-10-15T12:00:00Z"));
    let mut stale = moved;
    stale.position_after = Some(anchor.id.clone());
    assert!(matches!(store.update(&root, stale), Err(TaskStoreError::Conflict(_))));
    let mut invalid = placed;
    invalid.position_after = Some(invalid.id.clone());
    assert!(matches!(store.update(&root, invalid), Err(TaskStoreError::Invalid(_))));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn epic_files_have_only_design_and_notes_no_execution_sections() {
    let root = temp_board("epic-markdown");
    let store = FsTaskStore::default();
    store.ensure_board(&root, "Test").unwrap();
    let mut d = draft("Coordination Epic", "draft", None);
    d.kind = task_core::TaskKind::Epic;
    d.acceptance_criteria.clear();
    d.notes = "Design scope and decisions".into();
    let created = store.create(&root, d).unwrap();
    let raw = fs::read_to_string(&created.path).unwrap();
    assert!(raw.contains("## Notes"));
    assert!(raw.contains("Design scope and decisions"));
    assert!(!raw.contains("## Acceptance Criteria"));
    assert!(!raw.contains("## AI Notes"));
    let mut changed = created;
    changed.description = "Updated design description".into();
    let updated = store.update(&root, changed).unwrap();
    let raw = fs::read_to_string(&updated.path).unwrap();
    assert!(!raw.contains("## Acceptance Criteria"));
    assert!(!raw.contains("## AI Notes"));
    assert!(raw.contains("Updated design description"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn archive_done_older_than_three_days_is_atomic_and_undoable() {
    use chrono::{Duration, SecondsFormat, Utc};
    let root = temp_board("archive-aged-done");
    let store = FsTaskStore::default();
    store.ensure_board(&root, "Test").unwrap();
    let old = store.create(&root, draft("Four days in Done", "done", None)).unwrap();
    let recent = store.create(&root, draft("Today in Done", "done", None)).unwrap();
    let cutoff = (Utc::now() - Duration::days(3)).to_rfc3339_opts(SecondsFormat::Secs, true);
    let mut changed = old.clone();
    changed.completed_at = Some((Utc::now() - Duration::days(4)).to_rfc3339_opts(SecondsFormat::Secs, true));
    let old = store.update(&root, changed).unwrap();
    let before = store.load(&root).unwrap();

    // Stale expected revisions must abort without archiving anything.
    let error = store.archive_column_older_than(&root, "done", &cutoff, &[
        (old.id.clone(), "stale".into()),
    ]).unwrap_err();
    assert!(matches!(error, TaskStoreError::Conflict(_)));
    assert!(!store.load(&root).unwrap().tasks.iter().any(|t| t.archived));

    let after = store.archive_column_older_than(&root, "done", &cutoff, &[
        (old.id.clone(), old.revision.clone()),
    ]).unwrap();
    assert!(after.tasks.iter().find(|t| t.id == old.id).unwrap().archived);
    assert!(!after.tasks.iter().find(|t| t.id == recent.id).unwrap().archived);
    let restored = store.restore_snapshot(&root, before, after).unwrap();
    assert!(!restored.tasks.iter().find(|t| t.id == old.id).unwrap().archived);

    assert!(matches!(
        store.archive_column_older_than(&root, "ai-ready", &cutoff, &[]),
        Err(TaskStoreError::Invalid(_))
    ));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn age_archival_preserves_epic_and_young_child_without_detaching() {
    use chrono::{Duration, SecondsFormat, Utc};
    use task_core::TaskKind;
    let root = temp_board("archive-aged-epic");
    let store = FsTaskStore::default();
    store.ensure_board(&root, "Test").unwrap();

    let mut draft_epic = draft("Milestone", "draft", None);
    draft_epic.kind = TaskKind::Epic;
    draft_epic.acceptance_criteria.clear();
    let epic = store.create(&root, draft_epic).unwrap();
    let old = store.create(&root, draft("Old finished", "done", Some(&epic.id))).unwrap();
    let recent = store.create(&root, draft("Recent finished", "done", Some(&epic.id))).unwrap();

    let mut older = old.clone();
    older.completed_at = Some((Utc::now() - Duration::days(4)).to_rfc3339_opts(SecondsFormat::Secs, true));
    let old = store.update(&root, older).unwrap();

    let current = store.load(&root).unwrap();
    assert_eq!(current.tasks.iter().find(|task| task.id == epic.id).unwrap().column, "done");

    let mut old_epic = current.tasks.iter().find(|task| task.id == epic.id).unwrap().clone();
    old_epic.completed_at = Some((Utc::now() - Duration::days(4)).to_rfc3339_opts(SecondsFormat::Secs, true));
    store.update(&root, old_epic).unwrap();

    let cutoff = (Utc::now() - Duration::days(3)).to_rfc3339_opts(SecondsFormat::Secs, true);
    let after = store.archive_column_older_than(&root, "done", &cutoff, &[
        (old.id.clone(), old.revision.clone()),
    ]).unwrap();
    assert!(after.tasks.iter().find(|task| task.id == old.id).unwrap().archived);
    assert!(!after.tasks.iter().find(|task| task.id == epic.id).unwrap().archived);
    let recent_after = after.tasks.iter().find(|task| task.id == recent.id).unwrap();
    assert!(!recent_after.archived);
    assert_eq!(recent_after.parent.as_deref(), Some(epic.id.as_str()));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn completing_human_task_clears_for_user_status_in_persisted_markdown() {
    let root = temp_board("done-clears-human-status");
    let store = FsTaskStore::default();
    store.ensure_board(&root, "Test").unwrap();
    let mut human = draft("Human approval", "in-progress", None);
    human.kind = task_core::TaskKind::ForUser;
    human.attention.push(task_core::TaskAttention::NeedsUser);
    let mut task = store.create(&root, human).unwrap();
    task.column = "done".into();
    let done = store.update(&root, task).unwrap();
    assert_eq!(done.kind, task_core::TaskKind::Task);
    assert!(done.completed.is_some());
    assert!(!done.attention.contains(&task_core::TaskAttention::NeedsUser));
    let reloaded = store.load(&root).unwrap();
    let saved = reloaded.tasks.iter().find(|task| task.id == done.id).unwrap();
    assert_eq!(saved.kind, task_core::TaskKind::Task);
    assert!(!fs::read_to_string(&done.path).unwrap().contains("needs-user"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn finished_dependencies_release_blocked_tasks_without_clearing_other_blockers() {
    let root = temp_board("auto-unblock-deps");
    let store = FsTaskStore::default();
    store.ensure_board(&root, "Test").unwrap();
    let first = store.create(&root, draft("First prerequisite", "in-progress", None)).unwrap();
    let second = store.create(&root, draft("Second prerequisite", "in-progress", None)).unwrap();

    let mut waiting = draft("Waiting on both", "blocked", None);
    waiting.depends_on = vec![first.id.clone(), second.id.clone()];
    let waiting = store.create(&root, waiting).unwrap();

    let mut independently_blocked = draft("Separate explicit blocker", "blocked", None);
    independently_blocked.depends_on = vec![first.id.clone()];
    independently_blocked.attention.push(task_core::TaskAttention::Blocked);
    let independently_blocked = store.create(&root, independently_blocked).unwrap();

    let mut first_done = first.clone();
    first_done.column = "done".into();
    store.update(&root, first_done).unwrap();
    let after_first = store.load(&root).unwrap();
    assert_eq!(after_first.tasks.iter().find(|t| t.id == waiting.id).unwrap().column, "blocked");

    let mut second_done = second.clone();
    second_done.column = "done".into();
    store.update(&root, second_done).unwrap();
    let released = store.load(&root).unwrap();
    assert_eq!(released.tasks.iter().find(|t| t.id == waiting.id).unwrap().column, "ai-ready");
    assert_eq!(
        released.tasks.iter().find(|t| t.id == independently_blocked.id).unwrap().column,
        "blocked"
    );
    fs::remove_dir_all(root).unwrap();
}
