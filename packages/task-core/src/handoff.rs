//! Read-only provenance packet supplied on task show and worker handoff.
use crate::Task;
use serde::Serialize;
use std::fs;

#[derive(Debug, Clone, Serialize)]
pub struct HandoffPacket {
    pub origin_sessions: Vec<String>,
    pub latest_claim: Option<String>,
    pub user_decisions: Vec<String>,
    pub progress: Vec<String>,
    pub artifact_references: Vec<String>,
    pub commit_references: Vec<String>,
    pub uncommitted_references: Vec<String>,
    pub latest_rejection: Option<String>,
    pub instructions: String,
}

pub fn handoff_bundle(task: &Task) -> HandoffPacket {
    let text = fs::read_to_string(&task.path).unwrap_or_default();
    let yaml = text.strip_prefix("---").and_then(|v| v.split_once("---"))
        .map(|(header, _)|header).unwrap_or("");
    let origin_sessions = serde_yaml::from_str::<serde_yaml::Value>(yaml).ok()
        .and_then(|v|v.get("originating_t3_sessions").cloned())
        .and_then(|v|serde_yaml::from_value::<Vec<String>>(v).ok())
        .unwrap_or_default();
    let user_decisions = task.question.as_ref().and_then(|question|question.answer.as_ref().map(|answer|
            format!("User answered {}: {}", question.prompt, answer)))
        .map(|answer|vec![answer])
        .unwrap_or_default().into_iter().chain(task.user_notes.lines().filter(|line|!line.trim().is_empty())
        .take(8).map(str::to_owned)).collect::<Vec<_>>();
    // Read actual on-task evidence. Never claim a clean checkout, a commit, or
    // passing tests on behalf of an earlier worker.
    let lines = [task.description.as_str(),task.ai_notes.as_str(),task.notes.as_str()]
        .into_iter().flat_map(str::lines).map(str::trim)
        .filter(|line|!line.is_empty()).collect::<Vec<_>>();
    let relevant = |words: &[&str], limit:usize| -> Vec<String> {
        lines.iter().filter(|line| {
            let lower=line.to_lowercase(); words.iter().any(|word|lower.contains(word))
        }).take(limit).map(|line|line.chars().take(350).collect()).collect()
    };
    let progress=lines.iter().rev().take(14).rev().map(|line|
        line.chars().take(280).collect()).collect();
    let artifact_references=relevant(&["artifact", "evidence", ".png", ".jpg", ".mp4", "screenshot", "docs/"],12);
    let commit_references=relevant(&["commit", "sha:", "revision"],10);
    let uncommitted_references=relevant(&["uncommitted", "working tree", "stash", "wip", "not committed"],8);
    let latest_rejection=lines.iter().rev().find(|line| {
        let l=line.to_lowercase();l.contains("rejected") || l.contains("needs changes") || l.contains("user rejection")
    }).map(|line|line.chars().take(450).collect());
    HandoffPacket {origin_sessions,latest_claim:task.claimed_by.clone(),user_decisions,
        progress,artifact_references,commit_references,uncommitted_references,
        latest_rejection,
        instructions:format!("Continue {} using current TaskBoard revision. FIRST implement any USER DECISION in user_decisions / AI Notes as the next requirement. Do not request approval again for an answered question. Inspect previous work and working tree before edits; missing evidence is unknown, not completed. {}",task.id,
            if task.review_mode == crate::ReviewMode::Session {
                "REVIEW MODE: Session (persistent GMA). Keep the user in the linked T3 session, request persistent GMA attention for feedback, iterate until the user is satisfied, and DO NOT submit TaskBoard Inbox questions, verification or visual reviews. Finish only when the user accepts the work in-session."
            } else {
                "REVIEW MODE: Inbox. Submit required review/verification through the original TaskBoard card (not GMA session review)."
            })}
}
