use std::{fs, path::PathBuf, time::{SystemTime, UNIX_EPOCH}};
use task_core::{FsTaskStore, TaskDraft, TaskKind, TaskStore};

fn board(label: &str) -> PathBuf {
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    std::env::temp_dir().join(format!("taskboard-crossboard-{label}-{}-{stamp}", std::process::id()))
}
fn draft(title: &str, depends_on: Vec<String>) -> TaskDraft {
    TaskDraft {
        reserved_id: None, reservation_token: None, title: title.into(),
        kind: TaskKind::Task, review_mode: task_core::ReviewMode::Inbox, deployable: None, repeat_every_days: None, next_due: None,
        depends_on, related_to: vec![], column: "ai-ready".into(),
        parent: None, category: None, icon: None, attention: vec![], question: None,
        blocked_reason: None, description: String::new(), acceptance_criteria: vec![],
        ai_notes: String::new(), user_notes: String::new(), notes: String::new(),
    }
}
#[test]
fn remote_hard_prerequisite_is_required_before_start_and_unlocks_on_done() {
    let store = FsTaskStore::default();
    let remote = board("remote");
    let local = board("local");
    store.ensure_board(&remote, "Remote").unwrap();
    store.ensure_board(&local, "Local").unwrap();
    let upstream = store.create(&remote, draft("Upstream", vec![])).unwrap();
    let reference = format!("{}#{}", remote.display(), upstream.id);
    let dependent = store.create(&local, draft("Dependent", vec![reference.clone()])).unwrap();
    let mut trying = dependent.clone();
    trying.column = "in-progress".into();
    assert!(store.update(&local, trying.clone()).is_err(), "unfinished remote dependency must block");

    let mut finished = upstream.clone();
    finished.column = "done".into();
    store.update(&remote, finished).unwrap();
    let current = store.load(&local).unwrap().tasks.into_iter().find(|task| task.id == dependent.id).unwrap();
    assert_eq!(current.depends_on, vec![reference]);
    trying.revision = current.revision;
    assert_eq!(store.update(&local, trying).unwrap().column, "in-progress");

    fs::remove_dir_all(&local).unwrap();
    fs::remove_dir_all(&remote).unwrap();
}
