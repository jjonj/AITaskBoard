use task_core::{
    FsTaskStore, ReviewClose, ReviewInputArtifact, ReviewRequest, TaskAttention, TaskDraft,
    TaskKind, TaskStore,
};

#[test]
fn close_review_finishes_task_without_inventing_a_verdict() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let store = FsTaskStore::default();
    store.ensure_board(root, "Review test").unwrap();
    let task = store.create(root, TaskDraft {
        reserved_id: None, reservation_token: None, title: "Do we need this?".into(),
        kind: TaskKind::Task, deployable: None, repeat_every_days: None, next_due: None,
        depends_on: vec![], related_to: vec![], column: "ai-ready".into(),
        parent: None, category: None, icon: None, attention: vec![], question: None,
        blocked_reason: None, description: "The user may dismiss it.".into(),
        acceptance_criteria: vec![], ai_notes: "".into(), user_notes: "".into(), notes: "".into(),
    }).unwrap();
    let first = root.join("option-a.png");
    let second = root.join("option-b.png");
    std::fs::write(&first, b"test image A").unwrap();
    std::fs::write(&second, b"test image B").unwrap();
    let manifest = store.review_request(root, ReviewRequest {
        task_id: task.id.clone(), mode: "pick".into(), judge: "Select an alternative?".into(),
        criteria: vec![], artifacts: vec![
            ReviewInputArtifact { source: first.to_string_lossy().into_owned(), label: "A".into() },
            ReviewInputArtifact { source: second.to_string_lossy().into_owned(), label: "B".into() },
        ], references: vec![], previous: None, poster: None, top_k: None,
        implemented_by: None, openrig_gate: None,
        expected_task_revision: Some(task.revision.clone()), expected_manifest_revision: None,
    }).unwrap();
    let pending = store.load(root).unwrap().tasks.into_iter()
        .find(|entry| entry.id == task.id).unwrap();
    assert_eq!(pending.review_status.as_deref(), Some("open"));
    let request = ReviewClose {
        task_id: task.id.clone(), note: "Not interested; don't continue.".into(),
        expected_task_revision: pending.revision.clone(),
        expected_manifest_revision: manifest.revision.clone(),
    };
    let closed = store.review_close(root, request.clone()).unwrap();
    assert_eq!(closed.status, "closed");
    assert!(closed.closed_without_decision_at.is_some());
    assert!(closed.rounds.last().unwrap().result.is_none());
    let finished = store.load(root).unwrap().tasks.into_iter()
        .find(|entry| entry.id == task.id).unwrap();
    assert_eq!(finished.column, "done");
    assert!(finished.completed.is_some());
    assert_eq!(finished.review_status.as_deref(), Some("resolved"));
    assert!(finished.question.is_none());
    assert!(!finished.attention.contains(&TaskAttention::Question));
    assert!(finished.user_notes.contains("Not interested"));
    assert!(finished.description.contains("without decision"));
    assert!(store.review_close(root, request).is_err(), "stale/double close must fail");
}
