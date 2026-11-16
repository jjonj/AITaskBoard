//! Global estimate of remaining tasks the T3 Keep Busy dispatcher can eventually pick up.
//! Dependency gates are evaluated transitively, including cross-board links.
use std::collections::{HashMap, HashSet};
use serde::Serialize;
use task_core::{ExecutionMode, FsTaskStore, QueueState, TaskAttention, TaskKind, TaskStore};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QueueWorkload {
    pub count: usize,
    pub ready_now: usize,
    pub dependency_waiting: usize,
    pub projects: Vec<ProjectWorkload>,
    pub unavailable_boards: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProjectWorkload {
    pub name: String,
    pub count: usize,
    pub ready_now: usize,
    pub dependency_waiting: usize,
}

struct WorkItem {
    done: bool,
    queued: bool,
    in_flight: bool,
    dependencies: Vec<String>,
    project_index: usize,
}

fn key(root: &str, id: &str) -> String {
    format!("{}#{id}", root.replace('\\', "/").trim_end_matches('/').to_lowercase())
}

fn prerequisite_key(root: &str, id: &str) -> String {
    match id.rsplit_once('#') {
        Some((other_root, other_id)) => key(other_root, other_id),
        None => key(root, id),
    }
}

/// All prerequisites must either be done, already running in T3, or themselves
/// be queued behind a resolvable chain. Missing dependencies and cycles fail closed.
fn eventually_executable(
    id: &str, all: &HashMap<String, WorkItem>, visiting: &mut HashSet<String>,
    memo: &mut HashMap<String, bool>,
) -> bool {
    if let Some(value) = memo.get(id) { return *value; }
    if !visiting.insert(id.to_owned()) { return false; }
    let result = all.get(id).is_some_and(|task| {
        task.done || task.in_flight ||
            (task.queued && task.dependencies.iter().all(|dependency|
                eventually_executable(dependency, all, visiting, memo)))
    });
    visiting.remove(id);
    memo.insert(id.to_owned(), result);
    result
}

pub(crate) async fn read() -> Result<QueueWorkload, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let projects = super::projects::list_projects_sync()?;
        let store = FsTaskStore::default();
        let mut all = HashMap::<String, WorkItem>::new();
        let mut unavailable_boards = Vec::new();
        let mut project_workloads = Vec::<ProjectWorkload>::new();
        let mut seen = HashSet::new();

        for project in projects.projects {
            let root = project.path;
            if !seen.insert(root.replace('\\', "/").trim_end_matches('/').to_lowercase()) { continue; }
            let snapshot = match store.load(std::path::Path::new(&root)) {
                Ok(board) => board,
                Err(_) => { unavailable_boards.push(project.name); continue; }
            };
            let project_index = project_workloads.len();
            project_workloads.push(ProjectWorkload {
                name: project.name, count: 0, ready_now: 0, dependency_waiting: 0,
            });
            for task in &snapshot.tasks {
                let id = key(&root, &task.id);
                let done = task.archived || task.completed.is_some();
                if done {
                    all.insert(id, WorkItem { done: true, queued: false, in_flight: false, dependencies: vec![], project_index });
                    continue;
                }
                if !matches!(task.kind, TaskKind::Task | TaskKind::Recurring) { continue; }
                let intent = match store.execution_intent(std::path::Path::new(&root), &task.id) {
                    Ok(value) => value,
                    Err(_) => continue,
                };
                let phase = intent.queue_state.unwrap_or(QueueState::Queued);
                let legacy_pending = (intent.queue_attempt.is_some() || intent.queue_receipt.is_some())
                    && intent.queue_retired_receipt.as_deref() != intent.queue_request.as_ref()
                        .map(|request| format!("qitem-taskboard-{request}")).as_deref();
                let valid_queue = intent.mode == ExecutionMode::Queue &&
                    intent.queue_request.is_some() && !legacy_pending &&
                    !matches!(phase, QueueState::Completed | QueueState::RecoveryRequired);
                let human_waiting = task.attention.contains(&TaskAttention::NeedsUser) ||
                    task.review_status.as_deref() == Some("open") ||
                    (task.attention.contains(&TaskAttention::Question) &&
                     task.question.as_ref().is_some_and(|q| q.answer.is_none() && !q.prompt.trim().is_empty()));
                let technical_block = task_core::is_task_blocked(task);
                let in_flight = valid_queue && !human_waiting && !technical_block &&
                    (task.claimed_by.is_some() || matches!(phase, QueueState::Working | QueueState::Starting | QueueState::Reserved));
                let eligible_column = snapshot.config.columns.iter().any(|column|
                    column.id == task.column && column.ai_ready && !column.advisory &&
                    !column.terminal && column.id != task_core::DRAFT_COLUMN_ID);
                let queued = valid_queue && task.claimed_by.is_none() &&
                    phase == QueueState::Queued && eligible_column &&
                    !human_waiting && !technical_block && task_core::recurring_due(task);
                let dependencies = task.depends_on.iter().map(|dep| prerequisite_key(&root, dep)).collect();
                all.insert(id, WorkItem { done: false, queued, in_flight, dependencies, project_index });
            }
        }

        let mut memo = HashMap::new();
        let mut count = 0;
        let mut ready_now = 0;
        for (id, item) in &all {
            if !item.queued { continue; }
            let mut visiting = HashSet::new();
            if eventually_executable(id, &all, &mut visiting, &mut memo) {
                count += 1;
                project_workloads[item.project_index].count += 1;
                if item.dependencies.iter().all(|dependency| all.get(dependency).is_some_and(|dep| dep.done)) {
                    ready_now += 1;
                    project_workloads[item.project_index].ready_now += 1;
                }
            }
        }
        for project in &mut project_workloads {
            project.dependency_waiting = project.count - project.ready_now;
        }
        project_workloads.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.name.cmp(&b.name)));
        Ok(QueueWorkload {
            count, ready_now, dependency_waiting: count - ready_now,
            projects: project_workloads, unavailable_boards,
        })
    }).await.map_err(|error| format!("Global queue scan failed: {error}"))?
}
