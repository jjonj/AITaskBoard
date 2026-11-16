//! Global estimate of remaining tasks the T3 Keep Busy dispatcher can eventually pick up.
//! Dependency gates are evaluated transitively, including cross-board links.
use std::collections::{HashMap, HashSet};
use serde::Serialize;
use chrono::{DateTime, Duration, Local, NaiveDate};
use task_core::{ExecutionMode, FsTaskStore, QueueState, TaskAttention, TaskKind, TaskStore};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QueueWorkload {
    pub count: usize,
    pub ready_now: usize,
    pub dependency_waiting: usize,
    pub projects: Vec<ProjectWorkload>,
    pub pulse: WorkloadPulse,
    pub unavailable_boards: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProjectWorkload {
    pub name: String,
    pub count: usize,
    pub ready_now: usize,
    pub dependency_waiting: usize,
    pub open_tasks: usize,
    pub closed_all_time: usize,
    pub closed_7: usize,
    pub closed_prev_7: usize,
    pub closed_30: usize,
    pub created_7: usize,
    pub created_30: usize,
    /// Age is measured from task creation, not time spent in the queue.
    pub oldest_queue_task_days: Option<i64>,
}

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkloadPulse {
    pub open_tasks: usize,
    pub closed_all_time: usize,
    pub closed_7: usize,
    pub closed_prev_7: usize,
    pub closed_30: usize,
    pub created_7: usize,
    pub created_prev_7: usize,
    pub created_30: usize,
    pub daily: Vec<DailyFlow>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DailyFlow {
    pub day: String,
    pub closed: usize,
    pub created: usize,
}

fn task_day(raw: &str) -> Option<NaiveDate> {
    let date = raw.get(..10)?;
    NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()
}

fn completion_day(task: &task_core::Task) -> Option<NaiveDate> {
    if task.completed.is_none() { return None; }
    // RFC3339 timestamps are UTC. Attribute completed work to the user's
    // local day; legacy date-only records stay on their recorded day.
    task.completed_at.as_deref()
        .and_then(|raw| DateTime::parse_from_rfc3339(raw).ok())
        .map(|datetime| datetime.with_timezone(&Local).date_naive())
        .or_else(|| task.completed.as_deref().and_then(task_day))
}

fn recent(date: NaiveDate, today: NaiveDate, days: i64) -> bool {
    date <= today && date >= today - Duration::days(days - 1)
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
        let today = Local::now().date_naive();
        let mut pulse = WorkloadPulse {
            daily: (0..30).rev().map(|days_ago| DailyFlow {
                day: (today - Duration::days(days_ago)).to_string(),
                closed: 0,
                created: 0,
            }).collect(),
            ..WorkloadPulse::default()
        };
        let mut queued_creation_dates = HashMap::<String, Option<NaiveDate>>::new();
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
                open_tasks: 0, closed_all_time: 0, closed_7: 0, closed_prev_7: 0,
                closed_30: 0, created_7: 0, created_30: 0, oldest_queue_task_days: None,
            });
            for task in &snapshot.tasks {
                // Flow metrics measure task records, not queue events. Exclude
                // epics/reviews and do not count archived-but-unfinished as closed.
                if matches!(task.kind, TaskKind::Task | TaskKind::Recurring) {
                    let stats = &mut project_workloads[project_index];
                    if task.completed.is_some() {
                        stats.closed_all_time += 1;
                        pulse.closed_all_time += 1;
                    } else if !task.archived {
                        stats.open_tasks += 1;
                        pulse.open_tasks += 1;
                    }
                    if let Some(created) = task_day(&task.created) {
                        if recent(created, today, 30) {
                            stats.created_30 += 1;
                            pulse.created_30 += 1;
                            pulse.daily[(29 - (today - created).num_days()) as usize].created += 1;
                        }
                        if recent(created, today, 7) {
                            stats.created_7 += 1;
                            pulse.created_7 += 1;
                        } else if created >= today - Duration::days(13) && created < today - Duration::days(6) {
                            pulse.created_prev_7 += 1;
                        }
                    }
                    if let Some(closed) = completion_day(task) {
                        if recent(closed, today, 30) {
                            stats.closed_30 += 1;
                            pulse.closed_30 += 1;
                            pulse.daily[(29 - (today - closed).num_days()) as usize].closed += 1;
                        }
                        if recent(closed, today, 7) {
                            stats.closed_7 += 1;
                            pulse.closed_7 += 1;
                        } else if closed >= today - Duration::days(13) && closed < today - Duration::days(6) {
                            stats.closed_prev_7 += 1;
                            pulse.closed_prev_7 += 1;
                        }
                    }
                }
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
                if queued { queued_creation_dates.insert(id.clone(), task_day(&task.created)); }
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
                let stats = &mut project_workloads[item.project_index];
                stats.count += 1;
                // Age is only a rough prioritization signal, not queue wait time.
                if let Some(task) = queued_creation_dates.get(id).and_then(|day| *day) {
                    let age = (today - task).num_days().max(0);
                    stats.oldest_queue_task_days = Some(stats.oldest_queue_task_days.unwrap_or(0).max(age));
                }
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
            projects: project_workloads, pulse, unavailable_boards,
        })
    }).await.map_err(|error| format!("Global queue scan failed: {error}"))?
}
