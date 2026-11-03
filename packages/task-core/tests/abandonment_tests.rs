use std::{fs, path::PathBuf, time::{Duration, SystemTime, UNIX_EPOCH}};
use task_core::{FsTaskStore, TaskAttention, TaskDraft, TaskKind, TaskQuestion, TaskStore};

fn board() -> (FsTaskStore, PathBuf) {
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let path = std::env::temp_dir().join(format!("taskboard-reclaim-{}-{stamp}", std::process::id()));
    let store = FsTaskStore::default();
    store.ensure_board(&path, "Test").unwrap();
    (store, path)
}

fn draft(column: &str) -> TaskDraft {
    TaskDraft {
        reserved_id: None, reservation_token: None, title: "Finish this task".into(),
        kind: TaskKind::Task, deployable: None, repeat_every_days: None, next_due: None,
        depends_on: vec![], related_to: vec![], column: column.into(), parent: None,
        category: None, icon: None, attention: vec![], question: None,
        blocked_reason: None, description: "Some work".into(), acceptance_criteria: vec![],
        ai_notes: "".into(), user_notes: "".into(), notes: "".into(),
    }
}

fn age_file(path: &str, hours: u64) {
    let text = fs::read_to_string(path).unwrap();
    let text = text.lines().filter(|line| !line.starts_with("last_activity_at:"))
        .collect::<Vec<_>>().join("\n") + "\n";
    fs::write(path, text).unwrap();
    let file = fs::OpenOptions::new().write(true).open(path).unwrap();
    file.set_modified(SystemTime::now() - Duration::from_secs(hours * 3600)).unwrap();
}

#[test]
fn claim_moves_into_progress_and_release_restores_original_todo() {
    let (store, root) = board();
    let task = store.create(&root, draft("ai-ready")).unwrap();
    let claim = store.claim(&root, &task.id, "agent").unwrap();
    assert_eq!(claim.column, "in-progress");
    assert!(fs::read_to_string(&claim.path).unwrap().contains("in_progress_origin: ai-ready"));
    let released = store.release_claim(&root, &claim.id, "agent").unwrap();
    assert_eq!(released.column, "ai-ready");
    assert_eq!(released.claimed_by, None);
    assert!(!fs::read_to_string(&released.path).unwrap().contains("in_progress_origin:"));
    let managed = store.claim_managed(&root, &task.id, "managed").unwrap();
    assert_eq!(managed.task.column, "in-progress");
    let released = store.release_managed(&root, &task.id, "managed", &managed.claim_token).unwrap();
    assert_eq!(released.column, "ai-ready");
    assert_eq!(released.claimed_by, None);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn abandoned_task_requeues_once_and_preserves_the_timeline() {
    let (store, root) = board();
    let task = store.create(&root, draft("ai-ready")).unwrap();
    let claimed = store.claim(&root, &task.id, "agent").unwrap();
    age_file(&claimed.path, 5);
    let recovered = store.load(&root).unwrap().tasks.into_iter().find(|t| t.id == task.id).unwrap();
    assert_eq!(recovered.column, "ai-ready");
    assert_eq!(recovered.claimed_by, None);
    assert!(recovered.description.contains("In Progress timed out after 4 hours"));
    let again = store.load(&root).unwrap().tasks.into_iter().find(|t| t.id == task.id).unwrap();
    assert_eq!(again.description.matches("In Progress timed out").count(), 1);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn pending_inbox_preserves_its_question_when_inactive_task_requeues() {
    let (store, root) = board();
    let task = store.create(&root, draft("ai-ready")).unwrap();
    let mut edited = task.clone();
    edited.attention.push(TaskAttention::Question);
    edited.question = Some(TaskQuestion { request_type: Some("decision".into()),
        prompt: "Choose one?".into(), options: vec!["A".into(), "B".into()], answer: None });
    let mut pending = store.update(&root, edited).unwrap();
    assert_eq!(pending.column, "ai-ready");
    // Human manually starts the task without taking an AI claim.
    pending.column = "in-progress".into();
    let pending = store.update_as(&root, pending, task_core::MutationActor::Human).unwrap();
    age_file(&pending.path, 5);
    let read = store.load(&root).unwrap().tasks.into_iter().find(|t| t.id == task.id).unwrap();
    assert_eq!(read.column, "ai-ready");
    assert_eq!(read.claimed_by, None);
    assert_eq!(read.question.as_ref().unwrap().prompt, "Choose one?");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn recent_and_t3_managed_claims_obey_four_hour_lease() {
    let (store, root) = board();
    let task = store.create(&root, draft("ai-ready")).unwrap();
    let managed = store.claim_managed(&root, &task.id,
        "t3-browser:8fd23b67-0a26-4082-8972-4425b500164f").unwrap();
    age_file(&managed.task.path, 3);
    let recent = store.load(&root).unwrap().tasks.into_iter()
        .find(|t| t.id == task.id).unwrap();
    assert_eq!(recent.column, "in-progress");
    assert!(recent.claimed_by.is_some());
    age_file(&recent.path, 5);
    let expired = store.load(&root).unwrap().tasks.into_iter()
        .find(|t| t.id == task.id).unwrap();
    assert_eq!(expired.column, "ai-ready");
    assert_eq!(expired.claimed_by, None);
    assert!(!fs::read_to_string(&expired.path).unwrap().contains("\nclaim_token:"));
    assert!(store.release_managed(&root, &task.id,
        "t3-browser:8fd23b67-0a26-4082-8972-4425b500164f", &managed.claim_token).is_err());
    fs::remove_dir_all(root).unwrap();
}
