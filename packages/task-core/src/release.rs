use crate::{BoardConfig, FsTaskStore, Task, TaskKind, TaskStoreError, TaskDraft, TaskStore, AI_READY_COLUMN_ID};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, fs, path::{Path, PathBuf}};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all="snake_case")]
pub enum ReleaseStage { AwaitingIntegration, Ready, Deployed, Verified, VerificationFailed, Unknown }
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourceRef {
    pub repository: String,
    pub commit: String,
    #[serde(default, skip_serializing_if="Option::is_none")]
    pub integrated_ref: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReleaseMember {
    pub task_id: String,
    pub stage: ReleaseStage,
    #[serde(default)] pub sources: Vec<SourceRef>,
    #[serde(default)] pub evidence: Vec<String>,
    #[serde(default)] pub attempt_ids: Vec<u32>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReleaseAttempt {
    pub number: u32,
    pub created: String,
    pub status: String,
    pub members: Vec<ReleaseMember>,
    pub manifest: Vec<SourceRef>,
    /// Durable correlation ID shared by one authorized build script invocation.
    #[serde(default, skip_serializing_if="Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default)] pub evidence: Vec<String>,
    #[serde(default)] pub repair_task_id: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReleaseRecord {
    pub schema: u32,
    pub kind: TaskKind,
    pub id: String,
    pub status: String,
    pub created: String,
    #[serde(default)] pub members: Vec<ReleaseMember>,
    #[serde(default)] pub attempts: Vec<ReleaseAttempt>,
    #[serde(default)] pub excluded: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseSummary {
    pub releases: Vec<ReleaseRecord>,
    pub integration_needed: usize,
    pub ready: usize,
    pub verified: usize,
    pub deployed: usize,
    pub verification_failed: usize,
}

fn dir(root: &Path) -> PathBuf { root.join("Releases") }
fn read_release(path: &Path) -> Result<ReleaseRecord, TaskStoreError> {
    let text = fs::read_to_string(path)?;
    let yaml = text.strip_prefix("---\n").and_then(|s| s.split_once("\n---")).map(|(yaml, _)| yaml)
        .ok_or_else(|| TaskStoreError::Parse(format!("invalid release file {}", path.display())))?;
    let record: ReleaseRecord = serde_yaml::from_str(yaml).map_err(|e| TaskStoreError::Parse(e.to_string()))?;
    if record.kind != TaskKind::Release { return Err(TaskStoreError::Invalid("release kind must be release".into())); }
    Ok(record)
}
fn write_release(root: &Path, record: &ReleaseRecord) -> Result<(), TaskStoreError> {
    fs::create_dir_all(dir(root))?;
    let path = dir(root).join(format!("{}.md", record.id));
    let yaml = serde_yaml::to_string(record).map_err(|e| TaskStoreError::Parse(e.to_string()))?;
    // TaskBoard's writer supports cloud-backed volumes without rename-over-existing.
    super::store::write_text_atomic(&path, &format!("---\n{yaml}---\n\n# Release {}\n", record.id))?;
    Ok(())
}
pub(crate) fn list_unlocked(root: &Path) -> Result<Vec<ReleaseRecord>, TaskStoreError> {
    let mut result = Vec::new();
    if dir(root).exists() {
        for file in fs::read_dir(dir(root))? {
            let file = file?.path();
            if file.extension().is_some_and(|ext| ext == "md") { result.push(read_release(&file)?); }
        }
    }
    result.sort_by(|a,b| a.id.cmp(&b.id));
    Ok(result)
}
fn summarize(releases: Vec<ReleaseRecord>) -> ReleaseSummary {
    let mut summary = ReleaseSummary { releases, integration_needed: 0, ready: 0, verified: 0, deployed: 0, verification_failed: 0 };
    for member in summary.releases.iter().flat_map(|r| &r.members) {
        match member.stage {
            ReleaseStage::AwaitingIntegration | ReleaseStage::Unknown => summary.integration_needed += 1,
            ReleaseStage::Ready => summary.ready += 1,
            ReleaseStage::Deployed => summary.deployed += 1,
            ReleaseStage::Verified => summary.verified += 1,
            ReleaseStage::VerificationFailed => summary.verification_failed += 1,
        }
    }
    summary
}
fn releases_seen_member(root: &Path, task_id: &str, except_id: &str) -> Result<bool, TaskStoreError> {
    Ok(list_unlocked(root)?.iter().any(|r| r.id != except_id && r.members.iter().any(|m| m.task_id == task_id)))
}
fn eligible(config: &BoardConfig, task: &Task, include_historical: bool) -> bool {
    if !config.releases_enabled || task.kind != TaskKind::Task || task.deployable == Some(false) || task.completed.is_none() { return false; }
    if include_historical { return true; }
    let Some(start) = config.release_enabled_at.as_deref() else { return false; };
    task.completed_at.as_deref().is_some_and(|date| date >= start)
}
pub(crate) fn reconcile_unlocked(root: &Path, config: &BoardConfig, tasks: &[Task], include_historical: bool) -> Result<ReleaseSummary, TaskStoreError> {
    let mut releases = list_unlocked(root)?;
    if !config.releases_enabled { return Ok(summarize(releases)); }
    // Reopened and excluded work is never fabricated as deployed. Preserve immutable attempt evidence.
    let done: HashSet<&str> = tasks.iter().filter(|t| eligible(config, t, include_historical)).map(|t| t.id.as_str()).collect();
    for release in &mut releases {
        let existing_attempts: HashSet<&str> = release.attempts.iter().flat_map(|a| a.members.iter().map(|m| m.task_id.as_str())).collect();
        let before = release.members.len();
        release.members.retain(|m| done.contains(m.task_id.as_str()) || existing_attempts.contains(m.task_id.as_str()));
        let mut modified = before != release.members.len();
        for member in &mut release.members {
            if !done.contains(member.task_id.as_str()) && existing_attempts.contains(member.task_id.as_str()) && member.stage != ReleaseStage::Unknown {
                member.stage = ReleaseStage::Unknown;
                member.evidence.push("Implementation task reopened/removed; immutable attempt preserved, current membership needs reassessment".into());
                modified = true;
            }
        }
        if modified { write_release(root, release)?; }
    }
    let mut enrolled: HashSet<String> = releases.iter().flat_map(|r| &r.members).map(|m| m.task_id.clone()).collect();
    enrolled.extend(releases.iter().flat_map(|r| r.excluded.iter().cloned()));
    let mut missing: Vec<&Task> = tasks.iter().filter(|t| done.contains(t.id.as_str()) && !enrolled.contains(t.id.as_str())).collect();
    missing.sort_by(|a,b| a.completed_at.cmp(&b.completed_at).then(a.id.cmp(&b.id)));
    if !missing.is_empty() {
        let index = releases.iter().rposition(|r| r.status == "collecting");
        let release_index = if let Some(index) = index { index } else {
            let next = releases.iter().filter_map(|r| r.id.strip_prefix("REL-")?.parse::<u32>().ok()).max().unwrap_or(0) + 1;
            releases.push(ReleaseRecord { schema: 1, kind: TaskKind::Release, id: format!("REL-{next:04}"), status: "collecting".into(), created: Utc::now().to_rfc3339(), members: vec![], attempts: vec![], excluded: vec![] });
            releases.len()-1
        };
        let record = &mut releases[release_index];
        for task in missing {
            record.members.push(ReleaseMember { task_id: task.id.clone(), stage: ReleaseStage::AwaitingIntegration, sources: vec![], evidence: vec![], attempt_ids: vec![] });
        }
        write_release(root, record)?;
    }
    Ok(summarize(releases))
}
/// Converges interrupted failures under the same cross-process board lock.
///
/// Crash windows:
/// 1. outcome file committed, repair absent -> make precisely one repair;
/// 2. repair task committed, backlink absent -> discover deterministic marker;
/// 3. backlink committed -> read-only on future retries.
///
/// A repair is *not* release verification: only actual outcome evidence may
/// change deployment/verification status.
pub(crate) fn reconcile_repairs_unlocked(root: &Path) -> Result<bool, TaskStoreError> {
    let mut changed = false;
    let store = FsTaskStore::default();
    let mut snapshot = store.load_unlocked(root)?;
    for mut release in list_unlocked(root)? {
        let mut release_changed = false;
        for attempt in &mut release.attempts {
            if !matches!(attempt.status.as_str(), "build_failed" | "verification_failed") {
                continue;
            }
            // An explicit, existing backlink is authoritative.
            if attempt.repair_task_id.as_deref().is_some_and(|id| snapshot.tasks.iter().any(|t| &t.id == id)) {
                continue;
            }
            let marker = format!("RELEASE_REPAIR:{}:{}", release.id, attempt.number);
            let existing = snapshot.tasks.iter().find(|t| t.ai_notes.lines().any(|line| line == marker));
            let repair_id = if let Some(task) = existing {
                task.id.clone()
            } else {
                let related_to = attempt.members.iter().map(|m| m.task_id.clone())
                    .filter(|id| snapshot.tasks.iter().any(|t| &t.id == id)).collect();
                let draft = TaskDraft {
                    kind: TaskKind::Task, review_mode: crate::ReviewMode::Inbox,
                    deployable: Some(true), blocked_reason: None,
                    repeat_every_days: None, next_due: None,
                    reserved_id: None, reservation_token: None,
                    depends_on: vec![], related_to,
                    title: format!("Repair {} attempt {} {}", release.id, attempt.number, attempt.status),
                    column: snapshot.config.columns.iter()
                        .find(|c| c.id == AI_READY_COLUMN_ID)
                        .or_else(|| snapshot.config.columns.iter().find(|c| c.ai_ready && !c.terminal))
                        .or_else(|| snapshot.config.columns.iter().find(|c| !c.terminal))
                        .ok_or_else(|| TaskStoreError::Invalid("no executable repair task column".into()))?.id.clone(),
                    parent: None, category: None, icon: Some("bug".into()),
                    attention: vec![], question: None,
                    description: format!("{} attempt {} failed. Evidence: {}. Fix only this failed build/verification; do not reopen original completed implementation.", release.id, attempt.number, attempt.evidence.join("; ")),
                    acceptance_criteria: vec!["Validate corrective source change before a new release attempt".into()],
                    ai_notes: marker, user_notes: String::new(), notes: String::new(),
                };
                let task = store.create_as_unlocked(root, draft, crate::MutationActor::Agent)?;
                snapshot.tasks.push(task.clone());
                task.id
            };
            attempt.repair_task_id = Some(repair_id);
            release_changed = true;
        }
        if release_changed {
            write_release(root, &release)?;
            changed = true;
        }
    }
    Ok(changed)
}

impl FsTaskStore {
    pub fn release_new(&self, root: &Path) -> Result<ReleaseRecord, TaskStoreError> {
        let _lock = super::store::acquire_lock(root)?;
        let board = self.load_unlocked(root)?;
        if !board.config.releases_enabled {
            return Err(TaskStoreError::Invalid("enable board releases before manually creating the next release".into()));
        }
        let mut releases = list_unlocked(root)?;
        let next = releases.iter().filter_map(|r| r.id.strip_prefix("REL-")?.parse::<u32>().ok()).max().unwrap_or(0) + 1;
        // The new collecting release receives later Done items; previous release work continues uninterrupted.
        for prior in releases.iter_mut().filter(|r| r.status == "collecting") {
            prior.status = "preparing".into();
            write_release(root, prior)?;
        }
        let record = ReleaseRecord {
            schema: 1, kind: TaskKind::Release, id: format!("REL-{next:04}"), status: "collecting".into(),
            created: Utc::now().to_rfc3339(), members: vec![], attempts: vec![], excluded: vec![],
        };
        write_release(root, &record)?;
        Ok(record)
    }
    pub fn release_migrate_blocked(&self, root: &Path, task_id: &str, evidence: &str) -> Result<Task, TaskStoreError> {
        if evidence.trim().is_empty() {
            return Err(TaskStoreError::Invalid("explicit blocked migration requires source-validation evidence".into()));
        }
        let snapshot = self.load(root)?;
        if !snapshot.config.releases_enabled {
            return Err(TaskStoreError::Invalid("board must explicitly enable releases before blocked migration".into()));
        }
        let mut task = snapshot.tasks.iter().find(|t| t.id == task_id && !t.archived)
            .cloned().ok_or_else(|| TaskStoreError::NotFound(task_id.into()))?;
        if task.kind != TaskKind::Task ||
           (task.column != "blocked" && !task.attention.contains(&crate::TaskAttention::Blocked)) {
            return Err(TaskStoreError::Invalid("explicit migration only supports an active blocked executable task".into()));
        }
        let terminal = snapshot.config.columns.iter().filter(|c| c.terminal).min_by_key(|c| c.order)
            .ok_or_else(|| TaskStoreError::Invalid("board has no Done column".into()))?;
        task.column = terminal.id.clone();
        task.attention.retain(|flag| *flag != crate::TaskAttention::Blocked);
        task.ai_notes = if task.ai_notes.trim().is_empty() {
            format!("Source-tested/committed implementation manually migrated from Blocked to Done: {evidence}")
        } else {
            format!("{}\n\nSource-tested/committed implementation manually migrated from Blocked to Done: {evidence}", task.ai_notes)
        };
        let completed = self.update(root, task)?;
        let _lock = super::store::acquire_lock(root)?;
        for mut release in list_unlocked(root)? {
            if let Some(member) = release.members.iter_mut().find(|m| m.task_id == task_id) {
                member.evidence.push(format!("Explicit human-initiated blocked migration: {evidence}"));
                write_release(root, &release)?;
                break;
            }
        }
        Ok(completed)
    }
    pub fn releases(&self, root: &Path) -> Result<ReleaseSummary, TaskStoreError> {
        let _lock = super::store::acquire_lock(root)?;
        let snapshot = self.load_unlocked(root)?;
        reconcile_unlocked(root, &snapshot.config, &snapshot.tasks, false)
    }
    pub fn reconcile_releases(&self, root: &Path, historical: bool) -> Result<ReleaseSummary, TaskStoreError> {
        let _lock = super::store::acquire_lock(root)?;
        let snapshot = self.load_unlocked(root)?;
        reconcile_unlocked(root, &snapshot.config, &snapshot.tasks, historical)
    }
    pub fn release_enable(&self, root: &Path, enabled: bool) -> Result<ReleaseSummary, TaskStoreError> {
        let _lock = super::store::acquire_lock(root)?;
        let snapshot = self.load_unlocked(root)?;
        let mut config = snapshot.config.clone();
        config.releases_enabled = enabled;
        if enabled && config.release_enabled_at.is_none() { config.release_enabled_at = Some(Utc::now().to_rfc3339()); }
        let path = root.join("_board.md");
        let current = fs::read_to_string(&path)?;
        super::store::write_text_atomic(&path, &super::store::render_board(&config, Some(&current))?)?;
        reconcile_unlocked(root, &config, &snapshot.tasks, false)
    }
    pub fn release_edit(&self, root: &Path, action: &str, release_id: &str, task_id: Option<&str>, source: Option<SourceRef>, evidence: Option<&str>) -> Result<ReleaseRecord, TaskStoreError> {
        let _lock = super::store::acquire_lock(root)?;
        let snapshot = self.load_unlocked(root)?;
        let mut releases = reconcile_unlocked(root, &snapshot.config, &snapshot.tasks, false)?.releases;
        let record = releases.iter_mut().find(|r| r.id == release_id).ok_or_else(|| TaskStoreError::NotFound(release_id.into()))?;
        match action {
            "include" => {
                let task_id = task_id.ok_or_else(|| TaskStoreError::Invalid("task ID required".into()))?;
                let task = snapshot.tasks.iter().find(|t| t.id == task_id).ok_or_else(|| TaskStoreError::NotFound(task_id.into()))?;
                if !eligible(&snapshot.config, task, true) { return Err(TaskStoreError::Invalid("task is not deployable Done work".into())); }
                if releases_seen_member(root, task_id, release_id)? { return Err(TaskStoreError::Invalid("task is already assigned to another release; move it explicitly".into())); }
                record.excluded.retain(|id| id != task_id);
                if !record.members.iter().any(|m| m.task_id == task_id) { record.members.push(ReleaseMember { task_id: task_id.into(), stage: ReleaseStage::AwaitingIntegration, sources: vec![], evidence: vec![], attempt_ids: vec![] }); }
            }
            "exclude" => {
                let id = task_id.ok_or_else(|| TaskStoreError::Invalid("task ID required".into()))?;
                if record.attempts.iter().any(|a| a.members.iter().any(|m| m.task_id == id)) { return Err(TaskStoreError::Invalid("cannot erase immutable attempt member; move future work to another release".into())); }
                record.members.retain(|m| m.task_id != id);
                if !record.excluded.contains(&id.to_string()) { record.excluded.push(id.to_string()); }
            }
            "source" => {
                let source = source.ok_or_else(|| TaskStoreError::Invalid("source required".into()))?;
                if source.repository.trim().is_empty() || source.commit.trim().is_empty() { return Err(TaskStoreError::Invalid("source requires repository and exact commit".into())); }
                let member = record.members.iter_mut().find(|m| Some(m.task_id.as_str()) == task_id).ok_or_else(|| TaskStoreError::NotFound("release member".into()))?;
                let repeated = member.sources.iter().any(|s| s == &source);
                if !repeated {
                    member.sources.retain(|s| s.repository != source.repository);
                    member.sources.push(source);
                    member.stage = if member.sources.iter().all(|s| s.integrated_ref.is_some()) {
                        ReleaseStage::Ready
                    } else { ReleaseStage::AwaitingIntegration };
                }
                if let Some(note) = evidence {
                    if !member.evidence.iter().any(|e| e == note) {
                        member.evidence.push(note.into());
                    }
                }
            }
            "move" => {
                let destination = evidence.ok_or_else(|| TaskStoreError::Invalid("destination release ID required".into()))?;
                if !list_unlocked(root)?.iter().any(|r| r.id == destination) || destination == record.id { return Err(TaskStoreError::Invalid("destination release must exist and differ".into())); }
                if record.attempts.iter().any(|a| a.members.iter().any(|m| Some(m.task_id.as_str()) == task_id)) { return Err(TaskStoreError::Invalid("attempt member immutable".into())); }
                let pos = record.members.iter().position(|m| Some(m.task_id.as_str()) == task_id).ok_or_else(|| TaskStoreError::NotFound("release member".into()))?;
                let member = record.members.remove(pos);
                record.excluded.push(member.task_id.clone());
                write_release(root, record)?;
                let target = releases.iter_mut().find(|r| r.id == destination).ok_or_else(|| TaskStoreError::NotFound(destination.into()))?;
                if !target.members.iter().any(|m| m.task_id == member.task_id) { target.members.push(member); }
                write_release(root, target)?;
                return Ok(target.clone());
            }
            _ => return Err(TaskStoreError::Invalid(format!("invalid release action {action}"))),
        };
        write_release(root, record)?;
        Ok(record.clone())
    }
    pub fn release_prepare(&self, root: &Path, id: &str, evidence: &str) -> Result<ReleaseRecord, TaskStoreError> {
        self.release_prepare_bound(root, id, evidence, None)
    }
    pub fn release_prepare_bound(&self, root: &Path, id: &str, evidence: &str, run_id: Option<&str>) -> Result<ReleaseRecord, TaskStoreError> {
        let run_id = run_id.map(str::trim);
        if run_id.is_some_and(|id| id.len() < 8 || id.len() > 128 || id.chars().any(char::is_whitespace)) {
            return Err(TaskStoreError::Invalid("run_id must be 8-128 printable non-whitespace characters".into()));
        }
        let _lock = super::store::acquire_lock(root)?;
        let mut record = list_unlocked(root)?.into_iter().find(|r| r.id == id).ok_or_else(|| TaskStoreError::NotFound(id.into()))?;
        if let Some(run_id) = run_id {
            if record.attempts.iter().any(|a| a.run_id.as_deref() == Some(run_id)) {
                // Replayed preflight: never create a second attempt.
                return Ok(record);
            }
        }
        if record.attempts.last().is_some_and(|a| a.status == "prepared" || a.status == "unknown") {
            return Err(TaskStoreError::Invalid("previous attempt still prepared/unknown; reconcile its outcome first".into()));
        }
        let already_deployed: HashSet<String> = record.attempts.iter().filter(|a| a.status == "deployed" || a.status == "verified").flat_map(|a| a.members.iter().map(|m| m.task_id.clone())).collect();
        let members: Vec<_> = record.members.iter().filter(|m| matches!(m.stage, ReleaseStage::Ready | ReleaseStage::VerificationFailed) && !already_deployed.contains(&m.task_id)
            && !m.sources.is_empty() && m.sources.iter().all(|s| s.integrated_ref.as_ref().is_some_and(|v| !v.is_empty()))).cloned().collect();
        if members.is_empty() { return Err(TaskStoreError::Invalid("no integrated ready members; record exact source and integrated ref before prepare".into())); }
        let mut manifest: Vec<SourceRef> = members.iter().flat_map(|m| m.sources.iter().cloned()).collect();
        manifest.sort_by(|a,b| a.repository.cmp(&b.repository).then(a.commit.cmp(&b.commit)));
        manifest.dedup();
        for index in 0..manifest.len() {
            if manifest.iter().skip(index + 1).any(|other| other.repository == manifest[index].repository && other.integrated_ref != manifest[index].integrated_ref) {
                return Err(TaskStoreError::Invalid("conflicting integrated revisions for one repository in an attempt; align exact integrated refs".into()));
            }
        }
        let number = record.attempts.len() as u32 + 1;
        for member in &mut record.members { if members.iter().any(|m| m.task_id == member.task_id) { member.attempt_ids.push(number); } }
        record.attempts.push(ReleaseAttempt { number, created: Utc::now().to_rfc3339(), status: "prepared".into(), members, manifest, run_id: run_id.map(str::to_owned), evidence: if evidence.is_empty() { vec![] } else { vec![evidence.into()] }, repair_task_id: None });
        write_release(root, &record)?;
        Ok(record)
    }
    /// Validate one immutable member against the exact deployed source snapshot.
    /// A process startup is not evidence that all features worked.
    pub fn release_verify_member(&self, root: &Path, id: &str, attempt: u32,
        task_id: &str, passed: bool, evidence: &str) -> Result<ReleaseRecord, TaskStoreError> {
        if evidence.trim().is_empty() {
            return Err(TaskStoreError::Invalid("per-member verification needs an objective receipt".into()));
        }
        let _lock = super::store::acquire_lock(root)?;
        let board = self.load_unlocked(root)?;
        let mut record = list_unlocked(root)?.into_iter().find(|r| r.id == id)
            .ok_or_else(|| TaskStoreError::NotFound(id.into()))?;
        let (immutable, deployed) = {
            let a = record.attempts.iter().find(|a| a.number == attempt)
                .ok_or_else(|| TaskStoreError::NotFound(format!("attempt {attempt}")))?;
            (a.members.iter().find(|m| m.task_id == task_id).cloned()
                .ok_or_else(|| TaskStoreError::NotFound(format!("attempt member {task_id}")))?,
             matches!(a.status.as_str(), "deployed" | "verified" | "verification_failed"))
        };
        if !deployed { return Err(TaskStoreError::Invalid("cannot verify before a proven deployment".into())); }
        let member = record.members.iter_mut().find(|m| m.task_id == task_id)
            .ok_or_else(|| TaskStoreError::NotFound(format!("release member {task_id}")))?;
        if immutable.sources != member.sources
            || !board.tasks.iter().any(|t| t.id == task_id && t.completed.is_some()) {
            return Err(TaskStoreError::Invalid("verification source differs from deployed immutable manifest".into()));
        }
        let stage = if passed { ReleaseStage::Verified } else { ReleaseStage::VerificationFailed };
        let label = if passed { "verified" } else { "verification_failed" };
        let receipt = format!("attempt {attempt}: {label}: {evidence}");
        if member.stage == stage && member.evidence.contains(&receipt) { return Ok(record); }
        if !matches!(member.stage, ReleaseStage::Deployed) {
            return Err(TaskStoreError::Invalid("member was already verified or changed; open a new attempt for further changes".into()));
        }
        member.stage = stage;
        member.evidence.push(receipt);
        let target = record.attempts.iter_mut().find(|a| a.number == attempt).unwrap();
        let all_verified = target.members.iter().all(|m| record.members.iter().any(|current|
            current.task_id == m.task_id && current.sources == m.sources && current.stage == ReleaseStage::Verified));
        let any_failed = target.members.iter().any(|m| record.members.iter().any(|current|
            current.task_id == m.task_id && current.sources == m.sources && current.stage == ReleaseStage::VerificationFailed));
        if any_failed { target.status = "verification_failed".into(); }
        else if all_verified { target.status = "verified".into(); }
        if record.members.iter().all(|m| m.stage == ReleaseStage::Verified) {
            record.status = "completed".into();
        }
        write_release(root, &record)?;
        if any_failed {
            reconcile_repairs_unlocked(root)?;
            return list_unlocked(root)?.into_iter().find(|r| r.id == id)
                .ok_or_else(|| TaskStoreError::NotFound(id.into()));
        }
        Ok(record)
    }
    pub fn release_outcome(&self, root: &Path, id: &str, attempt: u32, status: &str, evidence: &str) -> Result<ReleaseRecord, TaskStoreError> {
        self.release_outcome_bound(root, id, attempt, status, evidence, None)
    }
    pub fn release_outcome_bound(&self, root: &Path, id: &str, attempt: u32, status: &str, evidence: &str, run_id: Option<&str>) -> Result<ReleaseRecord, TaskStoreError> {
        if !matches!(status, "build_failed" | "deployed" | "unknown" | "verified" | "verification_failed") {
            return Err(TaskStoreError::Invalid("outcome must be build_failed/deployed/unknown/verified/verification_failed".into()));
        }
        if evidence.trim().is_empty() { return Err(TaskStoreError::Invalid("outcome evidence required".into())); }
        let _lock = super::store::acquire_lock(root)?;
        let board = self.load_unlocked(root)?;
        let mut record = list_unlocked(root)?.into_iter().find(|r| r.id == id).ok_or_else(|| TaskStoreError::NotFound(id.into()))?;
        let target = record.attempts.iter_mut().find(|a| a.number == attempt).ok_or_else(|| TaskStoreError::NotFound(format!("attempt {attempt}")))?;
        if let Some(run_id) = run_id {
            if target.run_id.as_deref() != Some(run_id) {
                return Err(TaskStoreError::Invalid("deployment run ID mismatch: refusing cross-attempt outcome".into()));
            }
        }
        if target.status == status && target.evidence.iter().any(|e| e == evidence) {
            return Ok(record); // stable script retry after a lost acknowledgement
        }
        let valid_transition = matches!((target.status.as_str(), status),
            ("prepared", "build_failed" | "deployed" | "unknown") |
            ("unknown", "build_failed" | "deployed") |
            ("deployed", "verified" | "verification_failed"));
        if !valid_transition {
            return Err(TaskStoreError::Invalid("invalid deployment attempt transition: prepared -> build_failed/unknown/deployed -> verification".into()));
        }
        if status == "verified" || status == "verification_failed" {
            return Err(TaskStoreError::Invalid("verify individual members with release verify; a process restart cannot verify all features".into()));
        }
        target.status = status.into();
        target.evidence.push(evidence.into());
        for member in &mut record.members {
            if let Some(included) = target.members.iter().find(|m| m.task_id == member.task_id) {
                if !board.tasks.iter().any(|t| t.id == member.task_id && t.completed.is_some()) {
                    member.stage = ReleaseStage::Unknown;
                    member.evidence.push(format!("attempt {attempt} {status}: task has since reopened; current status remains unknown"));
                    continue;
                }
                // An attempt verifies only its own immutable source snapshot, never newer edits.
                if included.sources != member.sources {
                    member.evidence.push(format!("attempt {attempt} {status} applied only to older manifest; current source differs"));
                    continue;
                }
                member.stage = match status {
                    "deployed" => ReleaseStage::Deployed,
                    "verified" => ReleaseStage::Verified,
                    "verification_failed" => ReleaseStage::VerificationFailed,
                    "unknown" => ReleaseStage::Unknown,
                    _ => ReleaseStage::Ready,
                };
                member.evidence.push(format!("attempt {attempt}: {status}: {evidence}"));
            }
        }
        if status == "verified" && record.members.iter().all(|m| m.stage == ReleaseStage::Verified) { record.status = "completed".into(); }
        write_release(root, &record)?;
        // Still holding the board lock: the stable marker prevents a duplicate
        // if a process dies after task creation but before the backlink write.
        // Future core loads also retry this convergence after an interruption.
        if matches!(status, "build_failed" | "verification_failed") {
            reconcile_repairs_unlocked(root)?;
            return list_unlocked(root)?.into_iter().find(|r| r.id == id)
                .ok_or_else(|| TaskStoreError::NotFound(id.into()));
        }
        Ok(record)
    }
}
