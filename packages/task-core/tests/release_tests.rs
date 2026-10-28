use std::{fs, path::PathBuf, time::{SystemTime, UNIX_EPOCH}};
use task_core::{FsTaskStore, TaskDraft, TaskKind, TaskStore, ReleaseStage, SourceRef, TaskStoreError, challenge_agent_block};
fn root(label: &str) -> PathBuf {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    std::env::temp_dir().join(format!("taskboard-release-{label}-{}-{now}", std::process::id()))
}
fn draft(title: &str) -> TaskDraft {
    TaskDraft {
        reserved_id: None, reservation_token: None,
        kind: TaskKind::Task, deployable: None, blocked_reason: None,
        repeat_every_days: None, next_due: None,
        depends_on: vec![], related_to: vec![], title: title.into(), column: "in-progress".into(), parent: None,
        category: None, icon: None, attention: vec![], question: None,
        description: String::new(), acceptance_criteria: vec![], ai_notes: String::new(), user_notes: String::new(), notes: String::new(),
    }
}
#[test]
fn lazy_rollover_is_idempotent_and_opt_in() {
    let root = root("idempotent"); let store = FsTaskStore::default();
    store.ensure_board(&root, "QA").unwrap();
    let mut pre = store.create(&root, draft("Old")).unwrap();
    pre.column = "done".into(); store.update(&root, pre).unwrap();
    assert!(store.releases(&root).unwrap().releases.is_empty());
    store.release_enable(&root, true).unwrap();
    assert!(store.releases(&root).unwrap().releases.is_empty(), "historical must not join unprompted");
    let mut done = store.create(&root, draft("New")).unwrap();
    done.column = "done".into();
    let done = store.update(&root, done).unwrap();
    let initial = store.releases(&root).unwrap();
    assert_eq!(initial.releases.len(), 1);
    assert_eq!(initial.releases[0].members[0].task_id, done.id);
    store.update(&root, done.clone()).unwrap();
    assert_eq!(store.releases(&root).unwrap().releases[0].members.len(), 1);
    let mut optout = store.create(&root, draft("Nondeployable")).unwrap();
    optout.deployable = Some(false);
    optout.column = "done".into(); store.update(&root, optout).unwrap();
    assert_eq!(store.releases(&root).unwrap().releases[0].members.len(), 1);
    let explicit = store.reconcile_releases(&root, true).unwrap();
    assert_eq!(explicit.releases[0].members.len(), 2);
    assert!(explicit.releases[0].members.iter().any(|m| m.task_id == done.id));
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn attempts_have_immutable_two_repo_snapshots_and_later_done_still_collects() {
    let root = root("attempts"); let store = FsTaskStore::default();
    store.ensure_board(&root, "QA").unwrap(); store.release_enable(&root, true).unwrap();
    let mut first = store.create(&root, draft("Feature 1")).unwrap();
    first.column="done".into(); let first = store.update(&root, first).unwrap();
    let rel = store.releases(&root).unwrap().releases[0].id.clone();
    store.release_edit(&root, "source", &rel, Some(&first.id),
        Some(SourceRef{repository:"T3".into(), commit:"isolated-abc".into(), integrated_ref: None}), None).unwrap();
    assert_eq!(store.releases(&root).unwrap().releases[0].members[0].stage, ReleaseStage::AwaitingIntegration);
    store.release_edit(&root, "source", &rel, Some(&first.id),
        Some(SourceRef{repository:"T3".into(), commit:"isolated-abc".into(), integrated_ref: Some("main-def".into())}), None).unwrap();
    store.release_edit(&root, "source", &rel, Some(&first.id),
        Some(SourceRef{repository:"OpenRig".into(), commit:"rig-123".into(), integrated_ref: Some("rig-main-456".into())}), None).unwrap();
    let pre = store.release_prepare(&root, &rel, "manifest only; no deployment").unwrap();
    assert_eq!(pre.attempts[0].members.len(), 1); assert_eq!(pre.attempts[0].manifest.len(), 2);
    let mut second = store.create(&root, draft("Feature 2")).unwrap();
    second.column="done".into(); let second = store.update(&root, second).unwrap();
    let still = store.releases(&root).unwrap();
    assert_eq!(still.releases[0].members.len(), 2);
    assert_eq!(still.releases[0].attempts[0].members.len(), 1, "snapshot must not change as workers finish");
    store.release_outcome(&root, &rel, 1, "deployed", "binary hash + operator deployment evidence").unwrap();
    store.release_verify_member(&root, &rel, 1, &first.id, true, "manual smoke check").unwrap();
    let after = store.releases(&root).unwrap();
    assert_eq!(after.releases[0].members.iter().find(|m| m.task_id == first.id).unwrap().stage, ReleaseStage::Verified);
    assert_eq!(after.releases[0].members.iter().find(|m| m.task_id == second.id).unwrap().stage, ReleaseStage::AwaitingIntegration);
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn failed_attempt_creates_linked_repair_not_reopening_done() {
    let root = root("failure"); let store = FsTaskStore::default();
    store.ensure_board(&root, "QA").unwrap(); store.release_enable(&root,true).unwrap();
    let mut task=store.create(&root,draft("Feature")).unwrap();
    task.column="done".into(); task=store.update(&root,task).unwrap();
    let rel=store.releases(&root).unwrap().releases[0].id.clone();
    store.release_edit(&root,"source",&rel,Some(&task.id),Some(SourceRef{repository:"app".into(),commit:"branch-c".into(),integrated_ref:Some("main-c".into())}),None).unwrap();
    store.release_prepare(&root,&rel,"attempt 1").unwrap();
    let result=store.release_outcome(&root,&rel,1,"build_failed","log: unresolved symbol").unwrap();
    let repair=result.attempts[0].repair_task_id.clone().expect("linked repair");
    assert!(store.load(&root).unwrap().tasks.iter().any(|t| t.id==repair && t.completed.is_none()));
    assert!(store.load(&root).unwrap().tasks.iter().any(|t| t.id==task.id && t.completed.is_some()));
    let retry=store.release_prepare(&root,&rel,"retry after fix").unwrap();
    assert_eq!(retry.attempts.len(),2);
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn direct_markdown_done_and_reopen_reconcile_without_duplication() {
    let root=root("sync");let store=FsTaskStore::default();store.ensure_board(&root,"QA").unwrap();store.release_enable(&root,true).unwrap();
    let task=store.create(&root,draft("Direct edit")).unwrap();
    let p=PathBuf::from(&task.path);let text=fs::read_to_string(&p).unwrap();
    fs::write(&p,text.replace("column: in-progress", "column: done")).unwrap();
    store.load(&root).unwrap(); assert_eq!(store.releases(&root).unwrap().releases[0].members.len(),1);
    let reopened=store.load(&root).unwrap().tasks.into_iter().find(|t| t.id==task.id).unwrap();
    let mut reopened=reopened; reopened.column="in-progress".into();store.update(&root,reopened).unwrap();
    assert!(store.releases(&root).unwrap().releases[0].members.is_empty());
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn gate_rejects_first_then_allows_once_with_exact_scope() {
    let root=root("gate");fs::create_dir_all(&root).unwrap();
    let err=challenge_agent_block(&root,"WT-0001","move","source mesh missing",None).unwrap_err();
    let TaskStoreError::BlockedChallenge{token,..}=err else {panic!("must self-review")};
    assert!(challenge_agent_block(&root,"WT-0002","move","source mesh missing",Some(&token)).is_err());
    assert!(challenge_agent_block(&root,"WT-0001","update","source mesh missing",Some(&token)).is_err());
    assert!(challenge_agent_block(&root,"WT-0001","move","different reason",Some(&token)).is_err());
    challenge_agent_block(&root,"WT-0001","move","source mesh missing",Some(&token)).unwrap();
    assert!(challenge_agent_block(&root,"WT-0001","move","source mesh missing",Some(&token)).is_err());
    assert!(challenge_agent_block(&root,"WT-0001","move","source mesh missing",Some("faked-token")).is_err());
    assert!(challenge_agent_block(&root,"WT-0001","move","",None).is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn simultaneous_workers_do_not_duplicate_release_members() {
    use std::sync::Arc;
    let root = Arc::new(root("parallel-done"));
    let store = FsTaskStore::default();
    store.ensure_board(&root, "Concurrency").unwrap();
    store.release_enable(&root, true).unwrap();
    let tasks: Vec<_> = (0..8).map(|n| store.create(&root, draft(&format!("Worker {n}"))).unwrap()).collect();
    let threads: Vec<_> = tasks.into_iter().map(|mut task| {
        let root = Arc::clone(&root);
        std::thread::spawn(move || {
            task.column = "done".into();
            FsTaskStore::default().update(&root, task).unwrap();
        })
    }).collect();
    for thread in threads { thread.join().unwrap(); }
    let summary = store.releases(&root).unwrap();
    assert_eq!(summary.releases.len(), 1);
    assert_eq!(summary.releases[0].members.len(), 8);
    let ids: std::collections::HashSet<_> = summary.releases[0].members.iter().map(|m| m.task_id.as_str()).collect();
    assert_eq!(ids.len(), 8);
    fs::remove_dir_all(root.as_ref()).unwrap();
}

#[test]
fn reopened_during_prepared_attempt_preserves_snapshot_but_not_verified_stage() {
    let root = root("reopened-attempt");
    let store = FsTaskStore::default();
    store.ensure_board(&root, "QA").unwrap();
    store.release_enable(&root, true).unwrap();
    let mut task = store.create(&root, draft("Member")).unwrap();
    task.column = "done".into(); task = store.update(&root, task).unwrap();
    let id = store.releases(&root).unwrap().releases[0].id.clone();
    store.release_edit(&root, "source", &id, Some(&task.id),
        Some(SourceRef {repository:"T3".into(), commit:"abc".into(), integrated_ref:Some("main-xyz".into())}), None).unwrap();
    store.release_prepare(&root, &id, "attempt before reopen").unwrap();
    let mut reopened = store.load(&root).unwrap().tasks.into_iter().find(|t| t.id == task.id).unwrap();
    reopened.column = "in-progress".into(); store.update(&root, reopened).unwrap();
    let outcome = store.release_outcome(&root, &id, 1, "deployed", "operator confirmation").unwrap();
    assert_eq!(outcome.attempts[0].members.len(),1);
    assert_eq!(outcome.members[0].stage,ReleaseStage::Unknown);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn legacy_blocked_migration_is_explicit_and_requires_evidence() {
    let root = root("blocked-migration");
    let store = FsTaskStore::default();
    store.ensure_board(&root, "QA").unwrap();
    let mut blocked = draft("Already source complete");
    blocked.column = "blocked".into();
    blocked.blocked_reason = Some("legacy source awaiting validation".into());
    let blocked = store.create_as(&root, blocked, task_core::MutationActor::Human).unwrap();
    assert!(store.release_migrate_blocked(&root, &blocked.id, "commit abc tested").is_err());
    store.release_enable(&root, true).unwrap();
    assert!(store.release_migrate_blocked(&root, &blocked.id, "").is_err());
    let completed = store.release_migrate_blocked(&root, &blocked.id, "commit abc / native smoke pass").unwrap();
    assert_eq!(completed.column, "done");
    assert!(completed.completed.is_some());
    let releases = store.releases(&root).unwrap();
    assert_eq!(releases.releases[0].members[0].task_id, blocked.id);
    assert!(releases.releases[0].members[0].evidence[0].contains("native smoke"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn manual_new_release_allows_movement_while_agents_keep_finishing() {
    let root = root("manual-movement");
    let store = FsTaskStore::default();
    store.ensure_board(&root,"QA").unwrap(); store.release_enable(&root,true).unwrap();
    let mut first = store.create(&root,draft("Older")).unwrap();
    first.column="done".into(); first=store.update(&root,first).unwrap();
    let previous=store.releases(&root).unwrap().releases[0].id.clone();
    let newer=store.release_new(&root).unwrap();
    assert_ne!(previous,newer.id);
    store.release_edit(&root,"move",&previous,Some(&first.id),None,Some(&newer.id)).unwrap();
    let releases=store.releases(&root).unwrap();
    assert_eq!(releases.releases[0].members.len(),0);
    assert_eq!(releases.releases[1].members.len(),1);
    let mut later=store.create(&root,draft("Later")).unwrap();
    later.column="done".into();store.update(&root,later).unwrap();
    let releases=store.releases(&root).unwrap();
    assert_eq!(releases.releases[1].members.len(),2);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn crash_after_failed_outcome_before_repair_creation_recovers_once() {
    let root=root("crash-before-repair");
    let store=FsTaskStore::default();
    store.ensure_board(&root,"QA").unwrap();
    store.release_enable(&root,true).unwrap();
    let mut implementation=store.create(&root,draft("Completed implementation")).unwrap();
    implementation.column="done".into();
    implementation=store.update(&root,implementation).unwrap();
    let release=store.releases(&root).unwrap().releases[0].id.clone();
    store.release_edit(&root,"source",&release,Some(&implementation.id),
        Some(SourceRef{repository:"T3".into(),commit:"isolated-xyz".into(),integrated_ref:Some("main-xyz".into())}),None).unwrap();
    store.release_prepare(&root,&release,"preflight manifest").unwrap();

    // Simulate a process termination immediately after persisting the failed
    // deployment outcome; no repair task exists yet, no backlink was written.
    let release_path=root.join("Releases").join(format!("{release}.md"));
    let content=fs::read_to_string(&release_path).unwrap();
    let content=content.replacen("status: prepared","status: build_failed",1)
        .replacen("evidence:\n- preflight manifest", "evidence:\n- preflight manifest\n- compile log unresolved symbol",1);
    fs::write(&release_path,content).unwrap();
    let recovered=store.load(&root).unwrap();
    let attempts=store.releases(&root).unwrap().releases[0].attempts.clone();
    assert_eq!(attempts[0].status,"build_failed");
    let repair=attempts[0].repair_task_id.as_ref().expect("repair backlink recovered");
    assert!(recovered.tasks.iter().any(|t| &t.id==repair && t.completed.is_none()));
    assert!(recovered.tasks.iter().any(|t| t.id==implementation.id && t.completed.is_some()));
    let before=store.load(&root).unwrap().tasks.len();
    store.load(&root).unwrap();
    assert_eq!(store.load(&root).unwrap().tasks.len(),before,"idempotent after restart");
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn crash_after_repair_task_creation_before_backlink_recovers_without_duplicate() {
    let root=root("crash-after-repair");
    let store=FsTaskStore::default();
    store.ensure_board(&root,"QA").unwrap();
    store.release_enable(&root,true).unwrap();
    let mut implementation=store.create(&root,draft("Completed implementation")).unwrap();
    implementation.column="done".into();
    implementation=store.update(&root,implementation).unwrap();
    let release=store.releases(&root).unwrap().releases[0].id.clone();
    store.release_edit(&root,"source",&release,Some(&implementation.id),
        Some(SourceRef{repository:"T3".into(),commit:"isolated-xyz".into(),integrated_ref:Some("main-xyz".into())}),None).unwrap();
    store.release_prepare(&root,&release,"preflight").unwrap();
    let outcome=store.release_outcome(&root,&release,1,"build_failed","build log").unwrap();
    let repair=outcome.attempts[0].repair_task_id.clone().unwrap();
    let original_count=store.load(&root).unwrap().tasks.len();
    let release_path=root.join("Releases").join(format!("{release}.md"));
    let content=fs::read_to_string(&release_path).unwrap();
    assert!(content.contains(&format!("repair_task_id: {repair}")));
    fs::write(&release_path,content.replace(&format!("repair_task_id: {repair}"),"repair_task_id: null")).unwrap();
    let recovered=store.load(&root).unwrap();
    assert_eq!(recovered.tasks.len(),original_count,"repair must be reused, not duplicated");
    assert_eq!(store.releases(&root).unwrap().releases[0].attempts[0].repair_task_id.as_deref(),Some(repair.as_str()));
    assert_eq!(store.releases(&root).unwrap().releases[0].attempts[0].status,"build_failed");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn direct_agent_core_api_requires_challenge_for_blocked_create_and_update() {
    use task_core::{MutationActor, TaskStoreError};
    let root = root("direct-api-gate");
    let store = FsTaskStore::default();
    store.ensure_board(&root, "QA").unwrap();
    let mut blocked = draft("Requires upstream dependency");
    blocked.column = "blocked".into();
    blocked.blocked_reason = Some("No permission to access required upstream dependency".into());
    let err = store.create(&root, blocked.clone()).unwrap_err();
    let token = match err { TaskStoreError::BlockedChallenge { token, .. } => token, _ => panic!("direct Agent API bypassed") };
    assert!(store.load(&root).unwrap().tasks.is_empty());
    let task = store.create_as_confirmed(&root, blocked, MutationActor::Agent, Some(&token)).unwrap();
    assert_eq!(task.column, "blocked");
    let err = store.create_as_confirmed(&root, draft("No challenge"), MutationActor::Agent, Some(&token)).unwrap_err();
    assert!(matches!(err, TaskStoreError::Invalid(_)));
    let mut working = store.create(&root, draft("Other task")).unwrap();
    working.attention.push(task_core::TaskAttention::Blocked);
    working.blocked_reason = Some("External compiler unavailable".into());
    let err = store.update(&root, working.clone()).unwrap_err();
    let token = match err { TaskStoreError::BlockedChallenge { token, .. } => token, _ => panic!("direct Agent update bypassed") };
    assert!(!store.load(&root).unwrap().tasks.iter().find(|t| t.id == working.id).unwrap().attention.contains(&task_core::TaskAttention::Blocked));
    let task = store.update_as_confirmed(&root, working, MutationActor::Agent, Some(&token), "update").unwrap();
    assert!(task.attention.contains(&task_core::TaskAttention::Blocked));
    fs::remove_dir_all(root).unwrap();
}
