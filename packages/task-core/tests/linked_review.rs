use std::{fs,time::{SystemTime,UNIX_EPOCH}};
use task_core::{FsTaskStore,TaskDraft,TaskKind,TaskStore,MutationActor,TaskQuestion};

fn draft(title:&str)->TaskDraft {
    TaskDraft{reserved_id:None,reservation_token:None,title:title.into(),kind:TaskKind::Task,review_mode:task_core::ReviewMode::Inbox,
      deployable:None,repeat_every_days:None,next_due:None,depends_on:vec![],related_to:vec![],
      column:"ai-ready".into(),parent:None,category:None,icon:None,attention:vec![],
      question:None,blocked_reason:None,description:"".into(),
      acceptance_criteria:vec!["Tooltip works".into()],ai_notes:"".into(),
      user_notes:"".into(),notes:"".into()}
}
#[test]
fn human_completed_review_approves_only_its_explicit_scope() {
    let dir=std::env::temp_dir().join(format!("review-link-{}",SystemTime::now()
        .duration_since(UNIX_EPOCH).unwrap().as_nanos()));
    let store=FsTaskStore::default();
    store.ensure_board(&dir,"Fixture").unwrap();
    let source=store.create(&dir,draft("Original")).unwrap();
    let mut review=draft("Review");
    review.kind=TaskKind::ForUser;
    review.column="in-progress".into();
    review.category=Some("Implementation Review".into());
    review.related_to=vec![source.id.clone()];
    let review=store.create(&dir,review).unwrap();
    let mut done=review;
    done.column="done".into();
    store.update_as(&dir,done,MutationActor::Human).unwrap();
    let source=store.load(&dir).unwrap().tasks.into_iter().find(|x|x.id==source.id).unwrap();
    assert!(source.completed.is_some());
    assert!(source.notes.contains("approved_parent_done"));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn needs_changes_from_done_reopens_original_and_preserves_feedback() {
    let dir=std::env::temp_dir().join(format!("review-needs-changes-{}",SystemTime::now()
        .duration_since(UNIX_EPOCH).unwrap().as_nanos()));
    let store=FsTaskStore::default();
    store.ensure_board(&dir,"Fixture").unwrap();
    let mut source=store.create(&dir,draft("Original")).unwrap();
    source.column="done".into();
    source=store.update(&dir,source).unwrap();
    let mut review=draft("Review");
    review.kind=TaskKind::ForUser;
    review.column="in-progress".into();
    review.category=Some("Implementation Review".into());
    review.related_to=vec![source.id.clone()];
    review.user_notes="Rotate graph downward; double-click must open session.".into();
    review.question=Some(TaskQuestion{
        request_type:Some("visual-review".into()),
        prompt:"Is the work accepted?".into(),options:vec![],
        answer:Some("Needs changes".into()),
    });
    let mut review=store.create(&dir,review).unwrap();
    review.column="done".into();
    store.update_as(&dir,review,MutationActor::Human).unwrap();
    let source=store.load(&dir).unwrap().tasks.into_iter().find(|t|t.id==source.id).unwrap();
    assert_eq!(source.column,"ai-ready");
    assert!(source.user_notes.contains("Rotate graph downward"));
    assert!(source.notes.contains("needs_changes"));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn partial_review_and_ambiguous_notes_do_not_auto_complete_original() {
    let dir=std::env::temp_dir().join(format!("review-partial-{}",SystemTime::now()
        .duration_since(UNIX_EPOCH).unwrap().as_nanos()));
    let store=FsTaskStore::default();
    store.ensure_board(&dir,"Fixture").unwrap();
    let mut source=draft("Original");
    source.acceptance_criteria.push("Authenticated reboot".into());
    let source=store.create(&dir,source).unwrap();
    let mut review=draft("Partial review");
    review.kind=TaskKind::ForUser;
    review.category=Some("Implementation Review".into());
    review.column="in-progress".into();
    review.related_to=vec![source.id.clone()];
    let mut review=store.create(&dir,review).unwrap();
    review.column="done".into();
    store.update_as(&dir,review,MutationActor::Human).unwrap();
    let source=store.load(&dir).unwrap().tasks.into_iter().find(|t|t.id==source.id).unwrap();
    assert!(source.completed.is_none());
    assert!(source.notes.contains("Remaining: [Authenticated reboot]"));
    fs::remove_dir_all(dir).unwrap();
}
