//! TaskBoard-owned busy-work FIFO. TaskBoard files are the durable source of
//! membership; the queue holds ordering only and is reconstructed after restart.
use std::{collections::{HashMap, HashSet, VecDeque}, path::PathBuf, sync::{Mutex, OnceLock}, time::Duration};
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use task_core::{ExecutionIntent, ExecutionMode, FsTaskStore, QueueState, TaskStore};

const INTERVAL: Duration = Duration::from_secs(20);
const BATCH: usize = 8;

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct Key { root: PathBuf, task: String, attempt: String }

#[derive(Clone)]
struct Busy {
    key: Key,
    state: Option<QueueState>,
    intent: ExecutionIntent,
    owner: Option<String>,
    paused_by_user: bool,
    since: Option<DateTime<Utc>>,
}

#[derive(Default)]
struct Fifo { items: VecDeque<Key>, first_seen: HashMap<Key, DateTime<Utc>> }
impl Fifo {
    fn sync(&mut self, active: &mut [Busy]) {
        let now = Utc::now();
        for row in active.iter_mut() {
            let first = self.first_seen.entry(row.key.clone()).or_insert(now);
            row.since = row.since.or(Some(*first));
        }
        let keys: HashSet<_> = active.iter().map(|entry| entry.key.clone()).collect();
        self.items.retain(|key| keys.contains(key));
        self.first_seen.retain(|key, _| keys.contains(key));
        let known: HashSet<_> = self.items.iter().cloned().collect();
        let mut additions: Vec<_> = active.iter().filter(|row| !known.contains(&row.key)).collect();
        additions.sort_by_key(|row| row.since);
        self.items.extend(additions.into_iter().map(|row| row.key.clone()));
    }
    fn cycle(&mut self, count: usize) -> Vec<Key> {
        let mut result = Vec::new();
        for _ in 0..self.items.len().min(count) {
            let Some(next) = self.items.pop_front() else { break };
            self.items.push_back(next.clone());
            result.push(next);
        }
        result
    }
    fn len(&self) -> usize { self.items.len() }
}

fn timestamp(raw: Option<&str>) -> Option<DateTime<Utc>> {
    raw.and_then(|s| DateTime::parse_from_rfc3339(s).ok()).map(|v| v.to_utc())
}
fn age_minutes(row: &Busy, now: DateTime<Utc>) -> Option<i64> {
    row.since.map(|v| (now - v).num_minutes())
}
fn active_state(state: Option<QueueState>) -> bool {
    matches!(state, Some(QueueState::Reserved | QueueState::Starting |
        QueueState::Working | QueueState::RecoveryRequired))
}
fn normalize(path: &str) -> String { path.replace('/', "\\").trim_end_matches('\\').to_ascii_lowercase() }

fn scan() -> Result<Vec<Busy>, String> {
    let catalog = crate::projects::list_projects_sync()?;
    let store = FsTaskStore::default();
    let mut active = Vec::new();
    for project in catalog.projects {
        let root = PathBuf::from(project.path);
        if !root.join("_board.md").is_file() { continue; }
        let board = store.load(&root).map_err(|error| format!("{}: {error}", root.display()))?;
        for task in board.tasks.iter().filter(|task| !task.archived && task.completed.is_none() &&
            !board.config.columns.iter().any(|column| column.terminal && column.id == task.column)) {
            let intent = store.execution_intent(&root, &task.id).map_err(|error| error.to_string())?;
            let state = if intent.mode == ExecutionMode::Queue { intent.queue_state } else { None };
            let pending_manual = intent.start_now_request.is_some() && intent.start_now_receipt.is_none();
            if !active_state(state) && task.claimed_by.is_none() && !pending_manual { continue; }
            let attempt = if active_state(state) {
                format!("queue:{}:{}", intent.queue_request.as_deref().unwrap_or(""),
                    intent.queue_reservation_id.as_deref().unwrap_or(""))
            } else {
                format!("manual:{}", task.claimed_by.as_deref()
                    .or(intent.start_now_request.as_deref()).unwrap_or(""))
            };
            let since = timestamp(intent.queue_state_since.as_deref());
            active.push(Busy {
                key: Key { root: root.clone(), task: task.id.clone(), attempt },
                state, intent, owner: task.claimed_by.clone(), since,
                paused_by_user: task.question.as_ref().is_some_and(|q|
                    q.request_type.as_deref() == Some("queue_recovery") &&
                    q.answer.as_deref() == Some("Keep paused")) || task.column == "blocked",
            });
        }
    }
    Ok(active)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Repair { None, Expire, Abandon, Working, RetryIncident, RequeueReleased, ReclaimClosed, Incident }

fn field<'a>(session: &'a Value, key: &str) -> Option<&'a str> {
    session.get(key).and_then(Value::as_str)
}
fn closed(session: &Value) -> bool { session.get("closedAt").is_some_and(|v| !v.is_null()) }
fn no_response(session: &Value) -> bool {
    session.get("totalTurns").and_then(Value::as_u64).unwrap_or(0) == 0 &&
    session.get("lastRole").is_none_or(Value::is_null) &&
    field(session, "conversationId").is_none() &&
    session.pointer("/bootstrapIdentity/conversationId").is_none_or(Value::is_null)
}
fn verified_reply(session: &Value, task: &Busy) -> bool {
    let conversation = field(session, "conversationId");
    let expected_board = task.key.root.to_string_lossy();
    conversation.is_some() &&
        field(session, "lastRole") == Some("assistant") &&
        session.get("totalTurns").and_then(Value::as_u64).is_some_and(|n| n > 0) &&
        session.pointer("/bootstrapIdentity/state").and_then(Value::as_str) == Some("registered") &&
        session.pointer("/bootstrapIdentity/conversationId").and_then(Value::as_str) == conversation &&
        session.pointer("/binding/taskRef/id").and_then(Value::as_str) == Some(task.key.task.as_str()) &&
        session.pointer("/binding/taskRef/board").and_then(Value::as_str)
            .is_some_and(|board| normalize(board) == normalize(&expected_board))
}
fn session_owner<'a>(row: &Busy, sessions: &'a [Value]) -> Vec<&'a Value> {
    let Some(request) = row.intent.queue_reservation_id.as_deref() else { return vec![] };
    sessions.iter().filter(|session| field(session, "spawnRequestId") == Some(request)).collect()
}
fn decide(row: &Busy, matches: &[&Value], now: DateTime<Utc>) -> Repair {
    let Some(age) = age_minutes(row, now) else { return Repair::None };
    if active_state(row.state) && row.state != Some(QueueState::RecoveryRequired) &&
        (row.intent.queue_request.as_deref().is_none_or(|id| uuid::Uuid::parse_str(id).is_err()) ||
         row.intent.queue_reservation_id.as_deref().is_none_or(|id| uuid::Uuid::parse_str(id).is_err())) {
        return if age >= 15 { Repair::Incident } else { Repair::None };
    }
    match row.state {
        Some(QueueState::Reserved) => {
            if timestamp(row.intent.queue_reservation_expires_at.as_deref()).is_some_and(|time| time <= now) {
                Repair::Expire
            } else { Repair::None }
        }
        Some(QueueState::Starting) | Some(QueueState::RecoveryRequired) => {
            let incident = row.state == Some(QueueState::RecoveryRequired);
            if matches.len() == 1 && verified_reply(matches[0], row) {
                return Repair::Working;
            }
            if incident {
                if row.paused_by_user { return Repair::None; }
                // Keep unresolved incidents in the FIFO. A verified closed, blank
                // guest is safe to retire, but 'loading' is not evidence of closure.
                if row.owner.is_none() && matches.len() == 1 &&
                    closed(matches[0]) && no_response(matches[0]) {
                    return Repair::RetryIncident;
                }
                return Repair::None;
            }
            if age >= 30 && (matches.is_empty() ||
                (matches.len() == 1 && closed(matches[0]) && no_response(matches[0]))) {
                return Repair::Abandon;
            }
            if age >= 45 { Repair::Incident } else { Repair::None }
        }
        Some(QueueState::Working) => {
            if matches.len() == 1 {
                let session = matches[0];
                let same = field(session, "id") == row.intent.queue_session_id.as_deref();
                if same && closed(session) {
                    if row.owner.is_none() { return Repair::RequeueReleased; }
                    if row.owner.as_deref() == row.intent.queue_session_id.as_deref()
                        .map(|id| format!("t3:{id}")).as_deref() && no_response(session) {
                        return Repair::ReclaimClosed;
                    }
                }
                if same && !closed(session) && verified_reply(session, row) {
                    // Idle is not a failure: maintain ownership until positive evidence.
                    return Repair::None;
                }
            }
            if age >= 90 {
                if row.owner.is_none() && matches.is_empty() { Repair::RequeueReleased }
                else { Repair::Incident }
            } else { Repair::None }
        }
        _ => Repair::None, // Manual claims stay monitored; never blindly replace an owner.
    }
}

fn inventory() -> Option<Vec<Value>> {
    let data = crate::agent_notices::shared_t3_inventory()?;
    let sessions = data.get("sessions")?.as_array()?;
    if data.get("hasMore").and_then(Value::as_bool) == Some(true) ||
        data.get("incomplete").and_then(Value::as_bool) == Some(true) ||
        data.get("total").and_then(Value::as_u64).is_some_and(|n| n != sessions.len() as u64) {
        return None;
    }
    Some(sessions.clone())
}

fn apply(row: &Busy, action: Repair, matches: &[&Value]) -> Result<(), String> {
    if action == Repair::None { return Ok(()) }
    let store = FsTaskStore::default();
    let root = row.key.root.as_path();
    let id = row.key.task.as_str();
    let request = row.intent.queue_request.as_deref().unwrap_or("__missing__");
    let reservation = row.intent.queue_reservation_id.as_deref().unwrap_or("__missing__");
    let result = match action {
        Repair::Expire => store.expire_reserved_queue_task(root, id, request, reservation),
        Repair::Abandon => store.requeue_failed_start(root, id, request, reservation,
            "No live original T3 execution verified after 30 minutes"),
        Repair::Working => store.claim_queue_working(root, id, request, reservation,
            field(matches[0], "id").ok_or("Session identity missing")?),
        Repair::RetryIncident => store.retry_queue_incident(root, id, request, reservation,
            "Original T3 session explicitly closed without a conversation or assistant reply"),
        Repair::RequeueReleased => store.requeue_released_worker(root, id, request, reservation,
            row.intent.queue_session_id.as_deref().ok_or("Working session ID missing")?,
            "Original T3 worker confirmed inactive"),
        Repair::ReclaimClosed => store.reclaim_closed_queue_worker(root, id, request, reservation,
            row.intent.queue_session_id.as_deref().ok_or("Working session ID missing")?,
            "Original unstarted T3 worker explicitly closed"),
        Repair::Incident => store.flag_queue_recovery_required(root, id, request, row.state
            .ok_or("Execution phase missing")?, reservation,
            "Execution deadline exceeded without verified T3 progress; retaining original ownership and monitoring"),
        Repair::None => unreachable!(),
    };
    result.map(|_| ()).map_err(|e| e.to_string())
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
    let mut active = scan()?;
    fifo.sync(&mut active);
    let due = fifo.cycle(BATCH);
    let by_key: HashMap<_, _> = active.into_iter().map(|row| (row.key.clone(), row)).collect();
    {
        let mut state = status().lock().unwrap_or_else(|e| e.into_inner());
        state.last_error = None;
        state.queued = fifo.len();
        state.last_checked_at = Some(Utc::now().to_rfc3339());
    }
    if due.is_empty() { return Ok(()) }
    let sessions = inventory();
    if sessions.is_none() {
        status().lock().unwrap_or_else(|e| e.into_inner()).last_error =
            Some("T3 session inventory unavailable; retaining busy tasks for the next FIFO pass".into());
    }
    let session_proof = serde_json::json!({"sessions": sessions});
    let mut checked_manual_boards = HashSet::new();
    let now = Utc::now();
    for key in due {
        let Some(row) = by_key.get(&key) else { continue };
        if row.state == Some(QueueState::Reserved) {
            let action = decide(row, &[], now);
            if action != Repair::None && apply(row, action, &[]).is_ok() { break; }
            continue;
        }
        let Some(sessions) = sessions.as_ref() else { continue };
        if !active_state(row.state) {
            if row.owner.is_some() && checked_manual_boards.insert(row.key.root.clone()) {
                // Existing claim recovery requires explicit permanently-gone T3
                // evidence and a grace period; idle or unloaded is not gone.
                crate::orphan_recovery::reconcile(&row.key.root, Some(&session_proof))?;
            }
            continue;
        }
        let matches = session_owner(row, &sessions);
        let action = decide(row, &matches, now);
        // Never replace an attempt while a different active worker owns the task.
        if matches!(action, Repair::Abandon | Repair::RetryIncident | Repair::RequeueReleased | Repair::ReclaimClosed) &&
            sessions.iter().any(|session| field(session, "spawnRequestId") != row.intent.queue_reservation_id.as_deref() &&
                !closed(session) && session.pointer("/binding/taskRef/id").and_then(Value::as_str) == Some(&row.key.task) &&
                session.pointer("/binding/taskRef/board").and_then(Value::as_str).is_some_and(|board|
                    normalize(board) == normalize(&row.key.root.to_string_lossy()))) {
            continue;
        }
        if action == Repair::None { continue }
        if let Err(error) = apply(row, action, &matches) {
            // A concurrently changed task is retried from fresh board state next tick.
            status().lock().unwrap_or_else(|e| e.into_inner()).last_error =
                Some(format!("{}: {error}", row.key.task));
        } else {
            status().lock().unwrap_or_else(|e| e.into_inner()).last_action =
                Some(format!("{}: {action:?}", row.key.task));
            break; // One mutation per sweep, then reread all authoritative state.
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
            std::thread::sleep(INTERVAL);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn work(id: &str, state: QueueState, minutes_old: i64) -> Busy {
        let when = Utc::now() - chrono::Duration::minutes(minutes_old);
        Busy {
            key: Key {root: PathBuf::from("C:/board"), task:id.into(), attempt:format!("attempt-{id}")},
            state:Some(state), owner:None, paused_by_user:false, since:Some(when),
            intent: ExecutionIntent {
                mode:ExecutionMode::Queue, queue_request:Some(uuid::Uuid::new_v4().to_string()),
                queue_reservation_id:Some(uuid::Uuid::new_v4().to_string()),
                queue_state:Some(state), queue_state_since:Some(when.to_rfc3339()),
                queue_attempt:None,queue_receipt:None,queue_retired_receipt:None,
                queue_reservation_expires_at:None,queue_session_id:None,
                queue_error:None,start_now_request:None,start_now_receipt:None,revision:String::new(),
            },
        }
    }
    #[test]
    fn fifo_rotates_every_busy_state_including_recovery_incidents_and_manual_claims() {
        let mut fifo=Fifo::default();
        let a=work("reserved", QueueState::Reserved, 90);
        let b=work("working", QueueState::Working, 60);
        let c=work("incident", QueueState::RecoveryRequired, 30);
        let mut manual=work("manual", QueueState::Queued, 10);
        manual.state=None;manual.owner=Some("agent:123".into());
        fifo.sync(&mut [a.clone(),b.clone(),c.clone(),manual.clone()]);
        assert_eq!(fifo.cycle(2),vec![a.key.clone(),b.key.clone()]);
        assert_eq!(fifo.cycle(2),vec![c.key.clone(),manual.key.clone()]);
        fifo.sync(&mut [b.clone(),c.clone(),manual.clone()]);
        assert_eq!(fifo.cycle(4),vec![a.key.clone(),b.key.clone(),c.key.clone(),manual.key.clone()]
            .into_iter().filter(|k|k!=&a.key).collect::<Vec<_>>());
    }
    #[test]
    fn expired_reserved_requeues_while_recent_reserved_rotates() {
        let mut old=work("expired", QueueState::Reserved, 10);
        old.intent.queue_reservation_expires_at=Some((Utc::now()-chrono::Duration::minutes(2)).to_rfc3339());
        assert_eq!(decide(&old,&[],Utc::now()), Repair::Expire);
        old.intent.queue_reservation_expires_at=Some((Utc::now()+chrono::Duration::minutes(2)).to_rfc3339());
        assert_eq!(decide(&old,&[],Utc::now()), Repair::None);
    }
    #[test]
    fn missing_start_requeues_and_ambiguous_start_escalates_but_remains_busy() {
        let row=work("starting",QueueState::Starting,46);
        assert_eq!(decide(&row,&[],Utc::now()), Repair::Abandon);
        let ambiguous=json!({"id":"guest","spawnRequestId":"start-starting",
            "status":"loading","bootstrapIdentity":{"state":"awaiting_marker"},
            "totalTurns":0,"promptDelivery":"unknown"});
        assert_eq!(decide(&row,&[&ambiguous],Utc::now()), Repair::Incident);
        let mut incident=row.clone();incident.state=Some(QueueState::RecoveryRequired);
        assert_eq!(decide(&incident,&[&ambiguous],Utc::now()), Repair::None);
        assert!(active_state(incident.state));
    }
    #[test]
    fn incident_is_rechecked_and_only_a_verified_closed_blank_guest_is_retried() {
        let row=work("incident", QueueState::RecoveryRequired, 70);
        let loading=json!({"id":"guest","status":"loading","closedAt":null,
            "bootstrapIdentity":{"state":"awaiting_marker"},"totalTurns":0});
        assert_eq!(decide(&row,&[&loading],Utc::now()), Repair::None);
        let closed=json!({"id":"guest","closedAt":"closed","totalTurns":0,
            "bootstrapIdentity":{"state":"awaiting_marker"}});
        assert_eq!(decide(&row,&[&closed],Utc::now()), Repair::RetryIncident);
        assert_eq!(decide(&row,&[],Utc::now()), Repair::None);
        let mut paused = row.clone(); paused.paused_by_user=true;
        assert_eq!(decide(&paused,&[&closed],Utc::now()), Repair::None);
    }
    #[test]
    fn claimed_worker_is_not_silently_replaced_on_uncertain_session_state() {
        let mut row=work("working", QueueState::Working, 100);
        row.owner=Some("t3:agent".into());row.intent.queue_session_id=Some("agent".into());
        let uncertain=json!({"id":"agent","closedAt":null,"bootstrapIdentity":{"state":"registered"}});
        assert_eq!(decide(&row,&[&uncertain],Utc::now()), Repair::Incident);
        let gone=json!({"id":"agent","closedAt":"closed","totalTurns":0});
        assert_eq!(decide(&row,&[&gone],Utc::now()), Repair::ReclaimClosed);
    }
}
