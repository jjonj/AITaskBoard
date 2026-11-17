//! The automatic TaskBoard recovery loop: stale busy attempts use a rotating
//! FIFO; answered Inbox reviews stranded in Waiting are also resumed here.
use std::{collections::{HashMap, HashSet, VecDeque}, path::PathBuf, sync::{Mutex, OnceLock}, time::Duration};
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use task_core::{busy_recovery_idle_threshold, ExecutionIntent, ExecutionMode, FsTaskStore, QueueState, TaskKind, TaskStore};

const TICK: Duration = Duration::from_secs(20);
const BATCH: usize = 8;

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct Key { root: PathBuf, task: String, attempt: String }

#[derive(Clone)]
struct Busy {
    key: Key,
    state: Option<QueueState>,
    intent: ExecutionIntent,
    owner: Option<String>,
    last_change: Option<DateTime<Utc>>,
}

#[derive(Default)]
struct Fifo { items: VecDeque<Key>, first_seen: HashMap<Key, DateTime<Utc>> }
impl Fifo {
    fn sync(&mut self, entries: &[Busy]) {
        let live: HashSet<_> = entries.iter().map(|v| v.key.clone()).collect();
        self.items.retain(|k| live.contains(k));
        self.first_seen.retain(|k, _| live.contains(k));
        let existing: HashSet<_> = self.items.iter().cloned().collect();
        let mut added: Vec<_> = entries.iter().filter(|e| !existing.contains(&e.key)).collect();
        added.sort_by_key(|e| e.last_change);
        for entry in added {
            self.first_seen.insert(entry.key.clone(), Utc::now());
            self.items.push_back(entry.key.clone());
        }
    }
    fn next(&mut self, count: usize) -> Vec<Key> {
        let mut result = Vec::new();
        for _ in 0..self.items.len().min(count) {
            let key = self.items.pop_front().expect("nonempty FIFO");
            self.items.push_back(key.clone());
            result.push(key);
        }
        result
    }
}

fn parse_date(value: Option<&str>) -> Option<DateTime<Utc>> {
    value.and_then(|v| DateTime::parse_from_rfc3339(v).ok()).map(|v| v.to_utc())
}
fn busy(state: Option<QueueState>) -> bool {
    matches!(state, Some(QueueState::Reserved | QueueState::Starting |
        QueueState::Working | QueueState::RecoveryRequired))
}
fn scan() -> Result<(Vec<Busy>, Vec<(PathBuf, String)>), String> {
    let catalog = crate::projects::list_projects_sync()?;
    let store = FsTaskStore::default();
    let mut entries = Vec::new();
    let mut answered_review_waits = Vec::new();
    for project in catalog.projects {
        let root = PathBuf::from(project.path);
        if !root.join("_board.md").is_file() { continue; }
        let board = match store.load(&root) {
            Ok(board) => board,
            Err(err) if err.to_string().contains("board is busy") => continue,
            Err(err) => return Err(format!("{}: {err}", root.display())),
        };
        for task in board.tasks.iter().filter(|task| !task.archived && task.completed.is_none() &&
            matches!(task.kind, TaskKind::Task | TaskKind::Recurring) &&
            !board.config.columns.iter().any(|c| c.terminal && c.id == task.column)) {
            if task.kind == TaskKind::Task && task.column == "blocked" &&
                matches!(task.review_status.as_deref(), Some("resolved" | "awaiting_revision")) {
                answered_review_waits.push((root.clone(), task.id.clone()));
            }
            let intent = store.execution_intent(&root, &task.id).map_err(|e| e.to_string())?;
            let state = (intent.mode == ExecutionMode::Queue).then_some(intent.queue_state).flatten();
            let pending_start = intent.start_now_request.is_some() && intent.start_now_receipt.is_none();
            if !busy(state) && task.claimed_by.is_none() && task.column != "in-progress" && !pending_start {
                continue;
            }
            let last_change = [parse_date(intent.queue_state_since.as_deref()),
                parse_date(task.observed_modified_at.as_deref())].into_iter().flatten().max();
            let attempt = format!("{}:{}:{}:{}",
                intent.queue_request.as_deref().unwrap_or(""),
                intent.queue_reservation_id.as_deref().unwrap_or(""),
                intent.start_now_request.as_deref().unwrap_or(""),
                task.claimed_by.as_deref().unwrap_or(""));
            entries.push(Busy {
                key: Key {root:root.clone(), task:task.id.clone(), attempt},
                state, intent, owner:task.claimed_by.clone(), last_change,
            });
        }
    }
    Ok((entries, answered_review_waits))
}

fn t3_activity(row: &Busy, sessions: &[Value]) -> Option<i64> {
    let owner_id = row.owner.as_deref().and_then(|s|
        s.strip_prefix("t3:").or_else(|| s.strip_prefix("t3-browser:")));
    sessions.iter().filter(|s| {
        let field = |name: &str| s.get(name).and_then(Value::as_str);
        field("spawnRequestId") == row.intent.queue_reservation_id.as_deref() &&
            row.intent.queue_reservation_id.is_some() ||
        field("id") == row.intent.queue_session_id.as_deref() &&
            row.intent.queue_session_id.is_some() ||
        field("id") == owner_id && owner_id.is_some()
    }).filter_map(|s| s.get("lastWorkingAt").and_then(Value::as_i64))
        .filter(|ms| *ms > 0).max()
}
fn should_recover(row: &Busy, first_seen: Option<DateTime<Utc>>,
    session_activity_ms: Option<i64>, now: DateTime<Utc>) -> bool {
    let observed = session_activity_ms.and_then(DateTime::<Utc>::from_timestamp_millis)
        .filter(|time| *time <= now);
    // The in-memory FIFO's first observation is only a fallback. Restarting
    // TaskBoard must never reset the durable inactivity clock.
    let last = [row.last_change, observed].into_iter().flatten().max().or(first_seen);
    last.is_some_and(|last| now.signed_duration_since(last) >= busy_recovery_idle_threshold(row.state))
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RecoveryStatus {
    active: bool, queued: usize, last_checked_at: Option<String>,
    last_error: Option<String>, last_action: Option<String>,
}
static STATUS: OnceLock<Mutex<RecoveryStatus>> = OnceLock::new();
fn status() -> &'static Mutex<RecoveryStatus> {
    STATUS.get_or_init(|| Mutex::new(RecoveryStatus::default()))
}
pub(crate) fn current_status() -> RecoveryStatus {
    status().lock().unwrap_or_else(|e| e.into_inner()).clone()
}
fn tick(fifo: &mut Fifo) -> Result<(), String> {
    let (entries, answered_review_waits) = scan()?;
    fifo.sync(&entries);
    let due = fifo.next(BATCH);
    let by_key: HashMap<_, _> = entries.into_iter().map(|entry| (entry.key.clone(), entry)).collect();
    {
        let mut state = status().lock().unwrap_or_else(|e| e.into_inner());
        state.last_checked_at = Some(Utc::now().to_rfc3339());
        state.queued = fifo.items.len();
        state.last_error = None;
    }
    // This recovery is independent of busy/Starting timeouts. A review
    // verdict is already authoritative; one repair per tick bounds writes.
    let store = FsTaskStore::default();
    for (root, id) in answered_review_waits.into_iter().take(BATCH) {
        match store.recover_answered_review_wait(&root, &id) {
            Ok(true) => {
                status().lock().unwrap_or_else(|e| e.into_inner()).last_action =
                    Some(format!("{id}: answered review resumed from Waiting"));
                break;
            }
            Ok(false) => {},
            Err(err) => {
                status().lock().unwrap_or_else(|e| e.into_inner()).last_error =
                    Some(format!("{id}: review wait recovery: {err}"));
            }
        }
    }
    if due.is_empty() { return Ok(()) }
    let sessions = crate::agent_notices::shared_t3_inventory()
        .and_then(|data| {
            let sessions = data.get("sessions")?.as_array()?;
            if data.get("incomplete").and_then(Value::as_bool) == Some(true) ||
                data.get("hasMore").and_then(Value::as_bool) == Some(true) { return None; }
            Some(sessions.clone())
        }).unwrap_or_default();
    let now = Utc::now();
    for key in due {
        let Some(row) = by_key.get(&key) else { continue };
        // Old data only; this doesn't create another kind of recovery.
        if row.state == Some(QueueState::RecoveryRequired) {
            let Some(request) = row.intent.queue_request.as_deref() else { continue };
            let Some(reservation) = row.intent.queue_reservation_id.as_deref() else { continue };
            match store.normalize_legacy_queue_recovery(&key.root, &key.task, request, reservation) {
                Ok(_) => { status().lock().unwrap_or_else(|e| e.into_inner()).last_action =
                    Some(format!("{}: legacy state cleared",key.task)); break; }
                Err(err) => {
                    status().lock().unwrap_or_else(|e| e.into_inner()).last_error =
                        Some(format!("{}: {err}",key.task));
                }
            }
            continue;
        }
        let agent_activity = t3_activity(row, &sessions);
        if !should_recover(row, fifo.first_seen.get(&key).copied(), agent_activity, now) {
            continue;
        }
        match store.recover_stale_busy_task(&key.root, &key.task, row.owner.as_deref(),
            row.intent.queue_request.as_deref(), row.intent.queue_reservation_id.as_deref(),
            row.intent.queue_state, row.intent.start_now_request.as_deref(), agent_activity) {
            Ok(_) => {
                status().lock().unwrap_or_else(|e| e.into_inner()).last_action =
                    Some(format!("{}: recovered after {} idle",key.task,
                        if row.state == Some(QueueState::Starting) { "10 minutes" } else { "three hours" }));
                break; // Reread authoritative board state next tick.
            }
            Err(err) => {
                status().lock().unwrap_or_else(|e| e.into_inner()).last_error =
                    Some(format!("{}: {err}",key.task));
            }
        }
    }
    Ok(())
}
pub(crate) fn start() {
    std::thread::spawn(|| {
        let mut fifo = Fifo::default();
        status().lock().unwrap_or_else(|e| e.into_inner()).active = true;
        loop {
            if let Err(error) = tick(&mut fifo) {
                status().lock().unwrap_or_else(|e| e.into_inner()).last_error = Some(error);
            }
            std::thread::sleep(TICK);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn work(id: &str, state: QueueState, minutes_old: i64) -> Busy {
        Busy {key:Key{root:PathBuf::from("C:/board"),task:id.into(),attempt:id.into()},
            state:Some(state),owner:None,
            last_change:Some(Utc::now()-chrono::Duration::minutes(minutes_old)),
            intent:ExecutionIntent {
                mode:ExecutionMode::Queue,queue_state:Some(state),
                queue_request:Some(uuid::Uuid::new_v4().to_string()),
                queue_reservation_id:Some(uuid::Uuid::new_v4().to_string()),
                queue_state_since:None,queue_attempt:None,queue_receipt:None,
                queue_retired_receipt:None,queue_reservation_expires_at:None,queue_session_id:None,
                queue_error:None,start_now_request:None,start_now_receipt:None,revision:String::new()
            }}
    }
    #[test]
    fn fifo_rotates_all_busy_claims_and_removes_only_finished() {
        let mut fifo=Fifo::default();
        let a=work("reserved",QueueState::Reserved,300);
        let b=work("working",QueueState::Working,300);
        let c=work("incident",QueueState::RecoveryRequired,300);
        let mut d=work("claimed_queued",QueueState::Queued,300);
        d.owner=Some("t3:old".into());
        fifo.sync(&[a.clone(),b.clone(),c.clone(),d.clone()]);
        assert_eq!(fifo.next(2),vec![a.key.clone(),b.key.clone()]);
        assert_eq!(fifo.next(2),vec![c.key.clone(),d.key.clone()]);
        fifo.sync(&[b.clone(),c.clone(),d.clone()]);
        assert_eq!(fifo.next(3),vec![b.key.clone(),c.key.clone(),d.key.clone()]);
    }
    #[test]
    fn starting_uses_ten_minutes_while_other_busy_work_uses_three_hours() {
        let now=Utc::now();
        let starting=work("starting",QueueState::Starting,11);
        assert!(should_recover(&starting,Some(now),None,now));
        assert!(!should_recover(&work("recent",QueueState::Starting,9),None,None,now));
        assert!(!should_recover(&starting,None,
            Some((now-chrono::Duration::minutes(5)).timestamp_millis()),now));
        for state in [QueueState::Reserved,QueueState::Working,QueueState::Queued] {
            let row=work("any",state,181);
            assert!(should_recover(&row,Some(now),None,now),
                "restart does not reset an already expired durable timestamp");
            assert!(!should_recover(&work("recent",state,11),None,None,now));
            assert!(!should_recover(&row,None,
                Some((now-chrono::Duration::minutes(5)).timestamp_millis()),now));
        }
    }
    #[test]
    fn exact_claimant_session_counts_as_activity_even_with_queued_state() {
        let mut row=work("mismatch",QueueState::Queued,220);
        row.owner=Some("t3:agent-123".into());
        let sessions=vec![json!({"id":"agent-123","lastWorkingAt":
            (Utc::now()-chrono::Duration::minutes(10)).timestamp_millis()})];
        let time=t3_activity(&row,&sessions);
        assert!(time.is_some());
        assert!(!should_recover(&row,row.last_change,time,Utc::now()));
    }
}
