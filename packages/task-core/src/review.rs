//! Durable, board-scoped review requests and human verdicts.
//! The JSON manifest is the source of truth; task frontmatter mirrors the open state.
use super::*;
use chrono::SecondsFormat;

const MAX_SYNC_FILE: u64 = 25 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewArtifact {
    pub id: String,
    pub label: String,
    pub kind: String,
    pub storage: String,
    pub path: String,
    #[serde(default)]
    pub pruned: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poster: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewPin {
    pub x: f64,
    pub y: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub viewport_width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub viewport_height: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scroll_x: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scroll_y: Option<f64>,
    #[serde(default)]
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewVerdict {
    pub verdict: String,
    #[serde(default)]
    pub choices: Vec<String>,
    #[serde(default)]
    pub ranking: Vec<String>,
    #[serde(default)]
    pub pins: Vec<ReviewPin>,
    #[serde(default)]
    pub note: String,
    pub resolved_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewRound {
    pub number: u32,
    pub mode: String,
    pub judge: String,
    #[serde(default)]
    pub criteria: Vec<String>,
    #[serde(default)]
    pub artifacts: Vec<ReviewArtifact>,
    #[serde(default)]
    pub references: Vec<ReviewArtifact>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous: Option<ReviewArtifact>,
    pub top_k: usize,
    pub requested_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<ReviewVerdict>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loser_prune_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewManifest {
    pub schema: u32,
    pub task_id: String,
    pub status: String,
    pub current_round: u32,
    pub rounds: Vec<ReviewRound>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub implemented_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openrig_gate: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receipt_prune_at: Option<String>,
    #[serde(default)]
    pub revision: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewInputArtifact {
    pub source: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewRequest {
    pub task_id: String,
    pub mode: String,
    pub judge: String,
    #[serde(default)]
    pub criteria: Vec<String>,
    pub artifacts: Vec<ReviewInputArtifact>,
    #[serde(default)]
    pub references: Vec<ReviewInputArtifact>,
    #[serde(default)]
    pub previous: Option<ReviewInputArtifact>,
    #[serde(default)]
    pub poster: Option<String>,
    #[serde(default)]
    pub top_k: Option<usize>,
    #[serde(default)]
    pub implemented_by: Option<String>,
    #[serde(default)]
    pub openrig_gate: Option<String>,
    #[serde(default)]
    pub expected_task_revision: Option<String>,
    #[serde(default)]
    pub expected_manifest_revision: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewReconsider {
    pub task_id: String,
    pub expected_task_revision: String,
    pub expected_manifest_revision: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewResolution {
    pub task_id: String,
    pub verdict: String,
    #[serde(default)]
    pub choices: Vec<String>,
    #[serde(default)]
    pub ranking: Vec<String>,
    #[serde(default)]
    pub pins: Vec<ReviewPin>,
    #[serde(default)]
    pub note: String,
    pub expected_task_revision: String,
    pub expected_manifest_revision: String,
}

fn now() -> String { Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true) }
fn manifest_file(root: &Path, id: &str) -> PathBuf {
    root.join("Reviews").join(id).join("manifest.json")
}
fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
}
fn parse_manifest(path: &Path) -> Result<ReviewManifest, TaskStoreError> {
    let raw = fs::read_to_string(path)?;
    let mut manifest: ReviewManifest =
        serde_json::from_str(raw.trim_start_matches('\u{feff}')).map_err(|e| TaskStoreError::Parse(e.to_string()))?;
    manifest.revision = revision(&raw);
    Ok(manifest)
}
fn save_manifest(path: &Path, manifest: &ReviewManifest) -> Result<(), TaskStoreError> {
    let mut clone = manifest.clone();
    clone.revision.clear(); // derived from bytes; never hash itself recursively
    let json = serde_json::to_string_pretty(&clone)
        .map_err(|e| TaskStoreError::Parse(e.to_string()))?;
    write_text_atomic(path, &(json + "\n"))
}
fn path_size(path: &Path) -> Result<u64, TaskStoreError> {
    if path.is_file() { return Ok(fs::metadata(path)?.len()); }
    if !path.is_dir() { return Err(TaskStoreError::NotFound(path.display().to_string())); }
    let mut sum = 0;
    for item in fs::read_dir(path)? {
        let entry = item?;
        if entry.file_type()?.is_symlink() { return Err(TaskStoreError::Invalid("review artifact symlinks are not supported".into())); }
        sum += path_size(&entry.path())?;
    }
    Ok(sum)
}
fn copy_path(from: &Path, to: &Path) -> Result<(), TaskStoreError> {
    if from.is_file() {
        if let Some(parent) = to.parent() { fs::create_dir_all(parent)?; }
        fs::copy(from, to)?;
    } else {
        fs::create_dir_all(to)?;
        for entry in fs::read_dir(from)? {
            let entry = entry?;
            if entry.file_type()?.is_symlink() { return Err(TaskStoreError::Invalid("review artifact symlinks are not supported".into())); }
            copy_path(&entry.path(), &to.join(entry.file_name()))?;
        }
    }
    Ok(())
}
fn local_cache(root: &Path, task_id: &str, round: u32) -> Result<PathBuf, TaskStoreError> {
    let appdata = std::env::var_os("LOCALAPPDATA")
        .ok_or_else(|| TaskStoreError::Invalid("LOCALAPPDATA not available for local review media".into()))?;
    let name = root.file_name().and_then(|v| v.to_str()).unwrap_or("Tasks");
    let board_key = revision(&root.to_string_lossy());
    Ok(PathBuf::from(appdata).join("TaskBoard").join("review-media")
        .join(format!("{name}-{}", &board_key[..12]))
        .join(task_id).join(format!("r{round}")))
}
fn kind_of(source: &str) -> Result<String, TaskStoreError> {
    if source.starts_with("http://") || source.starts_with("https://") {
        return Ok("url".to_string());
    }
    let p = Path::new(source);
    if p.is_dir() && p.join("index.html").is_file() { return Ok("html".to_string()); }
    let extension = p.extension().and_then(|v| v.to_str()).unwrap_or("").to_ascii_lowercase();
    let kind = match extension.as_str() {
        "png" | "jpg" | "jpeg" | "webp" | "gif" => "image",
        "mp4" | "webm" => "video",
        "html" | "htm" => "html",
        _ => return Err(TaskStoreError::Invalid(format!("unsupported review artifact: {source}"))),
    };
    if !p.is_file() { return Err(TaskStoreError::NotFound(source.into())); }
    Ok(kind.into())
}
fn capture_html_poster(source: &Path, output: &Path) -> bool {
    #[cfg(windows)]
    {
        let browser = [
            r"C:\Program Files\Google\Chrome\Application\chrome.exe",
            r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
        ].iter().find(|candidate| Path::new(candidate).is_file());
        let Some(browser) = browser else { return false };
        let input = if source.is_dir() { source.join("index.html") } else { source.to_path_buf() };
        let Ok(input) = input.canonicalize() else { return false };
        let address = format!("file:///{}", input.to_string_lossy().replace('\\', "/"));
        let profile = std::env::temp_dir().join(format!("taskboard-review-capture-{}-{}",
            std::process::id(), Utc::now().timestamp_micros()));
        let result = std::process::Command::new(browser)
            .args(["--headless=new", "--disable-gpu", "--disable-extensions", "--no-first-run",
                "--disable-background-networking", "--hide-scrollbars", "--window-size=480,300"])
            .arg(format!("--user-data-dir={}", profile.display()))
            .arg(format!("--screenshot={}", output.display()))
            .arg(address)
            .output();
        let launched = result.is_ok_and(|value| value.status.success());
        // Chromium may detach its screenshot writer just after the parent process exits.
        for _ in 0..60 {
            if output.metadata().is_ok_and(|meta| meta.len() > 0) { break; }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let _ = fs::remove_dir_all(profile);
        launched && output.metadata().is_ok_and(|meta| meta.len() > 0)
    }
    #[cfg(not(windows))]
    { let _ = (source, output); false }
}

fn copy_artifact(
    root: &Path, task_id: &str, round: u32, index: usize,
    entry: &ReviewInputArtifact, poster: Option<&str>,
) -> Result<ReviewArtifact, TaskStoreError> {
    let kind = kind_of(&entry.source)?;
    let id = format!("{}", (b'A' + index as u8) as char);
    let name = Path::new(&entry.source).file_name().and_then(|v| v.to_str())
        .unwrap_or("artifact").to_string();
    let round_dir = root.join("Reviews").join(task_id).join(format!("r{round}"));
    fs::create_dir_all(&round_dir)?;
    let mut poster_path = None;
    if let Some(poster) = poster {
        let file = Path::new(poster);
        let extension = file.extension().and_then(|v| v.to_str()).unwrap_or("webp");
        if !["png", "jpg", "jpeg", "webp", "gif"].contains(&extension.to_ascii_lowercase().as_str())
            || !file.is_file() {
            return Err(TaskStoreError::Invalid("poster must be an existing image file".into()));
        }
        let filename = format!("{id}.poster.{extension}");
        fs::copy(file, round_dir.join(&filename))?;
        poster_path = Some(format!("Reviews/{task_id}/r{round}/{filename}"));
    }
    // ffmpeg is optional. If installed, derive a small poster automatically.
    if kind == "html" && poster_path.is_none() {
        let poster_name = format!("{id}.poster.png");
        if capture_html_poster(Path::new(&entry.source), &round_dir.join(&poster_name)) {
            poster_path = Some(format!("Reviews/{task_id}/r{round}/{poster_name}"));
        }
    }
    if kind == "video" && poster_path.is_none() {
        let poster_name = format!("{id}.poster.webp");
        let poster_file = round_dir.join(&poster_name);
        let extracted = std::process::Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-y", "-i", &entry.source,
                "-frames:v", "1", "-vf", "scale=512:-1"])
            .arg(&poster_file).status();
        if extracted.is_ok_and(|status| status.success()) && poster_file.is_file() {
            poster_path = Some(format!("Reviews/{task_id}/r{round}/{poster_name}"));
        }
    }
    if kind == "url" {
        if poster_path.is_none() { return Err(TaskStoreError::Invalid("URL artifacts require --poster".into())); }
        return Ok(ReviewArtifact { id, label: entry.label.clone(), kind, storage: "url".into(),
            path: entry.source.clone(), pruned: false, poster: poster_path });
    }
    let source = Path::new(&entry.source);
    let size = path_size(source)?;
    let use_local = size > MAX_SYNC_FILE;
    if use_local && poster_path.is_none() {
        return Err(TaskStoreError::Invalid(format!("artifact {id} exceeds 25 MB; supply --poster for local-only media")));
    }
    if kind == "video" && poster_path.is_none() {
        return Err(TaskStoreError::Invalid("video poster extraction failed or ffmpeg unavailable; supply --poster".into()));
    }
    let destination = if use_local { local_cache(root, task_id, round)? } else { round_dir };
    let target = destination.join(format!("{id}-{name}"));
    copy_path(source, &target)?;
    Ok(ReviewArtifact { id, label: entry.label.clone(), kind,
        storage: if use_local { "local" } else { "synced" }.into(),
        path: target.to_string_lossy().to_string(), pruned: false, poster: poster_path })
}
fn append_timeline(description: &str, note: &str) -> String {
    let stamp = now();
    let quoted = note.lines().map(|line| if line.is_empty() {
        ">".into()
    } else {
        format!("> {line}")
    }).collect::<Vec<String>>().join("\n");
    format!("{}\n\n<!-- taskboard:update:user-note|{stamp} -->\n{quoted}\n<!-- taskboard:end-update -->",
        description.trim_end())
}
fn find_review_task(snapshot: &BoardSnapshot, id: &str) -> Result<Task, TaskStoreError> {
    snapshot.tasks.iter().find(|task| task.id == id && !task.archived &&
        task.kind != TaskKind::Epic).cloned()
        .ok_or_else(|| TaskStoreError::NotFound(id.into()))
}
fn persist_task_review(root: &Path, task: &Task) -> Result<Task, TaskStoreError> {
    let path = find_task_path(root, &task.id)?;
    let existing = fs::read_to_string(&path)?;
    if revision(&existing) != task.revision {
        return Err(TaskStoreError::Conflict(format!("{} changed on disk", task.id)));
    }
    let text = render_task(task, Some(&existing))?;
    write_text_atomic(&path, &text)?;
    let mut saved = task.clone();
    saved.revision = revision(&text);
    Ok(saved)
}
fn ensure_expected(received: &str, expected: &str, entity: &str) -> Result<(), TaskStoreError> {
    if received != expected {
        return Err(TaskStoreError::Conflict(format!("{entity} changed since the review was loaded")));
    }
    Ok(())
}

impl FsTaskStore {
    /// Recreate missing HTML queue posters without overwriting current verdicts.
    /// Uses the same board mutation lock as review request/resolve.
    pub fn review_capture_thumbnails(&self, root: &Path, task_id: &str) -> Result<ReviewManifest, TaskStoreError> {
        let _lock = acquire_lock(root)?;
        if !valid_id(task_id) { return Err(TaskStoreError::Invalid("invalid review task ID".into())); }
        let file = manifest_file(root, task_id);
        let mut manifest = parse_manifest(&file)?;
        let mut changed = false;
        for round in &mut manifest.rounds {
            for artifact in &mut round.artifacts {
                if artifact.kind != "html" || artifact.poster.is_some() || artifact.storage == "url" {
                    continue;
                }
                let dest = root.join("Reviews").join(task_id).join(format!("r{}", round.number))
                    .join(format!("{}.poster.png", artifact.id));
                if dest.metadata().is_ok_and(|meta| meta.len() > 0) ||
                    capture_html_poster(Path::new(&artifact.path), &dest) {
                    artifact.poster = Some(format!("Reviews/{task_id}/r{}/{}.poster.png",
                        round.number, artifact.id));
                    changed = true;
                }
            }
        }
        if changed { save_manifest(&file, &manifest)?; }
        parse_manifest(&file)
    }

    pub fn review_show(&self, root: &Path, task_id: &str) -> Result<ReviewManifest, TaskStoreError> {
        let _lock = acquire_lock(root)?;
        parse_manifest(&manifest_file(root, task_id))
    }

    pub fn review_request(&self, root: &Path, input: ReviewRequest) -> Result<ReviewManifest, TaskStoreError> {
        let _lock = acquire_lock(root)?;
        if !valid_id(&input.task_id) { return Err(TaskStoreError::Invalid("invalid review task ID".into())); }
        let snapshot = self.load_unlocked(root)?;
        let mut task = find_review_task(&snapshot, &input.task_id)?;
        ensure_expected(input.expected_task_revision.as_deref().unwrap_or(""), &task.revision, &task.id)?;
        let mode = input.mode.as_str();
        if !matches!(mode, "pick" | "approve" | "rank" | "markup") {
            return Err(TaskStoreError::Invalid("review mode must be pick, approve, rank, or markup".into()));
        }
        if input.judge.trim().is_empty() {
            return Err(TaskStoreError::Invalid("review --judge is required".into()));
        }
        if input.judge.trim().chars().count() > 300 {
            return Err(TaskStoreError::Invalid("Inbox review question exceeds 300 characters. Make it shorter, human-readable and actionable; put background details in the task description.".into()));
        }
        let count = input.artifacts.len();
        if (mode == "pick" && !(2..=4).contains(&count)) ||
            (mode == "markup" && count != 1) ||
            ((mode == "approve" || mode == "rank") && count == 0) {
            return Err(TaskStoreError::Invalid("invalid number of artifacts for the review mode".into()));
        }
        // A single new design can be approved on its own. Previous is optional;
        // when supplied, the inspector also enables before/after comparison.
        let top_k = input.top_k.unwrap_or(1);
        if top_k == 0 || top_k > count { return Err(TaskStoreError::Invalid("top-K must be 1..artifact count".into())); }
        let file = manifest_file(root, &input.task_id);
        let mut manifest = if file.is_file() {
            parse_manifest(&file)?
        } else {
            ReviewManifest { schema: 1, task_id: input.task_id.clone(),
                status: "open".into(), current_round: 0, rounds: Vec::new(),
                implemented_by: None, openrig_gate: None, archived_at: None,
                receipt_prune_at: None, revision: String::new() }
        };
        ensure_expected(input.expected_manifest_revision.as_deref().unwrap_or(""),
            &manifest.revision, "review manifest")?;
        if manifest.status == "open" && manifest.current_round > 0
            && manifest.rounds.last().is_some_and(|round| round.result.is_none()) {
            return Err(TaskStoreError::Invalid("finish or supersede the open round before requesting another".into()));
        }
        let number = manifest.current_round + 1;
        let mut artifacts = Vec::new();
        for (index, artifact) in input.artifacts.iter().enumerate() {
            artifacts.push(copy_artifact(root, &task.id, number, index, artifact, input.poster.as_deref())?);
        }
        let previous = if let Some(previous) = &input.previous {
            Some(copy_artifact(root, &task.id, number, 23, previous, input.poster.as_deref())?)
        } else { None };
        let mut references = Vec::new();
        for (index, reference) in input.references.iter().enumerate() {
            references.push(copy_artifact(root, &task.id, number, index + 10,
                reference, input.poster.as_deref())?);
        }
        manifest.rounds.push(ReviewRound {
            number, mode: mode.to_string(), judge: input.judge,
            criteria: input.criteria, artifacts, references, previous, top_k,
            requested_at: now(), result: None, loser_prune_at: None,
        });
        manifest.current_round = number;
        manifest.status = "open".into();
        manifest.implemented_by = input.implemented_by.or(manifest.implemented_by);
        manifest.openrig_gate = input.openrig_gate.or(manifest.openrig_gate);
        task.review_status = Some("open".into());
        task.review_round = Some(number);
        if !task.attention.contains(&TaskAttention::Question) {
            task.attention.push(TaskAttention::Question);
        }
        task.question = Some(TaskQuestion {
            request_type: None,
            prompt: manifest.rounds.last().unwrap().judge.clone(),
            options: Vec::new(), answer: None,
        });
        save_manifest(&file, &manifest)?;
        persist_task_review(root, &task)?;
        parse_manifest(&file)
    }

    /// Start a new human decision round without rewriting the prior verdict or
    /// requiring media that may have been pruned or moved since the first round.
    pub fn review_reconsider(&self, root: &Path, input: ReviewReconsider) -> Result<ReviewManifest, TaskStoreError> {
        let _lock = acquire_lock(root)?;
        if !valid_id(&input.task_id) { return Err(TaskStoreError::Invalid("invalid review task ID".into())); }
        let snapshot = self.load_unlocked(root)?;
        let mut task = find_review_task(&snapshot, &input.task_id)?;
        ensure_expected(&input.expected_task_revision, &task.revision, &task.id)?;
        let file = manifest_file(root, &task.id);
        let mut manifest = parse_manifest(&file)?;
        ensure_expected(&input.expected_manifest_revision, &manifest.revision, "review manifest")?;
        if manifest.status != "open" {
            return Err(TaskStoreError::Invalid("only an open review can be reconsidered".into()));
        }
        let prior = manifest.rounds.last().ok_or_else(||
            TaskStoreError::Invalid("review has no round".into()))?;
        if !prior.result.as_ref().is_some_and(|result|
            result.verdict == "rejected" || result.verdict == "more-variants") {
            return Err(TaskStoreError::Invalid("only a rejected or more-variants round can be reconsidered".into()));
        }
        let mut next = prior.clone();
        next.number = manifest.current_round + 1;
        next.requested_at = now();
        next.result = None;
        next.loser_prune_at = None;
        manifest.current_round = next.number;
        task.review_round = Some(next.number);
        task.review_status = Some("open".into());
        if !task.attention.contains(&TaskAttention::Question) {
            task.attention.push(TaskAttention::Question);
        }
        task.question = Some(TaskQuestion {
            request_type: None, prompt: next.judge.clone(), options: Vec::new(), answer: None,
        });
        manifest.rounds.push(next);
        save_manifest(&file, &manifest)?;
        persist_task_review(root, &task)?;
        parse_manifest(&file)
    }

    pub fn review_resolve(&self, root: &Path, input: ReviewResolution) -> Result<ReviewManifest, TaskStoreError> {
        let _lock = acquire_lock(root)?;
        if !valid_id(&input.task_id) { return Err(TaskStoreError::Invalid("invalid review task ID".into())); }
        let snapshot = self.load_unlocked(root)?;
        let mut task = find_review_task(&snapshot, &input.task_id)?;
        ensure_expected(&input.expected_task_revision, &task.revision, &task.id)?;
        let file = manifest_file(root, &task.id);
        let mut manifest = parse_manifest(&file)?;
        ensure_expected(&input.expected_manifest_revision, &manifest.revision, "review manifest")?;
        if manifest.status != "open" { return Err(TaskStoreError::Invalid("review is not open".into())); }
        let round = manifest.rounds.last_mut().ok_or_else(||
            TaskStoreError::Invalid("review has no round".into()))?;
        if round.result.is_some() { return Err(TaskStoreError::Invalid("current round already resolved".into())); }
        if !matches!(input.verdict.as_str(), "picked" | "approved" | "rejected" | "ranked" | "marked" | "more-variants") {
            return Err(TaskStoreError::Invalid("invalid review verdict".into()));
        }
        let ids: HashSet<&str> = round.artifacts.iter().map(|a| a.id.as_str()).collect();
        if input.choices.iter().any(|id| !ids.contains(id.as_str())) ||
            input.ranking.iter().any(|id| !ids.contains(id.as_str())) {
            return Err(TaskStoreError::Invalid("review answer references unknown artifact".into()));
        }
        if input.pins.iter().any(|pin| !(0.0..=1.0).contains(&pin.x) ||
            !(0.0..=1.0).contains(&pin.y) || pin.timestamp.is_some_and(|v| v < 0.0)) {
            return Err(TaskStoreError::Invalid("pins must use normalized x/y and valid timestamps".into()));
        }
        if (input.verdict == "picked" && (round.mode != "pick" || input.choices.len() != 1)) ||
            (input.verdict == "ranked" && (round.mode != "rank" || input.ranking.len() != round.artifacts.len())) ||
            (input.verdict == "approved" && (round.mode != "approve" || input.choices.len() != 1)) ||
            (input.verdict == "marked" && round.mode != "markup") ||
            (input.ranking.len() != input.ranking.iter().collect::<HashSet<_>>().len()) {
            return Err(TaskStoreError::Invalid("review verdict requires a valid choice or full ranking".into()));
        }
        let date = now();
        round.result = Some(ReviewVerdict {
            verdict: input.verdict.clone(), choices: input.choices.clone(), ranking: input.ranking.clone(),
            pins: input.pins.clone(), note: input.note.clone(), resolved_at: date.clone(),
        });
        round.loser_prune_at = Some((Utc::now() + ChronoDuration::hours(48))
            .to_rfc3339_opts(SecondsFormat::Millis, true));
        // Rejection and request for more variants keep the human/agent review
        // conversation open; a later request starts the next round.
        manifest.status = if input.verdict == "more-variants" || input.verdict == "rejected" {
            "open"
        } else {
            "resolved"
        }.into();
        task.review_status = Some(manifest.status.clone());
        // Human rejection makes the original implementation actionable again.
        // The review conversation remains open for the next artifact revision.
        if task.kind == TaskKind::Task &&
            (input.verdict == "rejected" || input.verdict == "more-variants") {
            let technically_blocked = task.column == BLOCKED_COLUMN_ID ||
                task.attention.contains(&TaskAttention::Blocked);
            let was_completed = snapshot.config.columns.iter()
                .any(|column| column.id == task.column && column.terminal);
            if !technically_blocked && (was_completed || task.claimed_by.is_none()) {
                task.column = AI_READY_COLUMN_ID.to_string();
                if was_completed { task.claimed_by = None; }
                apply_completion_state(&snapshot.config, &mut task);
            }
            if !task.attention.contains(&TaskAttention::UserNote) {
                task.attention.push(TaskAttention::UserNote);
            }
        }
        if manifest.status == "resolved" {
            task.attention.retain(|flag| *flag != TaskAttention::Question);
            task.question = None;
        } else if !task.attention.contains(&TaskAttention::Question) {
            task.attention.push(TaskAttention::Question);
        }
        let summary = format!("Review round {}: {}. Choice: [{}]. Ranking: [{}]. Pins: {}. Note: {}",
            round.number, input.verdict, input.choices.join(", "),
            input.ranking.join(", "), serde_json::to_string(&input.pins).unwrap_or_default(), input.note);
        task.description = append_timeline(&task.description, &summary);
        save_manifest(&file, &manifest)?;
        persist_task_review(root, &task)?;
        parse_manifest(&file)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewGcItem {
    pub path: String,
    pub reason: String,
    pub action: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewGcReport {
    pub dry_run: bool,
    pub board: String,
    pub actions: Vec<ReviewGcItem>,
    pub stale_open: Vec<String>,
    pub warnings: Vec<String>,
    pub synced_bytes: u64,
}

fn media_path(root: &Path, value: &str) -> PathBuf {
    let path = PathBuf::from(value);
    if path.is_absolute() { path } else { root.join(value) }
}
fn timestamp(value: &str) -> Option<chrono::DateTime<Utc>> {
    chrono::DateTime::parse_from_rfc3339(value).ok().map(|time| time.with_timezone(&Utc))
}
fn move_to_recycle(path: &Path) -> Result<(), TaskStoreError> {
    if !path.exists() { return Ok(()); }
    #[cfg(windows)]
    {
        let script = if path.is_dir() {
            r#"Add-Type -AssemblyName Microsoft.VisualBasic; [Microsoft.VisualBasic.FileIO.FileSystem]::DeleteDirectory($env:TASKBOARD_GC_PATH,[Microsoft.VisualBasic.FileIO.UIOption]::OnlyErrorDialogs,[Microsoft.VisualBasic.FileIO.RecycleOption]::SendToRecycleBin)"#
        } else {
            r#"Add-Type -AssemblyName Microsoft.VisualBasic; [Microsoft.VisualBasic.FileIO.FileSystem]::DeleteFile($env:TASKBOARD_GC_PATH,[Microsoft.VisualBasic.FileIO.UIOption]::OnlyErrorDialogs,[Microsoft.VisualBasic.FileIO.RecycleOption]::SendToRecycleBin)"#
        };
        use std::os::windows::process::CommandExt;
        let status = std::process::Command::new("powershell.exe")
            .creation_flags(0x08000000) // CREATE_NO_WINDOW: do not flash a console.
            .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command", script])
            .env("TASKBOARD_GC_PATH", path).status()?;
        if !status.success() || path.exists() {
            return Err(TaskStoreError::Invalid(format!("Recycle Bin move failed: {}", path.display())));
        }
        Ok(())
    }
    #[cfg(not(windows))]
    { let _ = path; Err(TaskStoreError::Invalid("Review GC deletion requires Windows Recycle Bin".into())) }
}
fn save_receipt(root: &Path, manifest: &ReviewManifest) -> Result<bool, TaskStoreError> {
    let folder = root.join("Reviews").join(&manifest.task_id);
    let receipt = folder.join("receipt.webp");
    if receipt.metadata().is_ok_and(|m| m.len() <= 200 * 1024 && m.len() > 0) {
        return Ok(true);
    }
    let winner = manifest.rounds.iter().rev().find_map(|round| {
        let result = round.result.as_ref()?;
        let ids: Vec<&String> = if result.verdict == "ranked" {
            result.ranking.iter().take(round.top_k).collect()
        } else if matches!(result.verdict.as_str(), "picked" | "approved") {
            result.choices.iter().collect()
        } else if result.verdict == "marked" {
            round.artifacts.iter().map(|a| &a.id).collect()
        } else { Vec::new() };
        ids.first().and_then(|id| round.artifacts.iter().find(|a| &a.id == *id))
    });
    let Some(artifact) = winner else { return Ok(false) };
    let input = if let Some(poster) = &artifact.poster {
        media_path(root, poster)
    } else if artifact.kind == "image" || artifact.kind == "video" {
        media_path(root, &artifact.path)
    } else { return Ok(false) };
    if !input.is_file() { return Ok(false); }
    for (width, quality) in [(480, 60), (320, 50), (240, 40)] {
        let temp = folder.join("receipt.tmp.webp");
        let status = std::process::Command::new("ffmpeg")
            .args(["-nostdin", "-hide_banner", "-loglevel", "error", "-y", "-i"])
            .arg(&input)
            .args(["-frames:v", "1", "-vf", &format!("scale={width}:-1"),
                "-c:v", "libwebp", "-quality", &quality.to_string(), "-compression_level", "6"])
            .arg(&temp).status();
        if status.is_ok_and(|s| s.success()) && temp.metadata().is_ok_and(|m| m.len() <= 200 * 1024 && m.len() > 0) {
            fs::rename(&temp, receipt)?;
            return Ok(true);
        }
        let _ = fs::remove_file(temp);
    }
    Ok(false)
}
pub(super) fn stale_open_review_ids(root: &Path) -> Vec<String> {
    let mut stale = Vec::new();
    let reviews = root.join("Reviews");
    if let Ok(entries) = fs::read_dir(reviews) {
        for entry in entries.flatten() {
            let path = entry.path().join("manifest.json");
            if !path.is_file() { continue; }
            if let Ok(manifest) = parse_manifest(&path) {
                if manifest.status == "open" && manifest.rounds.last()
                    .and_then(|round| timestamp(&round.requested_at))
                    .is_some_and(|at| at + ChronoDuration::days(30) <= Utc::now()) {
                    stale.push(manifest.task_id);
                }
            }
        }
    }
    stale
}

pub(super) fn measure_review_bytes(root: &Path) -> u64 {
    fn walk(path: &Path) -> u64 {
        if path.is_file() { return fs::metadata(path).map(|m| m.len()).unwrap_or_default(); }
        fs::read_dir(path).into_iter().flatten().flatten()
            .map(|item| walk(&item.path())).sum()
    }
    walk(&root.join("Reviews"))
}

impl FsTaskStore {
    pub fn review_gc(&self, root: &Path, dry_run: bool) -> Result<ReviewGcReport, TaskStoreError> {
        let _lock = acquire_lock(root)?;
        let snapshot = self.load_unlocked(root)?;
        let reviews = root.join("Reviews");
        let synced_bytes = measure_review_bytes(root);
        let mut result = ReviewGcReport {
            dry_run, board: root.to_string_lossy().to_string(),
            actions: Vec::new(), stale_open: Vec::new(),
            warnings: if synced_bytes > 1024 * 1024 * 1024 {
                vec![format!("Review sync data exceeds 1 GB: {synced_bytes} bytes")]
            } else { Vec::new() },
            synced_bytes,
        };
        if !reviews.is_dir() { return Ok(result); }
        let clock = Utc::now();
        let mut gc_log = Vec::new();
        for entry in fs::read_dir(&reviews)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() { continue; }
            let task_id = entry.file_name().to_string_lossy().to_string();
            if !valid_id(&task_id) { continue; }
            let manifest_file = entry.path().join("manifest.json");
            if !manifest_file.is_file() { continue; }
            let mut manifest = match parse_manifest(&manifest_file) {
                Ok(m) => m,
                Err(error) => { result.warnings.push(format!("{task_id}: {error}")); continue; }
            };
            if manifest.task_id != task_id {
                result.warnings.push(format!("{task_id}: manifest ID mismatch; skipped"));
                continue;
            }
            let implementing = manifest.implemented_by.as_deref().unwrap_or(&task_id);
            let task_archived = snapshot.tasks.iter().find(|t| t.id == implementing)
                .is_some_and(|t| t.archived);
            let mut changed = false;
            if task_archived && manifest.archived_at.is_none() {
                manifest.archived_at = Some(now());
                manifest.receipt_prune_at = Some((clock + ChronoDuration::days(90))
                    .to_rfc3339_opts(SecondsFormat::Millis, true));
                changed = true;
            } else if !task_archived && manifest.archived_at.is_some() {
                manifest.archived_at = None;
                manifest.receipt_prune_at = None;
                changed = true;
            }
            if manifest.status == "open" && manifest.rounds.last()
                .and_then(|r| timestamp(&r.requested_at))
                .is_some_and(|time| time + ChronoDuration::days(30) <= clock) {
                result.stale_open.push(task_id.clone());
            }

            // After 90 days, prune final receipt + manifest. No other task files are touched.
            if manifest.receipt_prune_at.as_deref().and_then(timestamp)
                .is_some_and(|time| time <= clock) {
                for path in [entry.path().join("receipt.webp"), manifest_file.clone()] {
                    if path.exists() {
                        result.actions.push(ReviewGcItem {
                            path: path.to_string_lossy().to_string(),
                            reason: "archive retention: 90 days".into(),
                            action: "recycle".into(),
                        });
                        if !dry_run { move_to_recycle(&path)?;
                            gc_log.push(format!("{}\trecycle\t{}\t90-day archive retention", now(), path.display())); }
                    }
                }
                continue;
            }
            if task_archived && manifest.archived_at.is_some() {
                let receipt = entry.path().join("receipt.webp");
                let existing = receipt.metadata().is_ok_and(|meta| meta.len() > 0 && meta.len() <= 200 * 1024);
                if !dry_run && !save_receipt(root, &manifest)? {
                    result.warnings.push(format!("{task_id}: cannot make ≤200KB winner receipt, preserving media"));
                    if changed { save_manifest(&manifest_file, &manifest)?; }
                    continue;
                }
                if !existing {
                    result.actions.push(ReviewGcItem {
                        path: receipt.to_string_lossy().to_string(),
                        reason: "create ≤200KB winner receipt before pruning".into(),
                        action: "receipt".into(),
                    });
                    if !dry_run {
                        gc_log.push(format!("{}\treceipt\t{}\tarchived winner", now(), receipt.display()));
                    }
                }
            }
            for round in &mut manifest.rounds {
                let eligible = task_archived || round.loser_prune_at.as_deref()
                    .and_then(timestamp).is_some_and(|time| time <= clock);
                if !eligible { continue; }
                let chosen: HashSet<String> = if task_archived { HashSet::new() }
                else if let Some(verdict) = &round.result {
                    if round.number != manifest.current_round { HashSet::new() }
                    else if verdict.verdict == "ranked" {
                        verdict.ranking.iter().take(round.top_k).cloned().collect()
                    } else if matches!(verdict.verdict.as_str(), "picked" | "approved") {
                        verdict.choices.iter().cloned().collect()
                    } else if verdict.verdict == "marked" {
                        round.artifacts.iter().map(|a| a.id.clone()).collect()
                    } else { HashSet::new() }
                } else { HashSet::new() };
                for artifact in &mut round.artifacts {
                    if artifact.pruned || (!task_archived && chosen.contains(artifact.id.as_str())) { continue; }
                    for raw in [Some(artifact.path.as_str()), artifact.poster.as_deref()].into_iter().flatten() {
                        if artifact.storage == "url" && raw == artifact.path { continue; }
                        let path = media_path(root, raw);
                        if !path.exists() { continue; }
                        // Refuse to prune outside canonical review and owned cache roots.
                        let review_root = reviews.canonicalize().unwrap_or_else(|_| reviews.clone());
                        let allowed = path.canonicalize().ok().is_some_and(|canonical| {
                            canonical.starts_with(&review_root) ||
                            local_cache(root, &task_id, round.number).ok()
                                .and_then(|cache| cache.parent().and_then(|p| p.canonicalize().ok()))
                                .is_some_and(|cache| canonical.starts_with(cache))
                        });
                        if !allowed { result.warnings.push(format!("Unsafe review path not pruned: {}", path.display())); continue; }
                        result.actions.push(ReviewGcItem { path: path.display().to_string(),
                            reason: if task_archived { "implementing task archived" }
                                else { "48h loser / superseded round" }.into(),
                            action: "recycle".into() });
                        if !dry_run {
                            move_to_recycle(&path)?;
                            gc_log.push(format!("{}\trecycle\t{}\treview media prune", now(), path.display()));
                        }
                    }
                    if !dry_run { artifact.pruned = true; changed = true; }
                }
            }
            if task_archived {
                let refs = entry.path().join("refs");
                if refs.exists() {
                    result.actions.push(ReviewGcItem { path: refs.display().to_string(),
                        reason: "task archived: prune references".into(), action: "recycle".into() });
                    if !dry_run { move_to_recycle(&refs)?;
                        gc_log.push(format!("{}\trecycle\t{}\tarchive references", now(), refs.display())); }
                }
            }
            if changed && !dry_run {
                let current = fs::read_to_string(&manifest_file)?;
                if revision(&current) != manifest.revision {
                    return Err(TaskStoreError::Conflict(format!("review manifest changed during sweep: {task_id}")));
                }
                save_manifest(&manifest_file, &manifest)?;
            }
        }
        // Local-only media that lost its source manifest is an orphan, not an
        // active review round. Give it seven full days before recycling.
        if let Ok(board_cache) = local_cache(root, "_lookup", 1) {
            if let Some(board_cache) = board_cache.parent().and_then(|p| p.parent()) {
                if board_cache.is_dir() {
                    for orphan in fs::read_dir(board_cache)? {
                        let orphan = orphan?;
                        if !orphan.file_type()?.is_dir() { continue; }
                        let id = orphan.file_name().to_string_lossy().to_string();
                        if !valid_id(&id) || manifest_file(root, &id).exists() { continue; }
                        let age = orphan.metadata()?.modified().ok()
                            .and_then(|time| time.elapsed().ok())
                            .unwrap_or_default();
                        if age < std::time::Duration::from_secs(7 * 24 * 60 * 60) { continue; }
                        result.actions.push(ReviewGcItem {
                            path: orphan.path().display().to_string(),
                            reason: "orphan local review cache older than seven days".into(),
                            action: "recycle".into(),
                        });
                        if !dry_run {
                            move_to_recycle(&orphan.path())?;
                            gc_log.push(format!("{}\trecycle\t{}\t7-day orphan cache", now(),
                                orphan.path().display()));
                        }
                    }
                }
            }
        }
        if !dry_run && !gc_log.is_empty() {
            let mut file = fs::OpenOptions::new().create(true).append(true).open(reviews.join("_gc.log"))?;
            for line in gc_log { writeln!(file, "{line}")?; }
        }
        Ok(result)
    }
}
