//! Global estimate of remaining tasks the T3 Keep Busy dispatcher can eventually pick up.
//! Dependency gates are evaluated transitively, including cross-board links.
use std::{collections::{HashMap, HashSet}, io::Write, path::PathBuf, process::{Command, Stdio}};
use serde::Serialize;
use serde_json::{json, Value};
use chrono::{DateTime, Duration, Local, NaiveDate, Utc};
use crate::projects::ProjectPriority;
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
    pub next_pickups: Vec<NextPickup>,
    pub pickup_note: Option<String>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NextPickup {
    pub board_path: String,
    pub project: String,
    pub task_id: String,
    pub title: String,
    pub urgent: bool,
    pub project_priority: ProjectPriority,
}

struct PickupCandidate {
    details: NextPickup,
    created: String,
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

fn project_rank(priority: ProjectPriority) -> u8 {
    match priority { ProjectPriority::High => 0, ProjectPriority::Normal => 1, ProjectPriority::Low => 2 }
}

fn numeric_id(id: &str) -> u64 {
    id.rsplit_once('-').and_then(|(_, suffix)| suffix.parse().ok()).unwrap_or(u64::MAX)
}

// T3 treats both today and yesterday's date-only cards as possibly created in the last 24 hours.
fn recent_card(created: &str, today: NaiveDate, now: DateTime<Utc>) -> bool {
    if let Ok(day) = NaiveDate::parse_from_str(created, "%Y-%m-%d") {
        return day <= today && day >= today - Duration::days(1);
    }
    DateTime::parse_from_rfc3339(created).is_ok_and(|date|
        date.with_timezone(&Utc) <= now && date.with_timezone(&Utc) >= now - Duration::hours(24))
}

fn t3_read(input: Value) -> Result<Value, String> {
    let script = r"D:\SSDProjects\Tools\t3code\scripts\chatgpt-sessions.mjs";
    if !std::path::Path::new(script).is_file() { return Err("T3 session bridge unavailable".into()); }
    let home = std::env::var_os("T3CODE_HOME").map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(|h| PathBuf::from(h).join(".t3")))
        .ok_or("T3 home unavailable")?;
    let mut cmd = Command::new("node.exe");
    cmd.env("T3CODE_HOME", home).arg(script)
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
    #[cfg(windows)] {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let mut child = cmd.spawn().map_err(|e| format!("T3 session bridge unavailable: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(input.to_string().as_bytes()).map_err(|e| e.to_string())?;
    }
    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    if !output.status.success() { return Err("T3 session inventory unavailable".into()); }
    serde_json::from_slice(&output.stdout).map_err(|_| "Unreadable T3 session inventory".into())
}

// The same load T3 Keep Busy uses: all green sessions, including manual workers,
// mapped by exact task binding first and unique project name second.
fn working_load(projects: &[(String, String, String)]) -> Result<HashMap<String, usize>, String> {
    let capacity = t3_read(json!({"action":"caffy","op":"capacity"}))?;
    let inventory = t3_read(json!({"action":"list","which":"all","includeHidden":true}))?;
    let ids = capacity["workingIds"].as_array().ok_or("T3 working session IDs unavailable")?;
    let sessions = inventory["sessions"].as_array().ok_or("T3 session inventory missing")?;
    let mut names: HashMap<String, Option<String>> = HashMap::new();
    let roots: HashSet<String> = projects.iter().map(|(root, _, _)| root.clone()).collect();
    for (root, name, id) in projects {
        for label in [name, id] {
            let label = label.trim().to_lowercase();
            if label.is_empty() { continue; }
            match names.get(&label) {
                Some(Some(previous)) if previous != root => { names.insert(label, None); }
                None => { names.insert(label, Some(root.clone())); }
                _ => {}
            }
        }
    }
    let mut load = HashMap::new();
    for working in ids {
        let Some(id) = working.as_str() else { continue; };
        let Some(session) = sessions.iter().find(|s| s["id"].as_str() == Some(id)) else { continue; };
        let bound = session["binding"]["taskRef"]["board"].as_str()
            .map(|root| key(root, "")).map(|root| root.trim_end_matches('#').to_string());
        let owner = bound.filter(|root| roots.contains(root)).or_else(||
            session["project"].as_str().and_then(|name| names.get(&name.trim().to_lowercase()).cloned().flatten()));
        if let Some(root) = owner { *load.entry(root).or_insert(0) += 1; }
    }
    Ok(load)
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
        let hidden: HashSet<String> = projects.hidden_paths.iter()
            .map(|path| key(path, "").trim_end_matches('#').to_string()).collect();
        let mut known_projects = Vec::<(String, String, String)>::new();
        let mut candidates = HashMap::<String, PickupCandidate>::new();
        let mut startup_load = HashMap::<String, usize>::new();
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
            let project_name = project.name.clone();
            let project_priority = project.priority;
            let project_id = project.id.clone();
            let board_key = key(&root, "").trim_end_matches('#').to_string();
            if !seen.insert(root.replace('\\', "/").trim_end_matches('/').to_lowercase()) { continue; }
            let snapshot = match store.load(std::path::Path::new(&root)) {
                Ok(board) => board,
                Err(_) => { unavailable_boards.push(project.name); continue; }
            };
            known_projects.push((board_key.clone(), project_name.clone(), project_id));
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
                if intent.queue_reservation_id.is_some() && match phase {
                    QueueState::Reserved => intent.queue_reservation_expires_at.as_deref()
                        .and_then(|stamp| DateTime::parse_from_rfc3339(stamp).ok())
                        .is_some_and(|until| until.with_timezone(&Utc) > Utc::now()),
                    QueueState::Starting => intent.queue_state_since.as_deref()
                        .and_then(|stamp| DateTime::parse_from_rfc3339(stamp).ok())
                        .is_some_and(|since| Utc::now() - since.with_timezone(&Utc) < Duration::minutes(5)),
                    _ => false,
                } {
                    *startup_load.entry(board_key.clone()).or_insert(0) += 1;
                }
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
                if queued {
                    queued_creation_dates.insert(id.clone(), task_day(&task.created));
                    if !hidden.contains(&board_key) && uuid::Uuid::parse_str(intent.queue_request.as_deref().unwrap_or("")).is_ok()
                        && task_core::queue_task_ready(&snapshot, task) {
                        candidates.insert(id.clone(), PickupCandidate {
                            details: NextPickup { board_path: root.clone(), project: project_name.clone(),
                                task_id: task.id.clone(), title: task.title.clone(),
                                urgent: task.attention.contains(&TaskAttention::Priority), project_priority },
                            created: task.created.clone(),
                        });
                    }
                }
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
        let (mut board_load, pickup_note) = match working_load(&known_projects) {
            Ok(load) => (load, None),
            Err(reason) => (HashMap::new(), Some(format!("T3 live workload unavailable ({reason}); project balancing is approximate."))),
        };
        for (board, count) in startup_load {
            *board_load.entry(board).or_insert(0) += count;
        }
        let now = Utc::now();
        let mut next_pickups = candidates.into_iter()
            .filter(|(id, _)| all.get(id).is_some_and(|work| work.queued) &&
                all.get(id).is_some_and(|work| work.dependencies.iter().all(|dep|
                    all.get(dep).is_some_and(|item| item.done))))
            .map(|(_, candidate)| candidate).collect::<Vec<_>>();
        next_pickups.sort_by(|a, b| {
            let board_a = key(&a.details.board_path, "").trim_end_matches('#').to_string();
            let board_b = key(&b.details.board_path, "").trim_end_matches('#').to_string();
            b.details.urgent.cmp(&a.details.urgent)
                .then_with(|| board_load.get(&board_a).unwrap_or(&0).cmp(board_load.get(&board_b).unwrap_or(&0)))
                .then_with(|| project_rank(a.details.project_priority).cmp(&project_rank(b.details.project_priority)))
                .then_with(|| recent_card(&b.created, today, now).cmp(&recent_card(&a.created, today, now)))
                .then_with(|| numeric_id(&a.details.task_id).cmp(&numeric_id(&b.details.task_id)))
                .then_with(|| board_a.cmp(&board_b))
                .then_with(|| a.details.task_id.cmp(&b.details.task_id))
        });
        let next_pickups = next_pickups.into_iter().take(10).map(|item| item.details).collect();
        project_workloads.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.name.cmp(&b.name)));
        Ok(QueueWorkload {
            count, ready_now, dependency_waiting: count - ready_now,
            projects: project_workloads, pulse, unavailable_boards, next_pickups, pickup_note,
        })
    }).await.map_err(|error| format!("Global queue scan failed: {error}"))?
}
