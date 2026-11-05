mod review_gate_link;

use clap::{Args, CommandFactory, Parser, Subcommand};
use serde_json::{json, Value};
use std::{collections::HashSet, fs, path::{Path, PathBuf}, process};
use task_core::{AI_READY_COLUMN_ID, BoardConfig, BoardSnapshot, Column, FsTaskStore, Task, TaskAttention, TaskDraft, TaskQuestion, TaskStore, TaskStoreError, ReviewInputArtifact, ReviewRequest, ReviewResolution, ReviewPin};

const WARTRIBES_BOARD: &str = r"B:\GDrive\SharedWithPhone\Obsidian\jjj\Wartribes\Tasks";
const TASK_CREATE_GUIDANCE: &str = r"C:\Users\crovea\.agents\docs\guidance\taskboard.md";

#[derive(Parser)]
#[command(name = "taskboard", version, about = "Agent-facing CLI for the shared Markdown TaskBoard")]
struct Cli {
    #[arg(long, global = true, default_value = WARTRIBES_BOARD)]
    root: PathBuf,
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Task {
        #[command(subcommand)]
        command: TaskCommand,
    },
    Column {
        #[command(subcommand)]
        command: ColumnCommand,
    },
    Release { #[command(subcommand)] command: ReleaseCommand },
}

#[derive(Subcommand)]
enum TaskCommand {
    List(TaskListArgs),
    Show(TaskTarget),
    Search(TaskSearchArgs),
    Create(TaskCreateArgs),
    Update(TaskUpdateArgs),
    Move(TaskMoveArgs),
    /// Preview or repair old tasks whose claim and column disagree.
    ReconcileClaims { #[arg(long)] apply: bool },
    Parent(TaskParentArgs),
    Complete(TaskTarget),
    Finish(TaskFinishArgs),
    AskUser(TaskAskUserArgs),
    CompleteManaged(TaskManagedCompleteArgs),
    Archive(TaskTarget),
    Claim(TaskClaimArgs),
    #[command(alias = "release_task")]
    Release(TaskClaimArgs),
    Execution(TaskExecutionCommandArgs),
    Queue { #[command(subcommand)] command: TaskQueueCommand },
    Live { #[command(subcommand)] command: TaskLiveCommand },
    Review { #[command(subcommand)] command: ReviewCommand },
}

#[derive(Args)]
struct TaskExecutionCommandArgs {
    target: String,
    #[command(subcommand)]
    action: ExecutionAction,
}

#[derive(Subcommand)]
enum ExecutionAction {
    Show,
    Set { mode: String, #[arg(long)] revision: String },
    StartNow { #[arg(long)] revision: String },
    Receipt { #[arg(long)] request: String, #[arg(long)] receipt: String },
}

#[derive(Subcommand)]
enum TaskQueueCommand {
    /// Structured, read-only list for the T3 dispatcher.
    List,
    Reserve(TaskQueueReserveArgs),
    Starting(TaskQueueReservationArgs),
    Working(TaskQueueWorkingArgs),
    /// Return a never-bootstrapped worker to Queue after its exact T3 session has closed.
    Reclaim(TaskQueueReclaimArgs),
    /// Release a reservation only after a definitive T3 not-created receipt.
    Failed(TaskQueueFailedArgs),
    /// Human Start now preempts a queue item only before it enters Starting.
    Preempt(TaskTarget),
}

#[derive(Args)]
struct TaskQueueReserveArgs {
    target: String,
    #[arg(long)] queue_request: String,
    #[arg(long)] request_id: String,
    #[arg(long, default_value_t = 180)] lease_seconds: u64,
}

#[derive(Args)]
struct TaskQueueReservationArgs {
    target: String,
    #[arg(long)] request_id: String,
}

#[derive(Args)]
struct TaskQueueWorkingArgs {
    target: String,
    #[arg(long)] queue_request: String,
    #[arg(long)] request_id: String,
    #[arg(long)] session_id: String,
}

#[derive(Args)]
struct TaskQueueReclaimArgs {
    target: String,
    #[arg(long)] queue_request: String,
    #[arg(long)] request_id: String,
    #[arg(long)] session_id: String,
    #[arg(long)] reason: String,
    #[arg(long = "confirmed-session-closed", required = true, action = clap::ArgAction::SetTrue)]
    confirmed_session_closed: bool,
}

#[derive(Args)]
struct TaskQueueFailedArgs {
    target: String,
    #[arg(long)] queue_request: String,
    #[arg(long)] request_id: String,
    #[arg(long)] reason: String,
    #[arg(long = "confirmed-not-created", required = true, action = clap::ArgAction::SetTrue)]
    confirmed_not_created: bool,
}

/// Persist live collaboration on the existing task; never creates or sends a session.
#[derive(Subcommand)]
enum TaskLiveCommand {
    WithMe { target: String, #[arg(long)] off: bool },
    Ready { target: String, #[arg(long)] brief: String, #[arg(long)] est: Option<u32> },
    Start { target: String },
    Later { target: String, #[arg(long)] until: String },
    Clear { target: String },
}

#[derive(Subcommand)]
enum ReviewCommand {
    Request(ReviewRequestArgs),
    Show(TaskTarget),
    Thumbnails(TaskTarget),
    Resolve(ReviewResolveArgs),
    Gc { #[arg(long)] dry_run: bool, #[arg(long)] all_boards: bool },
    ReconcileLinked { #[arg(long)] apply: bool },
}

#[derive(Args)]
struct ReviewRequestArgs {
    task: String,
    #[arg(long)] mode: String,
    #[arg(long)] judge: String,
    #[arg(long)] criteria: Vec<String>,
    #[arg(long, required = true)] artifact: Vec<String>,
    #[arg(long)] reference: Vec<String>,
    #[arg(long)] previous: Option<String>,
    #[arg(long)] poster: Option<String>,
    #[arg(long)] top: Option<usize>,
    #[arg(long)] implemented_by: Option<String>,
    #[arg(long)] openrig_gate: Option<String>,
}

#[derive(Args)]
struct ReviewResolveArgs {
    task: String,
    #[arg(long)] verdict: String,
    #[arg(long)] choice: Vec<String>,
    #[arg(long)] rank: Vec<String>,
    #[arg(long)] pins_json: Option<String>,
    #[arg(long, default_value = "")] note: String,
    /// Human review verdict only. Agents must never self-resolve review requests.
    #[arg(long)] human_confirmed: bool,
}

#[derive(Subcommand)]
enum ReleaseCommand {
    List,
    New,
    Show(TaskTarget),
    Enable { #[arg(action = clap::ArgAction::Set)] enabled: bool },
    Reconcile { #[arg(long)] historical: bool },
    BlockedCandidates,
    MigrateBlocked { task: String, #[arg(long)] validated: bool, #[arg(long)] evidence: String },
    Include { release: String, task: String },
    Exclude { release: String, task: String },
    Move { release: String, task: String, destination: String },
    Source { release: String, task: String, #[arg(long)] repo: String, #[arg(long)] commit: String, #[arg(long)] integrated_ref: Option<String>, #[arg(long)] evidence: Option<String> },
    Prepare { release: String, #[arg(long, default_value="")] evidence: String,
        #[arg(long)] run_id: Option<String> },
    Outcome { release: String, attempt: u32, status: String, #[arg(long)] evidence: String,
        #[arg(long)] run_id: Option<String> },
    Verify { release: String, attempt: u32, task: String,
        #[arg(long)] evidence: String, #[arg(long, default_value="verified")] status: String },
}

#[derive(Subcommand)]
enum ColumnCommand {
    List,
    Create(ColumnCreateArgs),
    Rename(ColumnRenameArgs),
    Reorder(ColumnReorderArgs),
    Ready(ColumnReadyArgs),
    Advisory(ColumnReadyArgs),
    Remove(ColumnTarget),
}

#[derive(Args)]
struct TaskTarget {
    target: String,
}

#[derive(Args)]
struct TaskListArgs {
    #[arg(long)]
    include_archived: bool,
    #[arg(long)]
    column: Option<String>,
    #[arg(long)]
    parent: Option<String>,
    #[arg(long)]
    roots: bool,
    #[arg(long)]
    ai_ready: bool,
    /// Optional live task state filter: ready or in_session.
    #[arg(long)]
    live_state: Option<String>,
}

#[derive(Args)]
struct TaskSearchArgs {
    query: String,
    #[arg(long)]
    include_archived: bool,
}

#[derive(Args)]
struct TaskCreateArgs {
    #[arg(long)]
    title: String,
    #[arg(long)]
    column: Option<String>,
    #[arg(long)]
    parent: Option<String>,
    #[arg(long)]
    category: Option<String>,
    #[arg(long)]
    icon: Option<String>,
    #[arg(long, default_value = "task")]
    kind: String,
    #[arg(long)] repeat_every_days: Option<u32>,
    #[arg(long)] next_due: Option<String>,
    #[arg(long = "depends-on")]
    depends_on: Vec<String>,
    #[arg(long = "related-to")]
    related_to: Vec<String>,
    #[arg(long, default_value = "")]
    description: String,
    #[arg(long = "acceptance")]
    acceptance_criteria: Vec<String>,
    #[arg(long, default_value = "")]
    ai_notes: String,
    #[arg(long, default_value = "")]
    user_notes: String,
    #[arg(long = "attention")]
    attention: Vec<String>,
    #[arg(long)]
    question: Option<String>,
    #[arg(long = "option")]
    options: Vec<String>,
    /// Concise reason; required when creating directly in Blocked or with the blocked flag.
    #[arg(long)]
    blocked_reason: Option<String>,
    #[arg(long="confirm-genuine-block", hide=true)] confirm_genuine_block: Option<String>,
}

#[derive(Args)]
struct TaskUpdateArgs {
    target: String,
    #[arg(long)]
    title: Option<String>,
    #[arg(long)]
    description: Option<String>,
    #[arg(long)]
    category: Option<String>,
    #[arg(long)]
    clear_category: bool,
    #[arg(long)]
    icon: Option<String>,
    #[arg(long)]
    clear_icon: bool,
    #[arg(long)]
    kind: Option<String>,
    #[arg(long)] repeat_every_days: Option<u32>,
    #[arg(long)] next_due: Option<String>,
    #[arg(long)] clear_recurrence: bool,
    #[arg(long = "depends-on")]
    depends_on: Vec<String>,
    #[arg(long)]
    clear_dependencies: bool,
    #[arg(long = "related-to")]
    related_to: Vec<String>,
    #[arg(long)]
    clear_related: bool,
    #[arg(long)]
    notes: Option<String>,
    #[arg(long)]
    ai_notes: Option<String>,
    #[arg(long)]
    clear_ai_notes: bool,
    #[arg(long)]
    user_notes: Option<String>,
    #[arg(long)]
    clear_user_notes: bool,
    #[arg(long = "add-attention")]
    add_attention: Vec<String>,
    #[arg(long = "remove-attention")]
    remove_attention: Vec<String>,
    #[arg(long)]
    question: Option<String>,
    #[arg(long = "option")]
    options: Vec<String>,
    #[arg(long)]
    clear_question: bool,
    #[arg(long = "acceptance")]
    acceptance_criteria: Vec<String>,
    #[arg(long)]
    clear_acceptance: bool,
    /// Concise, easy-to-read reason. Required before an agent marks a task
    /// blocked (blocked flag or Blocked column); recorded in the Description timeline.
    #[arg(long)]
    blocked_reason: Option<String>,
    #[arg(long="confirm-genuine-block", hide=true)] confirm_genuine_block: Option<String>,
    #[arg(long, action=clap::ArgAction::Set)] deployable: Option<bool>,
}

#[derive(Args)]
struct TaskMoveArgs {
    target: String,
    column: String,
    /// Required for agents entering In Progress; use the actual agent/session identity.
    #[arg(long)]
    agent: Option<String>,
    /// Create a token-bound managed claim when entering In Progress.
    #[arg(long)]
    managed: bool,
    /// Required when moving into the Blocked column.
    #[arg(long)]
    blocked_reason: Option<String>,
    #[arg(long="confirm-genuine-block", hide=true)] confirm_genuine_block: Option<String>,
}

#[derive(Args)]
struct TaskParentArgs {
    target: String,
    #[arg(long, conflicts_with = "clear")]
    parent: Option<String>,
    #[arg(long)]
    clear: bool,
}

#[derive(Args)]
struct TaskAskUserArgs {
    target: String,
    #[arg(long = "type")]
    request_type: String,
    #[arg(long)]
    question: String,
    #[arg(long = "option")]
    options: Vec<String>,
    #[arg(long = "artifact")]
    artifacts: Vec<String>,
    #[arg(long)]
    mode: Option<String>,
    #[arg(long)]
    previous: Option<String>,
    #[arg(long)]
    poster: Option<String>,
}

#[derive(Args)]
struct TaskFinishArgs {
    target: String,
    #[arg(long, default_value = "none")]
    verification: String,
    #[arg(long)] instructions: Option<String>,
}

#[derive(Args)]
struct TaskManagedCompleteArgs {
    target: String,
    #[arg(long)]
    agent: String,
    #[arg(long)]
    claim_token: String,
    #[arg(long)]
    expected_revision: String,
}

#[derive(Args)]
struct TaskClaimArgs {
    target: String,
    #[arg(long)]
    agent: String,
    /// Managed TaskBoard claim lifetime (not OpenRig claimant generation).
    #[arg(long)]
    managed: bool,
    /// Required for conditional managed release.
    #[arg(long)]
    claim_token: Option<String>,
}

#[derive(Args)]
struct ColumnTarget {
    target: String,
}

#[derive(Args)]
struct ColumnCreateArgs {
    #[arg(long)]
    name: String,
    #[arg(long)]
    id: Option<String>,
    #[arg(long)]
    terminal: bool,
}

#[derive(Args)]
struct ColumnRenameArgs {
    target: String,
    name: String,
}

#[derive(Args)]
struct ColumnReorderArgs {
    target: String,
    #[arg(help = "1-based position")]
    position: usize,
}

#[derive(Args)]
struct ColumnReadyArgs {
    target: String,
    #[arg(action = clap::ArgAction::Set, value_parser = clap::value_parser!(bool))]
    value: bool,
}

#[derive(Debug)]
struct CliError {
    code: &'static str,
    message: String,
    details: Value,
    exit: i32,
}

impl CliError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self { code, message: message.into(), details: Value::Null, exit: 2 }
    }

    fn details(mut self, details: Value) -> Self {
        self.details = details;
        self
    }

    fn exit(mut self, exit: i32) -> Self {
        self.exit = exit;
        self
    }
}

/// Run the same CLI from the console binary or the desktop executable.
pub fn entry() {
    // Task-wide guidance belongs to task help, not to the create command.
    // Keep create --help as a backward-compatible guidance entry point.
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let task_help = argv.windows(2).any(|part| {
        part[0] == "task" && ["help", "--help", "-h"].contains(&part[1].as_str())
    });
    let create_help = argv.windows(3).any(|part| {
        part[0] == "task" && part[1] == "create" && ["--help", "-h"].contains(&part[2].as_str())
    });
    if task_help || create_help {
        let mut command = Cli::command();
        let task = command.find_subcommand_mut("task")
            .expect("TaskBoard task subcommand");
        let text = if task_help {
            task.render_long_help().to_string()
        } else {
            task.find_subcommand_mut("create")
                .expect("TaskBoard create subcommand")
                .render_long_help().to_string()
        };
        println!("{text}\n");
        match fs::read_to_string(TASK_CREATE_GUIDANCE) {
            Ok(guidance) => println!("{guidance}"),
            Err(error) => {
                eprintln!("Cannot read {}: {error}", TASK_CREATE_GUIDANCE);
                process::exit(2);
            }
        }
        return;
    }
    let cli = Cli::parse();
    match run(&cli) {
        Ok(value) => print_success(cli.json, value),
        Err(error) => {
            print_error(cli.json, &error);
            process::exit(error.exit);
        }
    }
}

fn run(cli: &Cli) -> Result<Value, CliError> {
    let store = FsTaskStore::default();
    match &cli.command {
        Command::Task { command } => run_task(&store, &cli.root, command),
        Command::Column { command } => run_column(&store, &cli.root, command),
        Command::Release { command } => run_release(&store, &cli.root, command),
    }
}

/// Human acceptance and approval are Inbox work, not technical blockers.
fn validate_agent_block_reason(reason: Option<&str>) -> Result<(), CliError> {
    let reason = reason.unwrap_or("").to_ascii_lowercase();
    let human_gate = ["user verification", "human verification", "user to verify",
        "user verify", "waiting for user", "awaiting user", "approval required",
        "waiting for approval", "awaiting approval", "user approval",
        "human approval", "user sign-off", "user signoff"];
    if human_gate.iter().any(|phrase| reason.contains(phrase)) {
        return Err(CliError::new("human_input_not_blocker",
            "Do not Block a task solely for human approval/verification. Keep AI work actionable, request Inbox feedback on the original task, or finish implementation in Done with a pending verification request."));
    }
    Ok(())
}

/// Inbox questions must be short enough to understand and act on at a glance.
fn validate_inbox_question(question: &str) -> Result<(), CliError> {
    let count = question.trim().chars().count();
    if count > 300 {
        return Err(CliError::new("question_too_long", format!(
            "Inbox question is {count} characters (maximum 300). Shorten it to one human-readable, actionable question. Put background details in the task description, not the question."
        )));
    }
    Ok(())
}

fn run_ask_user(store: &FsTaskStore, root: &Path, args: &TaskAskUserArgs) -> Result<Value, CliError> {
    let snapshot = load_checked(store, root)?;
    let mut task = resolve_task(&snapshot, &args.target, false)?.clone();
    ensure_task_rewrite_safe(&task)?;
    if task.kind != task_core::TaskKind::Task || is_human_only(&task) {
        return Err(CliError::new("invalid_request_target", "Use the original AI-owned implementation task, not a For user card or epic"));
    }
    if args.question.trim().is_empty() {
        return Err(CliError::new("question_required", "Provide a concrete --question"));
    }
    validate_inbox_question(&args.question)?;
    if task.review_status.as_deref() == Some("open") ||
        task.question.as_ref().is_some_and(|q| q.answer.is_none()) {
        return Err(CliError::new("request_already_open", "Resolve the existing request instead of duplicating it"));
    }
    if args.request_type == "visual-review" {
        if args.artifacts.is_empty() {
            return Err(CliError::new("visual_artifact_required", "A visual review needs a screenshot, clip, or HTML preview; a source patch is insufficient"));
        }
        return run_task(store, root, &TaskCommand::Review { command: ReviewCommand::Request(ReviewRequestArgs {
            task: task.id.clone(), mode: args.mode.clone().unwrap_or_else(|| "approve".into()),
            judge: args.question.clone(), criteria: Vec::new(), artifact: args.artifacts.clone(),
            reference: Vec::new(), previous: args.previous.clone(), poster: args.poster.clone(),
            top: None, implemented_by: None, openrig_gate: None,
        }) });
    }
    if !args.artifacts.is_empty() || args.previous.is_some() || args.mode.is_some() || args.poster.is_some() {
        return Err(CliError::new("invalid_request", "Artifact options require --type visual-review"));
    }
    let options = match args.request_type.as_str() {
        "decision" if args.options.len() >= 2 => args.options.clone(),
        "decision" => return Err(CliError::new("options_required", "A decision requires at least two --option choices")),
        "verification" => vec!["Done".into(), "Needs changes".into()],
        "action" => args.options.clone(),
        _ => return Err(CliError::new("invalid_request_type", "Use decision, visual-review, verification, or action")),
    };
    task.question = Some(TaskQuestion { request_type: Some(args.request_type.clone()),
        prompt: args.question.trim().into(), options, answer: None });
    if !task.attention.contains(&TaskAttention::Question) { task.attention.push(TaskAttention::Question); }
    let saved = store.update(root, task).map_err(map_store_error)?;
    Ok(json!({ "task": saved, "request_type": args.request_type }))
}

fn run_finish(store: &FsTaskStore, root: &Path, args: &TaskFinishArgs) -> Result<Value, CliError> {
    let snapshot = load_checked(store, root)?;
    let mut task = resolve_task(&snapshot, &args.target, false)?.clone();
    ensure_task_rewrite_safe(&task)?;
    if is_human_only(&task) || task.kind != task_core::TaskKind::Task {
        return Err(CliError::new("not_executable", "Only ordinary AI-owned tasks can use task finish"));
    }
    if task.review_status.as_deref() == Some("open") ||
        task.question.as_ref().is_some_and(|q| q.answer.is_none()) {
        return Err(CliError::new("request_already_open", "An unresolved human request exists on this task"));
    }
    let done = snapshot.config.columns.iter().filter(|c| c.terminal).min_by_key(|c| c.order)
        .ok_or_else(|| CliError::new("invalid_column", "No Done column available"))?;
    match args.verification.as_str() {
        "none" => {},
        "user" => {
            let instructions = args.instructions.as_deref().unwrap_or("").trim();
            if instructions.is_empty() {
                return Err(CliError::new("verification_instructions_required", "Provide --instructions with concrete verification steps and deployed evidence"));
            }
            validate_inbox_question(instructions)?;
            task.question = Some(TaskQuestion { request_type: Some("verification".into()),
                prompt: instructions.into(), options: vec!["Done".into(), "Needs changes".into()], answer: None });
            if !task.attention.contains(&TaskAttention::Question) { task.attention.push(TaskAttention::Question); }
        }
        _ => return Err(CliError::new("invalid_verification", "Use --verification none or user")),
    }
    task.column = done.id.clone();
    task.claimed_by = None;
    let saved = store.update(root, task).map_err(map_store_error)?;
    Ok(json!({ "task": saved, "verification": args.verification }))
}


fn run_live(store: &FsTaskStore, root: &Path, command: &TaskLiveCommand) -> Result<Value, CliError> {
    use task_core::{LiveTaskState, TaskLive};
    let target = match command {
        TaskLiveCommand::WithMe { target, .. } |
        TaskLiveCommand::Ready { target, .. } |
        TaskLiveCommand::Start { target } |
        TaskLiveCommand::Later { target, .. } |
        TaskLiveCommand::Clear { target } => target,
    };
    let snapshot = load_checked(store, root)?;
    let mut task = resolve_task(&snapshot, target, false)?.clone();
    ensure_task_rewrite_safe(&task)?;
    if !matches!(task.kind, task_core::TaskKind::Task | task_core::TaskKind::Recurring)
        || task.archived || task.completed.is_some() {
        return Err(CliError::new("invalid_live_task", "Live collaboration requires an active agent task"));
    }
    // Legacy needs-user is a human-owned task, not a live collaboration request.
    if task.live.is_none() && task.attention.contains(&TaskAttention::NeedsUser) {
        return Err(CliError::new("for_user_only", "Cannot convert an existing For user task to live collaboration"));
    }
    let was_ready = task.live.as_ref().is_some_and(|live|
        matches!(live.state, Some(LiveTaskState::Ready)));
    let blank = || TaskLive {
        with_me: true, state: None, brief: None, est_minutes: None,
        ready_at: None, later_until: None,
    };
    match command {
        TaskLiveCommand::WithMe { off: true, .. } | TaskLiveCommand::Clear { .. } => {
            task.live = None;
        }
        TaskLiveCommand::WithMe { off: false, .. } => {
            task.live.get_or_insert_with(blank).with_me = true;
        }
        TaskLiveCommand::Ready { brief, est, .. } => {
            if brief.trim().is_empty() {
                return Err(CliError::new("brief_required", "Ready for you requires a nonempty brief"));
            }
            if est.is_some_and(|n| !(1..=1440).contains(&n)) {
                return Err(CliError::new("invalid_estimate", "--est must be between 1 and 1440 minutes"));
            }
            let live = task.live.get_or_insert_with(blank);
            live.with_me = true;
            live.state = Some(LiveTaskState::Ready);
            live.brief = Some(brief.trim().to_string());
            live.est_minutes = *est;
            live.ready_at = Some(chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true));
            live.later_until = None;
        }
        TaskLiveCommand::Start { .. } => {
            let live = task.live.get_or_insert_with(blank);
            live.with_me = true;
            live.state = Some(LiveTaskState::InSession);
            live.later_until = None;
        }
        TaskLiveCommand::Later { until, .. } => {
            let time = chrono::DateTime::parse_from_rfc3339(until)
                .map_err(|_| CliError::new("invalid_until", "--until must be an ISO 8601 time with timezone"))?;
            if time <= chrono::Utc::now() {
                return Err(CliError::new("invalid_until", "--until must be in the future"));
            }
            let Some(live) = task.live.as_mut() else {
                return Err(CliError::new("not_ready", "Set task live ready before Later"));
            };
            if !matches!(live.state, Some(LiveTaskState::Ready)) {
                return Err(CliError::new("not_ready", "Later is only available for Ready tasks"));
            }
            live.later_until = Some(time.to_rfc3339());
        }
    }
    let is_ready = task.live.as_ref().is_some_and(|live|
        matches!(live.state, Some(LiveTaskState::Ready)));
    if is_ready && !task.attention.contains(&TaskAttention::NeedsUser) {
        task.attention.push(TaskAttention::NeedsUser);
    } else if was_ready && !is_ready {
        task.attention.retain(|flag| *flag != TaskAttention::NeedsUser);
    }
    let saved = store.update(root, task).map_err(map_store_error)?;
    Ok(json!(saved))
}

fn run_task_queue(store: &FsTaskStore, root: &Path, command: &TaskQueueCommand) -> Result<Value, CliError> {
    use task_core::{ExecutionMode, QueueState, queue_task_ready, recurring_due};
    match command {
        TaskQueueCommand::List => {
            let snapshot = load_checked(store, root)?;
            let mut items = Vec::new();
            for task in snapshot.tasks.iter().filter(|task| !task.archived &&
                matches!(task.kind, task_core::TaskKind::Task | task_core::TaskKind::Recurring)) {
                let intent = store.execution_intent(root, &task.id).map_err(map_store_error)?;
                if intent.mode != ExecutionMode::Queue { continue; }
                let ready = queue_task_ready(&snapshot, task) && recurring_due(task);
                let legacy_qitem = intent.queue_request.as_ref()
                    .map(|request| format!("qitem-taskboard-{request}"));
                let legacy_openrig = (intent.queue_attempt.is_some() || intent.queue_receipt.is_some()) &&
                    intent.queue_retired_receipt.as_deref() != legacy_qitem.as_deref();
                let terminal = task.completed.is_some() || snapshot.config.columns.iter()
                    .any(|column| column.terminal && column.id == task.column);
                let state = if terminal { QueueState::Completed } else {
                    intent.queue_state.unwrap_or(QueueState::Queued)
                };
                let state = match state {
                    QueueState::Queued => "queued",
                    QueueState::Reserved => "reserved",
                    QueueState::Starting => "starting",
                    QueueState::Working => "working",
                    QueueState::Completed => "completed",
                };
                let priority = if task.attention.contains(&TaskAttention::Priority) { "urgent" } else { "routine" };
                items.push(json!({
                    "taskId": task.id,
                    "title": task.title,
                    "created": task.created,
                    "priority": priority,
                    "dependencies": task.depends_on,
                    "readiness": ready,
                    "readinessReason": if ready { Value::Null } else {
                        json!("Task is claimed, blocked, outside a green AI Ready column, waiting on dependencies, or not due")
                    },
                    "claimed": task.claimed_by.is_some(),
                    "claimedBy": task.claimed_by,
                    "queueRequest": intent.queue_request,
                    "state": state,
                    "reservationId": intent.queue_reservation_id,
                    "reservationExpiresAt": intent.queue_reservation_expires_at,
                    "sessionId": intent.queue_session_id,
                    "legacyOpenRig": legacy_openrig,
                    "legacyQueueAttempt": intent.queue_attempt,
                    "legacyQueueReceipt": intent.queue_receipt,
                    "error": intent.queue_error,
                }));
            }
            items.sort_by(|a,b| {
                let pa = a["priority"] == "urgent";
                let pb = b["priority"] == "urgent";
                pb.cmp(&pa).then_with(|| a["created"].as_str().cmp(&b["created"].as_str()))
            });
            Ok(json!({
                "schema": "taskboard-dispatch-queue/v1",
                "boardRoot": root.to_string_lossy(),
                "observedAt": chrono::Utc::now().to_rfc3339(),
                "items": items,
            }))
        }
        TaskQueueCommand::Reserve(args) => {
            let result = store.reserve_queue_task(root, &args.target, &args.queue_request,
                &args.request_id, args.lease_seconds).map_err(map_store_error)?;
            Ok(json!({"status":"reserved","taskId":args.target,"intent":result}))
        }
        TaskQueueCommand::Starting(args) => {
            let result = store.start_queue_task(root, &args.target, &args.request_id).map_err(map_store_error)?;
            Ok(json!({"status":"starting","taskId":args.target,"intent":result}))
        }
        TaskQueueCommand::Working(args) => {
            let result = store.claim_queue_working(root, &args.target, &args.queue_request,
                &args.request_id, &args.session_id).map_err(map_store_error)?;
            Ok(json!({"status":"working","taskId":args.target,"sessionId":args.session_id,"intent":result}))
        }
        TaskQueueCommand::Reclaim(args) => {
            if !args.confirmed_session_closed {
                return Err(CliError::new("confirmation_required", "T3 must confirm the exact abandoned session is durably closed"));
            }
            let result = store.reclaim_closed_queue_worker(root, &args.target,
                &args.queue_request, &args.request_id, &args.session_id, &args.reason)
                .map_err(map_store_error)?;
            Ok(json!({"status":"queued","taskId":args.target,"intent":result}))
        }
        TaskQueueCommand::Failed(args) => {
            if !args.confirmed_not_created {
                return Err(CliError::new("confirmation_required", "Only a definitive T3 not-created receipt may release a start reservation"));
            }
            let result = store.requeue_failed_start(root, &args.target, &args.queue_request,
                &args.request_id, &args.reason).map_err(map_store_error)?;
            Ok(json!({"status":"queued","taskId":args.target,"intent":result}))
        }
        TaskQueueCommand::Preempt(args) => {
            let snapshot = load_checked(store, root)?;
            let task = resolve_task(&snapshot, &args.target, false)?;
            store.prepare_manual_start(root, &task.id).map_err(map_store_error)?;
            Ok(json!({"status":"preempted","taskId":task.id}))
        }
    }
}

fn run_task(store: &FsTaskStore, root: &Path, command: &TaskCommand) -> Result<Value, CliError> {
    match command {
        TaskCommand::Review { command } => match command {
            ReviewCommand::Request(args) => {
                validate_inbox_question(&args.judge)?;
                let snapshot = load_checked(store, root)?;
                let task = resolve_task(&snapshot, &args.task, false)?;
                let existing = store.review_show(root, &task.id).ok();
                let parse = |value: &String| {
                    let (source, label) = value.rsplit_once('=').unwrap_or((value.as_str(), ""));
                    ReviewInputArtifact {
                        source: source.to_string(),
                        label: if label.is_empty() {
                            std::path::Path::new(source).file_name()
                                .and_then(|s| s.to_str()).unwrap_or(source).to_string()
                        } else { label.to_string() },
                    }
                };
                let review = store.review_request(root, ReviewRequest {
                    task_id: task.id.clone(), mode: args.mode.clone(), judge: args.judge.clone(),
                    criteria: args.criteria.clone(),
                    artifacts: args.artifact.iter().map(&parse).collect(),
                    references: args.reference.iter().map(&parse).collect(),
                    previous: args.previous.as_ref().map(&parse),
                    poster: args.poster.clone(), top_k: args.top,
                    implemented_by: args.implemented_by.clone(),
                    openrig_gate: args.openrig_gate.clone(),
                    expected_task_revision: Some(task.revision.clone()),
                    expected_manifest_revision: existing.map(|m| m.revision),
                }).map_err(map_store_error)?;
                let mut result = json!(review);
                if let Some(gate) = &args.openrig_gate {
                    result["openrig_link"] = review_gate_link::register(root, &task.id, &review.revision, gate, &task.title);
                }
                Ok(result)
            }
            ReviewCommand::Show(args) => {
                let snapshot = load_checked(store, root)?;
                let task = resolve_task(&snapshot, &args.target, true)?;
                let review = store.review_show(root, &task.id).map_err(map_store_error)?;
                Ok(json!({ "task": task, "review": review }))
            }
            ReviewCommand::Thumbnails(args) => {
                let manifest = store.review_capture_thumbnails(root, &args.target)
                    .map_err(map_store_error)?;
                Ok(json!(manifest))
            }
            ReviewCommand::Resolve(args) => {
                if !args.human_confirmed {
                    return Err(CliError::new("human_only", "Only the human reviewer may resolve a review. Human callers must pass --human-confirmed. Agents must never supply it."));
                }
                let snapshot = load_checked(store, root)?;
                let task = resolve_task(&snapshot, &args.task, false)?;
                let existing = store.review_show(root, &task.id).map_err(map_store_error)?;
                let pins: Vec<ReviewPin> = match &args.pins_json {
                    Some(value) => serde_json::from_str(value)
                        .map_err(|e| CliError::new("usage_error", format!("invalid pins JSON: {e}")))?,
                    None => Vec::new(),
                };
                let review = store.review_resolve(root, ReviewResolution {
                    task_id: task.id.clone(), verdict: args.verdict.clone(),
                    choices: args.choice.clone(), ranking: args.rank.clone(),
                    pins, note: args.note.clone(),
                    expected_task_revision: task.revision.clone(),
                    expected_manifest_revision: existing.revision,
                }).map_err(map_store_error)?;
                Ok(json!(review))
            }
            ReviewCommand::ReconcileLinked { apply } => {
                let entries = store.reconcile_completed_reviews(root, *apply).map_err(map_store_error)?;
                Ok(json!({"applied": apply, "count": entries.len(), "entries": entries}))
            }            ReviewCommand::Gc { dry_run, all_boards } => {
                let mut roots = vec![root.to_path_buf()];
                if *all_boards {
                    roots.clear();
                    let appdata = std::env::var_os("APPDATA")
                        .ok_or_else(|| CliError::new("io_error", "APPDATA is not defined"))?;
                    let config = PathBuf::from(appdata).join("TaskBoard").join("projects.json");
                    let catalog: Value = serde_json::from_slice(&fs::read(config)
                        .map_err(|e| CliError::new("io_error", e.to_string()))?)
                        .map_err(|e| CliError::new("invalid_board_task_data", e.to_string()))?;
                    for project in catalog["projects"].as_array().into_iter().flatten() {
                        if let Some(path) = project["path"].as_str() {
                            if Path::new(path).join("_board.md").exists() {
                                let board = PathBuf::from(path);
                                if !roots.iter().any(|existing| existing == &board) { roots.push(board); }
                            }
                        }
                    }
                    if roots.is_empty() {
                        return Err(CliError::new("io_error", "No mounted project boards found"));
                    }
                }
                let mut reports = Vec::new();
                for directory in roots {
                    reports.push(store.review_gc(&directory, *dry_run).map_err(map_store_error)?);
                }
                Ok(json!({ "dry_run": dry_run, "reports": reports }))
            },
        },
        TaskCommand::List(args) => {
            if args.live_state.as_ref().is_some_and(|state| !matches!(state.as_str(), "ready" | "in_session")) {
                return Err(CliError::new("usage_error", "--live-state must be ready or in_session"));
            }
            if args.roots && args.parent.is_some() {
                return Err(CliError::new("usage_error", "--roots cannot be combined with --parent"));
            }
            let snapshot = load_checked(store, root)?;
            let column_id = match &args.column {
                Some(value) => Some(resolve_column(&snapshot.config, value)?.id.clone()),
                None => None,
            };
            let parent_id = match &args.parent {
                Some(value) => Some(resolve_task(&snapshot, value, args.include_archived)?.id.clone()),
                None => None,
            };
            let ready_columns: HashSet<&str> = snapshot
                .config
                .columns
                .iter()
                .filter(|column| column.ai_ready)
                .map(|column| column.id.as_str())
                .collect();
            let mut tasks: Vec<&Task> = snapshot.tasks.iter()
                .filter(|task| args.include_archived || !task.archived)
                .filter(|task| args.live_state.as_ref().is_none_or(|state| task.live.as_ref().and_then(|live| live.state.as_ref())
                    .is_some_and(|value| matches!((state.as_str(), value),
                        ("ready", task_core::LiveTaskState::Ready) | ("in_session", task_core::LiveTaskState::InSession)))))
                .filter(|task| column_id.as_ref().is_none_or(|column| &task.column == column))
                .filter(|task| !args.ai_ready || (matches!(task.kind, task_core::TaskKind::Task | task_core::TaskKind::Recurring)
                    && ready_columns.contains(task.column.as_str())
                    && ensure_ready(&snapshot, task)))
                .filter(|task| {
                    if args.roots { task.parent.is_none() }
                    else { parent_id.as_ref().is_none_or(|parent| task.parent.as_ref() == Some(parent)) }
                })
                .collect();
            // Actionable priorities first for agent discovery; stable otherwise.
            tasks.sort_by_key(|task| if ensure_ready(&snapshot, task) && task.attention.contains(&TaskAttention::Priority) { 0 } else { 1 });
            Ok(json!({ "tasks": tasks, "advisories": board_advisories(&snapshot) }))
        }
        TaskCommand::Show(args) => {
            let mut snapshot = load_checked(store, root)?;
            let selected = resolve_task(&snapshot, &args.target, true)?;
            let unread_id = (!selected.agent_interacted).then(|| selected.id.clone());
            if let Some(id) = unread_id {
                let touched = store.record_agent_read(root, &id).map_err(map_store_error)?;
                if let Some(task) = snapshot.tasks.iter_mut().find(|task| task.id == id) {
                    *task = touched;
                }
            }
            let task = resolve_task(&snapshot, &args.target, true)?;
            let children: Vec<&Task> = snapshot.tasks.iter()
                .filter(|candidate| candidate.parent.as_deref() == Some(task.id.as_str()))
                .collect();
            let blockers = unfinished_dependencies(&snapshot, task);
            let blocked_by = snapshot.tasks.iter().filter(|other| other.depends_on.contains(&task.id)).map(|other| &other.id).collect::<Vec<_>>();
            let epic_status = if task.kind == task_core::TaskKind::Epic {
                let all = &snapshot.tasks;
                let mut descendants: Vec<&Task> = Vec::new();
                let mut pending = vec![task.id.as_str()];
                while let Some(parent) = pending.pop() {
                    for t in all.iter().filter(|t| t.parent.as_deref() == Some(parent)) {
                        if t.kind == task_core::TaskKind::Epic { pending.push(&t.id); }
                        else { descendants.push(t); }
                    }
                }
                let done = descendants.iter().filter(|t| t.archived || t.completed.is_some()).count();
                Some(json!({"completed":done,"total":descendants.len(),"remaining":descendants.len()-done}))
            } else { None };
            let (blocked_reason, blocked_reason_source) = effective_blocked_reason(task);
            Ok(with_work_advisory(json!({ "task": task, "children": children, "waiting_for": blockers, "blocks": blocked_by, "readiness": readiness(&snapshot, task), "blocked_reason": blocked_reason, "blocked_reason_source": blocked_reason_source, "epic_progress": epic_status, "handoff":task_core::handoff_bundle(task), "advisories": board_advisories(&snapshot) }), task))
        }
        TaskCommand::Search(args) => {
            let snapshot = load_checked(store, root)?;
            let needle = args.query.to_lowercase();
            let tasks: Vec<&Task> = snapshot.tasks.iter()
                .filter(|task| args.include_archived || !task.archived)
                .filter(|task| task_matches(task, &needle))
                .collect();
            Ok(json!({ "tasks": tasks }))
        }
        TaskCommand::Create(args) => {
            if let Some(question) = &args.question { validate_inbox_question(question)?; }
            let snapshot = load_checked(store, root)?;
            // A question about existing implementation belongs on that task,
            // not on a duplicate Review/Verify card with a new ID.
            if args.question.is_some() {
                if let Some(owner) = args.related_to.iter().find(|id| snapshot.tasks.iter().any(|t| t.id == **id && !t.archived)) {
                    return Err(CliError::new("duplicate_feedback_task",
                        format!("{owner} already owns this work. Use task update {owner} --question ... or task review request {owner} instead of creating another task.")));
                }
            }
            let mut kind = parse_kind(&args.kind)?;
            let mut attention = parse_attention_values(&args.attention)?;
            if attention.contains(&TaskAttention::NeedsUser) {
                if kind == task_core::TaskKind::Epic {
                    return Err(CliError::new("usage_error", "Epics cannot be For user tasks").exit(4));
                }
                kind = task_core::TaskKind::ForUser;
                attention.retain(|flag| flag != &TaskAttention::NeedsUser);
            }
            let column = match &args.column {
                Some(value) => resolve_column(&snapshot.config, value)?.id.clone(),
                None if kind == task_core::TaskKind::ForUser => "in-progress".to_string(),
                None => default_create_column(&snapshot.config)?.id.clone(),
            };
            let parent = match &args.parent {
                Some(value) => Some(resolve_task(&snapshot, value, false)?.id.clone()),
                None => None,
            };
            let question = args.question.as_ref().map(|prompt| TaskQuestion {
                request_type: None,
                prompt: prompt.trim().to_string(),
                options: args.options.clone(),
                answer: None,
            });
            if question.is_some() && !attention.contains(&TaskAttention::Question) {
                attention.push(TaskAttention::Question);
            }
            if kind != task_core::TaskKind::Epic && !args.user_notes.trim().is_empty() && !attention.contains(&TaskAttention::UserNote) {
                attention.push(TaskAttention::UserNote);
            }
            let draft = TaskDraft {
                reserved_id: None,
                reservation_token: None,
                title: args.title.clone(),
                column,
                parent,
                kind,
                deployable: None,
                repeat_every_days: args.repeat_every_days,
                next_due: args.next_due.clone(),
                depends_on: args.depends_on.clone(),
                related_to: args.related_to.clone(),
                category: args.category.clone().filter(|value| !value.trim().is_empty()),
                icon: args.icon.clone(),
                attention,
                question,
                blocked_reason: args.blocked_reason.clone(),
                description: args.description.clone(),
                acceptance_criteria: args.acceptance_criteria.clone(),
                ai_notes: args.ai_notes.clone(),
                user_notes: args.user_notes.clone(),
                notes: String::new(),
            };
            let mut task = store.create_as_confirmed(root, draft, task_core::MutationActor::Agent, args.confirm_genuine_block.as_deref()).map_err(map_store_error)?;
            if let Some(reason) = &args.blocked_reason {
                task.blocked_reason = Some(reason.clone());
                task = store.update(root, task).map_err(map_store_error)?;
            }
            Ok(json!(task))
        }
        TaskCommand::Update(args) => {
            let snapshot = load_checked(store, root)?;
            let mut task = resolve_task(&snapshot, &args.target, false)?.clone();
            ensure_task_rewrite_safe(&task)?;
            if let Some(kind) = &args.kind {
                let new_kind = parse_kind(kind)?;
                if is_human_only(&task) && new_kind != task.kind {
                    return Err(CliError::new("for_user_only", "Only the user can reclassify a For user task; change its type in TaskBoard").exit(4));
                }
                task.kind = new_kind;
            }
            if args.clear_recurrence { task.repeat_every_days = None; task.next_due = None; }
            if let Some(days) = args.repeat_every_days { task.repeat_every_days = Some(days); }
            if let Some(due) = &args.next_due { task.next_due = Some(due.clone()); }
            if args.clear_dependencies { task.depends_on.clear(); }
            else if !args.depends_on.is_empty() { task.depends_on = args.depends_on.clone(); }
            if args.clear_related { task.related_to.clear(); }
            else if !args.related_to.is_empty() { task.related_to = args.related_to.clone(); }
            if let Some(title) = &args.title { task.title = title.clone(); }
            if let Some(description) = &args.description { task.description = description.clone(); }
            if args.clear_category {
                task.category = None;
            } else if let Some(category) = &args.category {
                task.category = Some(category.trim().to_string()).filter(|value| !value.is_empty());
            }
            if args.clear_icon {
                task.icon = None;
            } else if let Some(icon) = &args.icon {
                task.icon = Some(icon.clone());
            }
            if let Some(notes) = &args.notes { task.notes = notes.clone(); }
            if args.clear_ai_notes {
                task.ai_notes.clear();
            } else if let Some(ai_notes) = &args.ai_notes {
                task.ai_notes = ai_notes.clone();
            }
            if args.clear_user_notes {
                task.user_notes.clear();
                task.attention.retain(|flag| flag != &TaskAttention::UserNote);
            } else if let Some(user_notes) = &args.user_notes {
                task.user_notes = user_notes.clone();
                task.attention.retain(|flag| flag != &TaskAttention::UserNote);
                if task.kind != task_core::TaskKind::Epic && !user_notes.trim().is_empty() {
                    task.attention.push(TaskAttention::UserNote);
                }
            }
            for flag in parse_attention_values(&args.add_attention)? {
                if flag == TaskAttention::NeedsUser {
                    if task.kind == task_core::TaskKind::Epic || task.claimed_by.is_some() {
                        return Err(CliError::new("for_user_only", "Cannot mark an epic or claimed task For user; release its claim first").exit(4));
                    }
                    task.kind = task_core::TaskKind::ForUser;
                    task.attention.retain(|value| value != &TaskAttention::NeedsUser);
                } else if !task.attention.contains(&flag) {
                    task.attention.push(flag);
                }
            }
            for flag in parse_attention_values(&args.remove_attention)? {
                if flag == TaskAttention::NeedsUser && task.attention.contains(&TaskAttention::NeedsUser) {
                    return Err(CliError::new("for_user_only", "Only the user can clear a legacy NeedsUser / For user status in TaskBoard").exit(4));
                }
                task.attention.retain(|current| current != &flag);
            }
            if args.clear_question {
                task.question = None;
                task.attention.retain(|flag| flag != &TaskAttention::Question);
            } else if let Some(prompt) = &args.question {
                validate_inbox_question(prompt)?;
                task.question = Some(TaskQuestion {
                    request_type: None,
                    prompt: prompt.trim().to_string(),
                    options: args.options.clone(),
                    answer: None,
                });
                if !task.attention.contains(&TaskAttention::Question) {
                    task.attention.push(TaskAttention::Question);
                }
            }
            if args.clear_acceptance {
                task.acceptance_criteria.clear();
            } else if !args.acceptance_criteria.is_empty() {
                task.acceptance_criteria = args.acceptance_criteria.clone();
            }
            if let Some(deployable) = args.deployable { task.deployable = Some(deployable); }
            if let Some(reason) = &args.blocked_reason {
                validate_agent_block_reason(Some(reason))?;
                task.blocked_reason = Some(reason.clone());
            }
            Ok(json!(store.update_as_confirmed(root, task, task_core::MutationActor::Agent, args.confirm_genuine_block.as_deref(), "update").map_err(map_store_error)?))
        }
        TaskCommand::Move(args) => {
            let snapshot = load_checked(store, root)?;
            let column = resolve_column(&snapshot.config, &args.column)?.id.clone();
            let mut task = resolve_task(&snapshot, &args.target, false)?.clone();
            ensure_task_rewrite_safe(&task)?;
            if column == "in-progress" {
                if task.column == column && task.claimed_by.is_some() {
                    if args.agent.as_deref().is_some_and(|agent| task.claimed_by.as_deref() != Some(agent)) {
                        return Err(CliError::new("already_claimed", "In Progress belongs to a different agent").exit(3));
                    }
                    return Ok(json!(task));
                }
                if task.claimed_by.is_some() {
                    return Err(CliError::new("claim_column_mismatch", "Existing claim is outside In Progress; run task reconcile-claims --apply first").exit(3));
                }
                let agent = args.agent.as_deref().filter(|agent| !agent.trim().is_empty())
                    .ok_or_else(|| CliError::new("agent_required", "Agents entering In Progress must claim it: task claim ID --agent AGENT, or task move ID In-Progress --agent AGENT").exit(3))?;
                if args.managed {
                    let claim = store.claim_managed(root, &task.id, agent).map_err(map_store_error)?;
                    return Ok(json!({ "task": claim.task, "claim_token": claim.claim_token }));
                }
                return Ok(json!(store.claim(root, &task.id, agent).map_err(map_store_error)?));
            }
            if args.managed {
                return Err(CliError::new("invalid_managed", "--managed only applies when entering In Progress").exit(3));
            }
            if is_human_only(&task) && snapshot.config.columns.iter().any(|c| c.id == column && c.terminal) {
                return Err(CliError::new("for_user_only", "For user tasks can only be completed by the user in TaskBoard").exit(4));
            }
            if column == "blocked" && task.column == "blocked" && args.confirm_genuine_block.is_some() {
                return Err(CliError::new("blocked_confirmation_invalid", "challenge token cannot be reused for an already blocked task").exit(3));
            }
            let entering_blocked = column == "blocked";
            task.column = column;
            if entering_blocked { validate_agent_block_reason(args.blocked_reason.as_deref())?; }
            if let Some(reason) = &args.blocked_reason { task.blocked_reason = Some(reason.clone()); }
            Ok(json!(store.update_as_confirmed(root, task, task_core::MutationActor::Agent, args.confirm_genuine_block.as_deref(), "move").map_err(map_store_error)?))
        }
        TaskCommand::ReconcileClaims { apply } =>
            Ok(json!({ "applied": apply, "changes": store.reconcile_claim_columns(root, *apply).map_err(map_store_error)? })),
        TaskCommand::Parent(args) => {
            if !args.clear && args.parent.is_none() {
                return Err(CliError::new("usage_error", "provide --parent <task> or --clear"));
            }
            let snapshot = load_checked(store, root)?;
            let mut task = resolve_task(&snapshot, &args.target, false)?.clone();
            ensure_task_rewrite_safe(&task)?;
            task.parent = if args.clear {
                None
            } else {
                let parent = resolve_task(&snapshot, args.parent.as_ref().unwrap(), false)?;
                if parent.id == task.id {
                    return Err(CliError::new("hierarchy_cycle", "task cannot parent itself"));
                }
                Some(parent.id.clone())
            };
            Ok(json!(store.update(root, task).map_err(map_store_error)?))
        }
        TaskCommand::Live { command } => run_live(store, root, command),
        TaskCommand::Queue { command } => run_task_queue(store, root, command),
        TaskCommand::Execution(args) => {
            let snapshot = load_checked(store, root)?;
            let task = resolve_task(&snapshot, &args.target, false)?;
            let result = match &args.action {
                ExecutionAction::Show => store.execution_intent(root, &task.id),
                ExecutionAction::Set { mode, revision } => {
                    let mode = match mode.as_str() {
                        "backlog" => task_core::ExecutionMode::Backlog,
                        "queue" => task_core::ExecutionMode::Queue,
                        _ => return Err(CliError::new("invalid_execution_mode", "mode must be backlog or queue")),
                    };
                    store.set_execution_mode(root, &task.id, revision, mode)
                }
                ExecutionAction::StartNow { revision } => store.request_start_now(root, &task.id, revision),
                ExecutionAction::Receipt { request, receipt } => store.receipt_start_now(root, &task.id, request, receipt),
            }.map_err(map_store_error)?;
            Ok(json!(result))
        }
        TaskCommand::Finish(args) => run_finish(store, root, args),
        TaskCommand::AskUser(args) => run_ask_user(store, root, args),
        TaskCommand::CompleteManaged(args) => {
            let snapshot = load_checked(store, root)?;
            let task = resolve_task(&snapshot, &args.target, false)?;
            ensure_task_rewrite_safe(task)?;
            Ok(json!(store.complete_managed(root, &task.id, &args.agent, &args.claim_token, &args.expected_revision).map_err(map_store_error)?))
        }
        TaskCommand::Complete(args) => {
            let snapshot = load_checked(store, root)?;
            let mut terminals: Vec<&Column> = snapshot.config.columns.iter().filter(|c| c.terminal).collect();
            terminals.sort_by_key(|c| c.order);
            let terminal = terminals.first()
                .ok_or_else(|| CliError::new("invalid_column", "board has no terminal/completed column"))?;
            let mut task = resolve_task(&snapshot, &args.target, false)?.clone();
            ensure_task_rewrite_safe(&task)?;
            if is_human_only(&task) {
                return Err(CliError::new("for_user_only", "For user tasks can only be completed by the user in TaskBoard").exit(4));
            }
            task.column = terminal.id.clone();
            Ok(json!(store.update(root, task).map_err(map_store_error)?))
        }
        TaskCommand::Archive(args) => {
            let snapshot = load_checked(store, root)?;
            let task = resolve_task(&snapshot, &args.target, true)?;
            if is_human_only(task) && !task.archived {
                return Err(CliError::new("for_user_only", "Only the user may archive a For user task in TaskBoard").exit(4));
            }
            if !task.archived {
                let active_children: Vec<&str> = snapshot.tasks.iter()
                    .filter(|candidate| !candidate.archived && candidate.parent.as_deref() == Some(task.id.as_str()))
                    .map(|candidate| candidate.id.as_str())
                    .collect();
                if !active_children.is_empty() {
                    return Err(CliError::new(
                        "invalid_parent",
                        format!("cannot archive {} while it has active children", task.id),
                    ).details(json!({ "children": active_children })));
                }
            }
            Ok(json!(store.archive(root, &task.id).map_err(map_store_error)?))
        }
        TaskCommand::Claim(args) => {
            let snapshot = load_checked(store, root)?;
            let task = resolve_task(&snapshot, &args.target, false)?;
            ensure_task_rewrite_safe(task)?;
            if snapshot.config.columns.iter().any(|column| column.id == task.column && column.advisory) {
                return Err(CliError::new("not_executable", "Advisory cards are board-wide instructions, not executable tasks"));
            }
            let managed = if args.managed { Some(store.claim_managed(root, &task.id, &args.agent).map_err(map_store_error)?) } else { None };
            let claimed = if let Some(ref managed) = managed { managed.task.clone() } else { store.claim(root, &task.id, &args.agent).map_err(map_store_error)? };
            let mut result = with_work_advisory(json!(claimed), &claimed);
            if let Some(managed) = managed { result["claim_token"] = json!(managed.claim_token); }
            if let Some(object) = result.as_object_mut() {
                object.insert("advisories".into(), json!(board_advisories(&snapshot)));
            }
            Ok(result)
        }
        TaskCommand::Release(args) => {
            let snapshot = load_checked(store, root)?;
            let task = resolve_task(&snapshot, &args.target, true)?;
            ensure_task_rewrite_safe(task)?;
            if let Some(token) = &args.claim_token {
                Ok(json!(store.release_managed(root, &task.id, &args.agent, token).map_err(map_store_error)?))
            } else {
                Ok(json!(store.release_claim(root, &task.id, &args.agent).map_err(map_store_error)?))
            }
        }
    }
}

fn run_release(store: &FsTaskStore, root: &Path, command: &ReleaseCommand) -> Result<Value, CliError> {
    use task_core::SourceRef;
    let result = match command {
        ReleaseCommand::List => json!(store.releases(root).map_err(map_store_error)?),
        ReleaseCommand::New => json!(store.release_new(root).map_err(map_store_error)?),
        ReleaseCommand::Show(target) => {
            let data = store.releases(root).map_err(map_store_error)?;
            json!(data.releases.into_iter().find(|r| r.id == target.target).ok_or_else(|| CliError::new("release_not_found", &target.target))?)
        },
        ReleaseCommand::Enable {enabled} => json!(store.release_enable(root, *enabled).map_err(map_store_error)?),
        ReleaseCommand::Reconcile {historical} => json!(store.reconcile_releases(root, *historical).map_err(map_store_error)?),
        ReleaseCommand::BlockedCandidates => {
            let board = load_checked(store, root)?;
            json!({"candidates_for_manual_review": board.tasks.iter().filter(|t| !t.archived &&
                (t.column == "blocked" || t.attention.contains(&TaskAttention::Blocked)))
                .map(|t| json!({"id":t.id,"title":t.title,"column":t.column,"blocked_reason":t.blocked_reason,"kind":t.kind}))
                .collect::<Vec<_>>()})
        },
        ReleaseCommand::MigrateBlocked {task, validated, evidence} => {
            if !validated { return Err(CliError::new("explicit_validation_required",
                "Do not migrate a blocked task unless source implementation is already complete, committed and tested; retry with --validated and specific --evidence").exit(3)); }
            json!(store.release_migrate_blocked(root, task, evidence).map_err(map_store_error)?)
        },
        ReleaseCommand::Include {release,task} => json!(store.release_edit(root, "include", release, Some(task), None, None).map_err(map_store_error)?),
        ReleaseCommand::Exclude {release,task} => json!(store.release_edit(root, "exclude", release, Some(task), None, None).map_err(map_store_error)?),
        ReleaseCommand::Move {release,task,destination} => json!(store.release_edit(root, "move", release, Some(task), None, Some(destination)).map_err(map_store_error)?),
        ReleaseCommand::Source {release,task,repo,commit,integrated_ref,evidence} =>
            json!(store.release_edit(root, "source", release, Some(task), Some(SourceRef {repository:repo.clone(),commit:commit.clone(),integrated_ref:integrated_ref.clone()}), evidence.as_deref()).map_err(map_store_error)?),
        ReleaseCommand::Prepare {release,evidence,run_id} => json!(store.release_prepare_bound(root, release, evidence, run_id.as_deref()).map_err(map_store_error)?),
        ReleaseCommand::Outcome {release,attempt,status,evidence,run_id} => json!(store.release_outcome_bound(root, release, *attempt, status, evidence, run_id.as_deref()).map_err(map_store_error)?),
        ReleaseCommand::Verify {release,attempt,task,status,evidence} => {
            let passed = match status.as_str() {
                "verified" => true,
                "verification_failed" => false,
                _ => return Err(CliError::new("invalid_verification", "status must be verified or verification_failed")),
            };
            json!(store.release_verify_member(root, release, *attempt, task, passed, evidence).map_err(map_store_error)?)
        },
    };
    Ok(result)
}

fn run_column(store: &FsTaskStore, root: &Path, command: &ColumnCommand) -> Result<Value, CliError> {
    match command {
        ColumnCommand::List => {
            let snapshot = load_checked(store, root)?;
            let mut columns = snapshot.config.columns.clone();
            columns.sort_by_key(|c| c.order);
            Ok(json!({ "columns": columns }))
        }
        ColumnCommand::Create(args) => {
            let snapshot = load_checked(store, root)?;
            ensure_board_rewrite_safe(root, &snapshot.config)?;
            let mut config = snapshot.config.clone();
            let id = args.id.clone().unwrap_or_else(|| slugify(&args.name));
            if id.is_empty() {
                return Err(CliError::new("invalid_column", "column id cannot be empty"));
            }
            if config.columns.iter().any(|c| c.id.eq_ignore_ascii_case(&id)) {
                return Err(CliError::new("invalid_column", format!("column id already exists: {id}")));
            }
            let order = config.columns.iter().map(|c| c.order).max().unwrap_or(-1) + 1;
            config.columns.push(Column {
                id,
                name: args.name.trim().to_string(),
                order,
                terminal: args.terminal,
                hidden: false,
                snoozed_until: None,
                awake_until: None,
                ai_ready: task_core::default_column_ai_ready(&args.id.clone().unwrap_or_else(|| slugify(&args.name)), &args.name, args.terminal),
                advisory: false,
                auto_queue: false,
                color: column_color(order),
            });
            let saved = store.save_config(root, config).map_err(map_store_error)?;
            Ok(json!(saved))
        }
        ColumnCommand::Rename(args) => {
            let snapshot = load_checked(store, root)?;
            ensure_board_rewrite_safe(root, &snapshot.config)?;
            let id = resolve_column(&snapshot.config, &args.target)?.id.clone();
            let mut config = snapshot.config.clone();
            let column = config.columns.iter_mut().find(|c| c.id == id).unwrap();
            if args.name.trim().is_empty() {
                return Err(CliError::new("invalid_column", "column name cannot be empty"));
            }
            column.name = args.name.trim().to_string();
            let saved = store.save_config(root, config).map_err(map_store_error)?;
            Ok(json!(saved))
        }
        ColumnCommand::Reorder(args) => {
            let snapshot = load_checked(store, root)?;
            ensure_board_rewrite_safe(root, &snapshot.config)?;
            let id = resolve_column(&snapshot.config, &args.target)?.id.clone();
            if args.position == 0 || args.position > snapshot.config.columns.len() {
                return Err(CliError::new(
                    "invalid_column",
                    format!("position must be between 1 and {}", snapshot.config.columns.len()),
                ));
            }
            let mut columns = snapshot.config.columns.clone();
            columns.sort_by_key(|c| c.order);
            let index = columns.iter().position(|c| c.id == id).unwrap();
            let column = columns.remove(index);
            columns.insert(args.position - 1, column);
            for (index, column) in columns.iter_mut().enumerate() {
                column.order = index as i32;
            }
            let mut config = snapshot.config.clone();
            config.columns = columns;
            let saved = store.save_config(root, config).map_err(map_store_error)?;
            Ok(json!(saved))
        }
        ColumnCommand::Ready(args) => {
            let snapshot = load_checked(store, root)?;
            ensure_board_rewrite_safe(root, &snapshot.config)?;
            let id = resolve_column(&snapshot.config, &args.target)?.id.clone();
            let mut config = snapshot.config.clone();
            let column = config.columns.iter_mut().find(|c| c.id == id).unwrap();
            if args.value && (column.id == "draft" || column.terminal) {
                return Err(CliError::new(
                    "invalid_column",
                    "Draft and terminal columns cannot be marked AI ready",
                ));
            }
            column.ai_ready = args.value;
            if args.value { column.advisory = false; }
            let saved = store.save_config(root, config).map_err(map_store_error)?;
            Ok(json!(saved))
        }
        ColumnCommand::Advisory(args) => {
            let snapshot = load_checked(store, root)?;
            ensure_board_rewrite_safe(root, &snapshot.config)?;
            let id = resolve_column(&snapshot.config, &args.target)?.id.clone();
            let mut config = snapshot.config.clone();
            let column = config.columns.iter_mut().find(|c| c.id == id).unwrap();
            if args.value && (column.id == "draft" || column.id == AI_READY_COLUMN_ID || column.terminal) {
                return Err(CliError::new("invalid_column", "Draft, canonical AI Ready and terminal columns cannot be advisory"));
            }
            column.advisory = args.value;
            if args.value { column.ai_ready = false; }
            let saved = store.save_config(root, config).map_err(map_store_error)?;
            Ok(json!(saved))
        }
        ColumnCommand::Remove(args) => {
            let snapshot = load_checked(store, root)?;
            ensure_board_rewrite_safe(root, &snapshot.config)?;
            let column = resolve_column(&snapshot.config, &args.target)?;
            if snapshot.config.columns.len() == 1 {
                return Err(CliError::new("invalid_column", "cannot remove the board's last column"));
            }
            let in_use: Vec<&str> = snapshot.tasks.iter()
                .filter(|task| !task.archived && task.column == column.id)
                .map(|task| task.id.as_str())
                .collect();
            if !in_use.is_empty() {
                return Err(CliError::new(
                    "invalid_column",
                    format!("column {} is still used by active tasks", column.id),
                ).details(json!({ "tasks": in_use })));
            }
            let mut config = snapshot.config.clone();
            config.columns.retain(|candidate| candidate.id != column.id);
            config.columns.sort_by_key(|c| c.order);
            for (index, candidate) in config.columns.iter_mut().enumerate() {
                candidate.order = index as i32;
            }
            let saved = store.save_config(root, config).map_err(map_store_error)?;
            Ok(json!(saved))
        }
    }
}

fn parse_kind(value: &str) -> Result<task_core::TaskKind, CliError> {
    match value {
        "task" => Ok(task_core::TaskKind::Task),
        "epic" => Ok(task_core::TaskKind::Epic),
        "for_user" | "for-user" => Ok(task_core::TaskKind::ForUser),
        "recurring" => Ok(task_core::TaskKind::Recurring),
        _ => Err(CliError::new("usage_error", "kind must be task, epic, for_user or recurring")),
    }
}
fn unfinished_dependencies<'a>(snapshot: &'a BoardSnapshot, task: &'a Task) -> Vec<&'a str> {
    task.depends_on.iter().filter(|id| {
        snapshot.tasks.iter().find(|t| t.id == **id)
            .is_none_or(|t| !t.archived && t.completed.is_none())
    }).map(String::as_str).collect()
}
fn is_human_only(task: &Task) -> bool {
    task.kind == task_core::TaskKind::ForUser || (task.attention.contains(&TaskAttention::NeedsUser)
        && !task.live.as_ref().is_some_and(|live|
            matches!(live.state, Some(task_core::LiveTaskState::Ready))))
}
/// Board-wide temporary notices are returned alongside actionable tasks,
/// never mixed into the execution queue. Moving one to Draft hides it here.
fn board_advisories(snapshot: &BoardSnapshot) -> Vec<&Task> {
    let advisory_columns: HashSet<&str> = snapshot.config.columns.iter()
        .filter(|column| column.advisory)
        .map(|column| column.id.as_str()).collect();
    snapshot.tasks.iter().filter(|task| !task.archived
        && advisory_columns.contains(task.column.as_str())).collect()
}
fn ensure_ready(snapshot: &BoardSnapshot, task: &Task) -> bool {
    matches!(task.kind, task_core::TaskKind::Task | task_core::TaskKind::Recurring) &&
        task_core::recurring_due(task) && !is_human_only(task) &&
        !task.live.as_ref().is_some_and(|live| live.state.is_some()) &&
        unfinished_dependencies(snapshot, task).is_empty() &&
        !task.attention.contains(&TaskAttention::Blocked) &&
        task.column != "blocked"
}
fn readiness(snapshot: &BoardSnapshot, task: &Task) -> &'static str {
    if snapshot.config.columns.iter().any(|column| column.id == task.column && column.advisory) {
        return "advisory";
    }
    if task.kind == task_core::TaskKind::Epic { "epic" }
    else if is_human_only(task) { "for_user" }
    else if task.attention.contains(&TaskAttention::Blocked) || task.column == "blocked" { "blocked" }
    else if !unfinished_dependencies(snapshot, task).is_empty() { "waiting" }
    else if task.kind == task_core::TaskKind::Recurring && !task_core::recurring_due(task) { "scheduled" }
    else { "ready" }
}
/// Legacy blocked tasks predate `blocked_reason`; fall back to the first line
/// of their AI Notes so agents still see why they are blocked.
fn effective_blocked_reason(task: &Task) -> (Option<String>, Option<&'static str>) {
    if !task_core::is_task_blocked(task) { return (None, None); }
    if let Some(reason) = task.blocked_reason.as_deref().filter(|r| !r.trim().is_empty()) {
        return (Some(reason.to_string()), Some("blocked_reason"));
    }
    let legacy = task.ai_notes.lines().map(str::trim).find(|line| !line.is_empty())
        .map(|line| line.chars().take(200).collect::<String>());
    match legacy {
        Some(line) => (Some(line), Some("legacy_ai_notes")),
        None => (None, Some("missing_legacy")),
    }
}
fn ensure_task_rewrite_safe(task: &Task) -> Result<(), CliError> {
    let text = fs::read_to_string(&task.path)
        .map_err(|error| CliError::new("io_error", error.to_string()).exit(5))?;
    let (yaml, _) = markdown_parts(&text, &task.path)?;
    let value: serde_yaml::Value = serde_yaml::from_str(&yaml)
        .map_err(|error| CliError::new("invalid_board_task_data", error.to_string()).exit(4))?;
    if value.as_mapping().is_none() {
        return Err(CliError::new(
            "invalid_board_task_data",
            format!("{} frontmatter is not a mapping", task.path),
        ).exit(4));
    }
    Ok(())
}

fn ensure_board_rewrite_safe(root: &Path, _config: &BoardConfig) -> Result<(), CliError> {
    let path = root.join("_board.md");
    let text = fs::read_to_string(&path)
        .map_err(|error| CliError::new("io_error", error.to_string()).exit(5))?;
    let display = path.to_string_lossy().to_string();
    let (yaml, _) = markdown_parts(&text, &display)?;
    let value: serde_yaml::Value = serde_yaml::from_str(&yaml)
        .map_err(|error| CliError::new("invalid_board_task_data", error.to_string()).exit(4))?;
    if value.as_mapping().is_none() {
        return Err(CliError::new(
            "invalid_board_task_data",
            format!("{display} frontmatter is not a mapping"),
        ).exit(4));
    }
    Ok(())
}

fn markdown_parts(text: &str, path: &str) -> Result<(String, String), CliError> {
    let normalized = text.strip_prefix('\u{feff}').unwrap_or(text).replace("\r\n", "\n");
    if !normalized.starts_with("---\n") {
        return Err(CliError::new("invalid_board_task_data", format!("{path}: missing YAML frontmatter")).exit(4));
    }
    let rest = &normalized[4..];
    let marker = "\n---\n";
    let Some(index) = rest.find(marker) else {
        return Err(CliError::new("invalid_board_task_data", format!("{path}: unterminated YAML frontmatter")).exit(4));
    };
    Ok((rest[..index].to_string(), rest[index + marker.len()..].to_string()))
}

fn load_checked(store: &FsTaskStore, root: &Path) -> Result<BoardSnapshot, CliError> {
    let snapshot = store.load(root).map_err(map_store_error)?;
    if !snapshot.warnings.is_empty() {
        return Err(CliError::new(
            "invalid_board_task_data",
            "board contains invalid or duplicate task data; refusing to operate until it is resolved",
        ).details(json!({ "warnings": snapshot.warnings })));
    }
    Ok(snapshot)
}

fn resolve_task<'a>(snapshot: &'a BoardSnapshot, target: &str, include_archived: bool) -> Result<&'a Task, CliError> {
    let candidates: Vec<&Task> = snapshot.tasks.iter()
        .filter(|task| include_archived || !task.archived)
        .collect();

    if let Some(task) = candidates.iter()
        .copied()
        .find(|task| task.id.eq_ignore_ascii_case(target))
    {
        return Ok(task);
    }

    let title_matches: Vec<&Task> = candidates.into_iter()
        .filter(|task| task.title.eq_ignore_ascii_case(target))
        .collect();

    match title_matches.len() {
        1 => Ok(title_matches[0]),
        0 => Err(CliError::new("task_not_found", format!("task not found: {target}"))),
        _ => Err(CliError::new("ambiguous_task", format!("multiple tasks have title {target:?}"))
            .details(json!({ "matches": title_matches.iter().map(|t| &t.id).collect::<Vec<_>>() }))),
    }
}

fn resolve_column<'a>(config: &'a BoardConfig, target: &str) -> Result<&'a Column, CliError> {
    if let Some(column) = config.columns.iter().find(|column| column.id.eq_ignore_ascii_case(target)) {
        return Ok(column);
    }
    let matches: Vec<&Column> = config.columns.iter()
        .filter(|column| column.name.eq_ignore_ascii_case(target))
        .collect();
    match matches.len() {
        1 => Ok(matches[0]),
        0 => Err(CliError::new("invalid_column", format!("unknown column: {target}"))),
        _ => Err(CliError::new("invalid_column", format!("ambiguous column name: {target}"))
            .details(json!({ "matches": matches.iter().map(|c| &c.id).collect::<Vec<_>>() }))),
    }
}

fn task_matches(task: &Task, needle: &str) -> bool {
    task.id.to_lowercase().contains(needle)
        || task.title.to_lowercase().contains(needle)
        || task.category.as_deref().is_some_and(|category| category.to_lowercase().contains(needle))
        || task.description.to_lowercase().contains(needle)
        || task.ai_notes.to_lowercase().contains(needle)
        || task.user_notes.to_lowercase().contains(needle)
        || task.question.as_ref().is_some_and(|question| {
            question.prompt.to_lowercase().contains(needle)
                || question.answer.as_deref().is_some_and(|answer| answer.to_lowercase().contains(needle))
                || question.options.iter().any(|option| option.to_lowercase().contains(needle))
        })
        || task.notes.to_lowercase().contains(needle)
        || task.acceptance_criteria.iter().any(|item| item.to_lowercase().contains(needle))
}

fn parse_attention_values(values: &[String]) -> Result<Vec<TaskAttention>, CliError> {
    values.iter().map(|value| {
        match value.trim().to_ascii_lowercase().as_str() {
            "question" => Ok(TaskAttention::Question),
            "needs-user" | "needsuser" | "needs_user" => Ok(TaskAttention::NeedsUser),
            "user-note" | "usernote" | "user_note" => Ok(TaskAttention::UserNote),
            "blocked" => Ok(TaskAttention::Blocked),
            "priority" => Ok(TaskAttention::Priority),
            other => Err(CliError::new(
                "usage_error",
                format!("unknown attention flag {other:?}; expected question, needs-user, user-note, blocked, or priority"),
            )),
        }
    }).collect()
}

fn default_create_column(config: &BoardConfig) -> Result<&Column, CliError> {
    config
        .columns
        .iter()
        .find(|column| column.id == AI_READY_COLUMN_ID)
        .or_else(|| config.columns.iter().find(|column| column.ai_ready))
        .or_else(|| {
            config
                .columns
                .iter()
                .find(|column| !column.terminal && column.id != "draft")
        })
        .or_else(|| config.columns.iter().min_by_key(|column| column.order))
        .ok_or_else(|| CliError::new("invalid_column", "board has no columns"))
}

fn column_color(order: i32) -> String {
    const COLORS: [&str; 8] = [
        "#718f78",
        "#7388a8",
        "#8e7da8",
        "#a48669",
        "#6f8679",
        "#8c7d72",
        "#6f8b91",
        "#927b86",
    ];
    COLORS[order.rem_euclid(COLORS.len() as i32) as usize].to_string()
}

fn slugify(name: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for ch in name.trim().to_lowercase().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
    }
    out.trim_matches('-').to_string()
}

fn map_store_error(error: TaskStoreError) -> CliError {
    match error {
        TaskStoreError::BlockedChallenge { token, task, action } => CliError::new("blocked_confirmation_required",
            "Self-check: is implementation genuinely impossible to continue? Waiting for deployment, restart or live acceptance belongs to Releases, NOT Blocked. If source is committed and tested, move Done; if work remains possible, stay In Progress. ONLY for a real blocker, retry the identical command with --confirm-genuine-block TOKEN from this response.")
            .details(json!({"token": token, "task": task, "action": action, "expires_in_seconds":600, "retry_flag": "--confirm-genuine-block"})).exit(3),
        TaskStoreError::NotFound(value) => CliError::new("task_not_found", format!("task not found: {value}")),
        TaskStoreError::ClaimInstanceMismatch(message) => CliError::new("claim_instance_mismatch", message).exit(3),
        TaskStoreError::Conflict(message) => CliError::new("write_conflict", message).exit(3),
        TaskStoreError::AlreadyClaimed { task_id, claimed_by } => CliError::new(
            "already_claimed",
            format!("task {task_id} is claimed by {claimed_by}"),
        ).details(json!({ "task_id": task_id, "claimed_by": claimed_by })).exit(3),
        TaskStoreError::Parse(message) => CliError::new("invalid_board_task_data", message).exit(4),
        TaskStoreError::Io(error) => CliError::new("io_error", error.to_string()).exit(5),
        TaskStoreError::Invalid(message) => {
            if message.starts_with("blocked_") {
                return CliError::new(if message.starts_with("blocked_reason_required") { "blocked_reason_required" } else { "blocked_confirmation_invalid" }, message).exit(3);
            }
            let lower = message.to_lowercase();
            if lower.contains("blocked without a reason") {
                CliError::new("blocked_reason_required", message).exit(4)
            } else if lower.contains("for user") && lower.contains("agents may not claim") {
                CliError::new("for_user_only", message).exit(4)
            } else if lower.contains("epic") && lower.contains("not executable") {
                CliError::new("epic_not_executable", message).exit(4)
            } else if lower.contains("unfinished dependencies") {
                CliError::new("dependency_waiting", message).exit(3)
            } else if lower.contains("cycle") || lower.contains("parent itself") {
                CliError::new("hierarchy_cycle", message)
            } else if lower.contains("parent") {
                CliError::new("invalid_parent", message)
            } else if lower.contains("column") {
                CliError::new("invalid_column", message)
            } else {
                CliError::new("invalid_board_task_data", message).exit(4)
            }
        }
    }
}

// Task-type work restrictions are instructions for agents, not access controls.
// Include them on "show" (before starting) and "claim" (at the start of work),
// without changing command success, claim ownership, or the existing task shape.
fn with_work_advisory(mut result: Value, task: &Task) -> Value {
    if task.kind == task_core::TaskKind::Epic {
        result["warnings"] = json!([{"code": "epic_not_executable", "message": "EPIC: Coordination only. Do not claim, assign, move, or implement directly. Select a ready executable subtask.", "advisory_only": false}]);
        return result;
    }
    if task.kind == task_core::TaskKind::ForUser {
        result["warnings"] = json!([{"code": "for_user_only", "message": "FOR USER: This is a human-only task. Agents must not claim, perform, complete or archive it. They may add planning notes and create separate implementation tasks.", "advisory_only": false}]);
        return result;
    }
    let advisory = match task.icon.as_deref() {
        Some("visual-work") => Some((
            "visual_work_claude_opus_only",
            "HARD VISUAL WORK: Only Claude Opus may work on this task. If you are not Claude Opus, do not claim or implement it; leave it for a Claude Opus agent. This is advisory metadata, not an enforced restriction.",
        )),
        Some("visual-implementation") => Some((
            "visual_implementation_approved",
            "VISUAL IMPLEMENTATION: Approved visual mockup is ready for implementation. Any suitable agent, including non-Claude agents, may implement against the approved reference.",
        )),
        Some("design") => Some((
            "design_initial_draft_opus_astra_only",
            "DESIGN: Only Opus or Astra agents should write the INITIAL design draft. Other agents may work on subsequent revisions after an initial draft exists. Check the draft status and your model before proceeding. This is an advisory, not an enforced restriction.",
        )),
        _ => None,
    };
    let mut warnings: Vec<Value> = Vec::new();
    if task.attention.contains(&TaskAttention::Priority) {
        warnings.push(json!({
            "code": "priority_first",
            "message": "PRIORITY: Prefer this ready task over ordinary ready tasks. Respect blocked status and unfinished dependencies.",
            "advisory_only": true
        }));
    }
    if let Some((code, message)) = advisory {
        warnings.push(json!({
            "code": code,
            "message": message,
            "advisory_only": true
        }));
    }
    if !warnings.is_empty() {
        result["warnings"] = json!(warnings);
    }
    result
}

fn print_success(json_mode: bool, value: Value) {
    if json_mode {
        println!("{}", serde_json::to_string(&value).unwrap());
        return;
    }

    if let Some(tasks) = value.get("tasks").and_then(Value::as_array) {
        for task in tasks {
            let id = task.get("id").and_then(Value::as_str).unwrap_or("?");
            let title = task.get("title").and_then(Value::as_str).unwrap_or("");
            let column = task.get("column").and_then(Value::as_str).unwrap_or("");
            let kind = task.get("kind").and_then(Value::as_str).unwrap_or("task");
            match task.get("blocked_reason").and_then(Value::as_str) {
                Some(reason) => println!("{id}\t{column}\t{kind}\t{title}\t[blocked: {reason}]"),
                None => println!("{id}\t{column}\t{kind}\t{title}"),
            }
        }
        return;
    }
    if let Some(columns) = value.get("columns").and_then(Value::as_array) {
        for column in columns {
            let id = column.get("id").and_then(Value::as_str).unwrap_or("?");
            let name = column.get("name").and_then(Value::as_str).unwrap_or("");
            let order = column.get("order").and_then(Value::as_i64).unwrap_or(0);
            let terminal = column.get("terminal").and_then(Value::as_bool).unwrap_or(false);
            println!("{}\t{}\t{}\t{}", order + 1, id, name, if terminal { "terminal" } else { "" });
        }
        return;
    }
    println!("{}", serde_json::to_string_pretty(&value).unwrap());
}

fn print_error(json_mode: bool, error: &CliError) {
    if json_mode {
        eprintln!("{}", json!({
            "error": {
                "code": error.code,
                "message": error.message,
                "details": error.details,
            }
        }));
    } else {
        eprintln!("error[{}]: {}", error.code, error.message);
        if !error.details.is_null() {
            eprintln!("{}", serde_json::to_string_pretty(&error.details).unwrap());
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        sync::{Arc, Barrier},
        thread,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn temp_board(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("taskboard-cli-{label}-{}-{nonce}", process::id()))
    }

    fn draft(title: &str) -> TaskDraft {
        TaskDraft {
            reserved_id: None,
            reservation_token: None,
            title: title.into(),
            kind: task_core::TaskKind::Task,
            deployable: None,
            repeat_every_days: None, next_due: None,
            depends_on: vec![],
            related_to: vec![],
            column: AI_READY_COLUMN_ID.into(),
            parent: None,
            category: None,
            icon: None,
            attention: Vec::new(),
            question: None,
            blocked_reason: None,
            description: String::new(),
            acceptance_criteria: Vec::new(),
            ai_notes: String::new(),
            user_notes: String::new(),
            notes: String::new(),
        }
    }




    #[test]
    fn for_user_only_cards_are_not_ai_claimable_or_completable() {
        let root = temp_board("for-user");
        let store = FsTaskStore::default();
        store.ensure_board(&root, "Test").unwrap();
        let mut human = draft("Approve final spear animation");
        human.kind = task_core::TaskKind::ForUser;
        let human = store.create(&root, human).unwrap();

        let show = run_task(&store, &root, &TaskCommand::Show(TaskTarget { target: human.id.clone() })).unwrap();
        assert_eq!(show["readiness"], "for_user");
        assert_eq!(show["warnings"][0]["code"], "for_user_only");

        let ready = run_task(&store, &root, &TaskCommand::List(TaskListArgs {
            include_archived: false,
            column: None,
            parent: None,
            roots: false,
            ai_ready: true,
            live_state: None,
        })).unwrap();
        assert_eq!(ready["tasks"].as_array().unwrap().len(), 0);

        let claim = run_task(&store, &root, &TaskCommand::Claim(TaskClaimArgs {
            target: human.id.clone(), agent: "assistant".into(),
                    managed: false,
                    claim_token: None,
        })).unwrap_err();
        assert!(claim.message.contains("FOR USER"));

        let complete = run_task(&store, &root, &TaskCommand::Complete(TaskTarget { target: human.id.clone() })).unwrap_err();
        assert_eq!(complete.code, "for_user_only");

        let move_done = run_task(&store, &root, &TaskCommand::Move(TaskMoveArgs {
            target: human.id.clone(), column: "done".into(),
            agent: None, managed: false, blocked_reason: None, confirm_genuine_block: None,
        })).unwrap_err();
        assert_eq!(move_done.code, "for_user_only");

        let archived = run_task(&store, &root, &TaskCommand::Archive(TaskTarget { target: human.id.clone() })).unwrap_err();
        assert_eq!(archived.code, "for_user_only");

        // Human UI completion through native core works; CLI does not.
        let mut finished = store.load(&root).unwrap().tasks.into_iter().find(|t| t.id == human.id).unwrap();
        finished.column = "done".into();
        store.update(&root, finished).unwrap();
        let done = store.load(&root).unwrap().tasks.into_iter().find(|t| t.id == human.id).unwrap();
        assert!(done.completed.is_some());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn ask_user_and_finish_attach_one_request_to_the_original_task() {
        let root = temp_board("human-input");
        let store = FsTaskStore::default();
        store.ensure_board(&root, "Test").unwrap();
        let one = store.create(&root, draft("Implement feature")).unwrap();
        let id = one.id.clone();
        store.claim(&root, &id, "InitialWorker").unwrap();
        let user = TaskCommand::AskUser(TaskAskUserArgs {
            target: id.clone(), request_type: "decision".into(),
            question: "Which approach?".into(), options: vec!["A".into(), "B".into()],
            artifacts: vec![], mode: None, previous: None, poster: None,
        });
        let response = run_task(&store, &root, &user).unwrap();
        assert_eq!(response["task"]["question"]["request_type"], "decision");
        assert_eq!(response["task"]["column"], "blocked");
        assert!(response["task"]["claimed_by"].is_null());
        assert_eq!(run_task(&store, &root, &user).unwrap_err().code, "request_already_open");
        let snapshot = store.load(&root).unwrap();
        assert_eq!(snapshot.tasks.len(), 1);
        let mut answer = snapshot.tasks.into_iter().find(|task| task.id == id).unwrap();
        answer.question.as_mut().unwrap().answer = Some("A".into());
        answer.attention.retain(|flag| flag != &TaskAttention::Question);
        let resumed = store.update_as(&root, answer, task_core::MutationActor::Human).unwrap();
        assert_eq!(resumed.column, "ai-ready");
        assert!(resumed.claimed_by.is_none());
        assert!(resumed.ai_notes.contains("USER DECISION - IMPLEMENT NEXT"));
        assert!(resumed.ai_notes.contains("Answer: A"));
        store.claim(&root, &id, "TestWorker").unwrap();
        let finish = TaskCommand::Finish(TaskFinishArgs {
            target: id.clone(), verification: "user".into(),
            instructions: Some("Open the built app and check feature X".into()),
        });
        let completed = run_task(&store, &root, &finish).unwrap();
        assert_eq!(completed["task"]["column"], "done");
        assert!(completed["task"]["claimed_by"].is_null());
        assert_eq!(completed["task"]["question"]["request_type"], "verification");
        assert_eq!(completed["task"]["question"]["options"], json!(["Done", "Needs changes"]));
        assert!(completed["task"]["completed"].is_string());
        assert_eq!(run_task(&store, &root, &finish).unwrap_err().code, "request_already_open");
        assert_eq!(store.load(&root).unwrap().tasks.len(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn visual_ask_user_requires_evidence_and_uses_the_original_review_manifest() {
        let root = temp_board("visual-request");
        let store = FsTaskStore::default();
        store.ensure_board(&root, "Test").unwrap();
        let created = store.create(&root, draft("Review design")).unwrap();
        let mut request = TaskAskUserArgs { target: created.id.clone(),
            request_type: "visual-review".into(), question: "Approve this design?".into(),
            options: vec![], artifacts: vec![], mode: None, previous: None, poster: None };
        assert_eq!(run_ask_user(&store, &root, &request).unwrap_err().code, "visual_artifact_required");
        let image = root.join("design.png");
        fs::write(&image, b"\x89PNG\r\n\x1a\n").unwrap();
        request.artifacts.push(format!("{}=Design preview", image.display()));
        let review = run_ask_user(&store, &root, &request).unwrap();
        assert_eq!(review["status"], "open");
        let updated = store.load(&root).unwrap();
        assert_eq!(updated.tasks.len(), 1);
        assert_eq!(updated.tasks[0].id, created.id);
        assert_eq!(updated.tasks[0].review_status.as_deref(), Some("open"));
        assert_eq!(run_ask_user(&store, &root, &request).unwrap_err().code, "request_already_open");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn blocked_reason_is_required_and_surfaced_by_cli() {
        let root = temp_board("blocked-reason");
        let store = FsTaskStore::default();
        store.ensure_board(&root, "Test").unwrap();
        let task = store.create(&root, draft("Needs asset")).unwrap();
        let id = task.id.as_str();
        let root_arg = root.to_string_lossy().to_string();
        let exec = |extra: &[&str]| {
            let mut argv = vec!["taskboard", "--root", root_arg.as_str(), "task"];
            argv.extend_from_slice(extra);
            run(&Cli::try_parse_from(argv).unwrap())
        };

        let missing = exec(&["update", id, "--add-attention", "blocked"]).unwrap_err();
        assert_eq!(missing.code, "blocked_reason_required");
        let missing_move = exec(&["move", id, "blocked"]).unwrap_err();
        assert_eq!(missing_move.code, "blocked_reason_required");

        let first = exec(&["move", id, "blocked", "--blocked-reason", "Waiting for artist mesh."]).unwrap_err();
        assert_eq!(first.code, "blocked_confirmation_required");
        let challenge = first.details["token"].as_str().unwrap();
        exec(&["move", id, "blocked", "--blocked-reason", "Waiting for artist mesh.", "--confirm-genuine-block", challenge]).unwrap();
        let shown = exec(&["show", id]).unwrap();
        assert_eq!(shown["blocked_reason"], "Waiting for artist mesh.");
        assert_eq!(shown["task"]["blocked_reason"], "Waiting for artist mesh.");
        assert_eq!(shown["readiness"], "blocked");
        let listed = exec(&["list"]).unwrap();
        assert_eq!(listed["tasks"][0]["blocked_reason"], "Waiting for artist mesh.");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn column_ready_command_parses_explicit_boolean() {
        let cli = Cli::try_parse_from([
            "taskboard",
            "column",
            "ready",
            "rancui",
            "true",
        ])
        .unwrap();

        match cli.command {
            Command::Column {
                command: ColumnCommand::Ready(args),
            } => {
                assert_eq!(args.target, "rancui");
                assert!(args.value);
            }
            _ => panic!("expected column ready command"),
        }
    }

    #[test]
    fn ai_ready_list_includes_any_marked_column() {
        let root = temp_board("dynamic-ready");
        let store = FsTaskStore::default();
        store.ensure_board(&root, "Test").unwrap();

        let mut config = store.load(&root).unwrap().config;
        let order = config.columns.len() as i32;
        config.columns.push(Column {
            id: "rancui".into(),
            name: "RancUI".into(),
            order,
            terminal: false,
            hidden: false,
            snoozed_until: None,
            awake_until: None,
            ai_ready: true, advisory: false, auto_queue: false,
            color: "#777777".into(),
        });
        store.save_config(&root, config).unwrap();

        let mut task = draft("Ranc task");
        task.column = "rancui".into();
        store.create(&root, task).unwrap();

        let result = run_task(
            &store,
            &root,
            &TaskCommand::List(TaskListArgs {
                include_archived: false,
                column: None,
                parent: None,
                roots: false,
                ai_ready: true,
                live_state: None,
            }),
        )
        .unwrap();

        let tasks = result["tasks"].as_array().unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0]["title"], "Ranc task");

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn cli_user_notes_raise_user_note_attention() {
        let root = temp_board("user-note");
        let store = FsTaskStore::default();
        store.ensure_board(&root, "Test").unwrap();

        let args = TaskCreateArgs {
            title: "Human feedback".into(),
            kind: "task".into(),
            repeat_every_days: None, next_due: None,
            depends_on: vec![],
            related_to: vec![],
            column: None,
            parent: None,
            category: None,
            icon: None,
            description: String::new(),
            acceptance_criteria: Vec::new(),
            ai_notes: String::new(),
            user_notes: "Make a proposed design first.".into(),
            attention: Vec::new(),
            question: None,
            options: Vec::new(),
            blocked_reason: None, confirm_genuine_block: None,
        };

        run_task(&store, &root, &TaskCommand::Create(args)).unwrap();
        let task = store.load(&root).unwrap().tasks.into_iter().next().unwrap();
        assert_eq!(task.user_notes, "Make a proposed design first.");
        assert!(task.attention.contains(&TaskAttention::UserNote));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn release_attempt_run_id_is_idempotent_and_verifies_per_member() {
        let root = temp_board("release-bound");
        let store = FsTaskStore::default();
        store.ensure_board(&root, "Release contract").unwrap();
        store.release_enable(&root, true).unwrap();
        let mut first = store.create(&root, draft("First change")).unwrap();
        first.column = "done".into();
        first = store.update(&root, first).unwrap();
        let rel = store.releases(&root).unwrap().releases.pop().unwrap();
        assert_eq!(rel.members.len(), 1);
        let source = task_core::SourceRef {
            repository: "T3 Code".into(),
            commit: "exact-commit-A".into(),
            integrated_ref: Some("main-build-sha-A".into()),
        };
        store.release_edit(&root, "source", &rel.id, Some(&first.id),
            Some(source), Some("code reviewed, merged")).unwrap();
        let run = "release-test-run-0001";
        let prepared = store.release_prepare_bound(&root, &rel.id, "immutable sha256=test", Some(run)).unwrap();
        assert_eq!(prepared.attempts.len(), 1);
        assert_eq!(prepared.attempts[0].members.len(), 1);
        assert_eq!(store.release_prepare_bound(&root, &rel.id, "retry", Some(run)).unwrap().attempts.len(), 1);
        let mut late = store.create(&root, draft("Late addition")).unwrap();
        late.column = "done".into();
        late = store.update(&root, late).unwrap();
        assert_eq!(store.releases(&root).unwrap().releases[0].members.len(), 2);
        assert_eq!(store.releases(&root).unwrap().releases[0].attempts[0].members.len(), 1);
        store.release_outcome_bound(&root, &rel.id, 1, "unknown", "restart receipt lost", Some(run)).unwrap();
        assert_eq!(store.release_outcome_bound(&root, &rel.id, 1, "unknown",
            "restart receipt lost", Some(run)).unwrap().attempts[0].evidence.len(), 2);
        let deployed = store.release_outcome_bound(&root, &rel.id, 1, "deployed",
            "binary hash and live PID verified", Some(run)).unwrap();
        assert_eq!(deployed.attempts[0].status, "deployed");
        assert!(store.release_verify_member(&root, &rel.id, 1, &late.id, true, "not in bundle").is_err());
        let verified = store.release_verify_member(&root, &rel.id, 1, &first.id, true, "targeted UI smoke passed").unwrap();
        assert_eq!(verified.attempts[0].status, "verified");
        assert_eq!(verified.members.iter().find(|m| m.task_id == late.id).unwrap().stage,
            task_core::ReleaseStage::AwaitingIntegration);
        assert_eq!(store.release_verify_member(&root, &rel.id, 1, &first.id, true,
            "targeted UI smoke passed").unwrap().attempts.len(), 1);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn recurring_runs_reopen_and_wait_for_due_date() {
        let root = temp_board("recurring");
        let store = FsTaskStore::default();
        store.ensure_board(&root, "Recurring fixture").unwrap();
        let mut initial = draft("Repeat my audit");
        initial.kind = task_core::TaskKind::Recurring;
        initial.repeat_every_days = Some(3);
        let created = store.create(&root, initial).unwrap();
        assert_eq!(created.run_count, 0);
        let mut completing = created.clone();
        completing.column = "done".into();
        let cycled = store.update(&root, completing).unwrap();
        assert_eq!(cycled.kind, task_core::TaskKind::Recurring);
        assert_eq!(cycled.column, AI_READY_COLUMN_ID);
        assert_eq!(cycled.run_count, 1);
        assert!(cycled.last_run_at.is_some());
        assert!(cycled.next_due.is_some());
        assert!(cycled.completed.is_none());
        assert!(!task_core::recurring_due(&cycled));
        let ready = run_task(&store, &root, &TaskCommand::List(TaskListArgs {
            include_archived: false, column: None, parent: None,
            roots: false, ai_ready: true, live_state: None,
        })).unwrap();
        assert!(ready["tasks"].as_array().unwrap().is_empty());
        assert!(store.claim(&root, &cycled.id, "early-agent").is_err());
        let mut due = cycled.clone();
        due.next_due = Some("2000-01-01".into());
        let due = store.update(&root, due).unwrap();
        assert!(task_core::recurring_due(&due));
        assert_eq!(store.claim(&root, &due.id, "ready-agent").unwrap().claimed_by.as_deref(), Some("ready-agent"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn stale_task_revision_is_rejected() {
        let root = temp_board("stale");
        let store = FsTaskStore::default();
        store.ensure_board(&root, "Test").unwrap();
        let task = store.create(&root, draft("One")).unwrap();

        let path = PathBuf::from(&task.path);
        let original = fs::read_to_string(&path).unwrap();
        fs::write(&path, format!("{original}\nmanual external edit\n")).unwrap();

        let mut stale = task;
        stale.notes = "agent update".into();
        let result = store.update(&root, stale);
        assert!(matches!(result, Err(TaskStoreError::Conflict(_))));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn concurrent_creation_allocates_unique_ids() {
        let root = temp_board("ids");
        FsTaskStore::default().ensure_board(&root, "Test").unwrap();

        let workers = 8;
        let barrier = Arc::new(Barrier::new(workers));
        let handles: Vec<_> = (0..workers)
            .map(|index| {
                let root = root.clone();
                let barrier = barrier.clone();
                thread::spawn(move || {
                    barrier.wait();
                    FsTaskStore::default()
                        .create(&root, draft(&format!("Task {index}")))
                        .unwrap()
                        .id
                })
            })
            .collect();

        let mut ids: Vec<String> = handles.into_iter().map(|handle| handle.join().unwrap()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), workers);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn exact_duplicate_titles_are_ambiguous() {
        let root = temp_board("ambiguous");
        let store = FsTaskStore::default();
        store.ensure_board(&root, "Test").unwrap();
        store.create(&root, draft("Same Name")).unwrap();
        store.create(&root, draft("Same Name")).unwrap();

        let snapshot = store.load(&root).unwrap();
        let error = resolve_task(&snapshot, "Same Name", false).unwrap_err();
        assert_eq!(error.code, "ambiguous_task");

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn unknown_manual_task_content_is_preserved_on_agent_rewrite() {
        let root = temp_board("manual-content");
        let store = FsTaskStore::default();
        store.ensure_board(&root, "Test").unwrap();
        let task = store.create(&root, draft("Manual")).unwrap();

        let path = PathBuf::from(&task.path);
        let text = fs::read_to_string(&path).unwrap();
        let text = text.replace(
            "created: ",
            "custom_field: preserve-me\ncreated: ",
        ) + "\n## Extra Manual Section\n\nDo not delete this.\n";
        fs::write(&path, text).unwrap();

        let snapshot = store.load(&root).unwrap();
        let task = resolve_task(&snapshot, "WT-0001", false).unwrap();
        ensure_task_rewrite_safe(task).unwrap();

        let mut updated = task.clone();
        updated.title = "Updated safely".into();
        store.update(&root, updated).unwrap();

        let after = fs::read_to_string(&path).unwrap();
        assert!(after.contains("custom_field: preserve-me"));
        assert!(after.contains("## Extra Manual Section"));
        assert!(after.contains("Do not delete this."));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn priority_list_precedes_normal_ready_work() {
        let root = temp_board("priority-list");
        let store = FsTaskStore::default();
        store.ensure_board(&root, "Test").unwrap();
        let normal = store.create(&root, draft("Normal")).unwrap();
        let mut urgent_draft = draft("Urgent");
        urgent_draft.attention.push(TaskAttention::Priority);
        let urgent = store.create(&root, urgent_draft).unwrap();
        let result = run_task(&store, &root, &TaskCommand::List(TaskListArgs {
            include_archived: false, column: None, parent: None, roots: false, ai_ready: true, live_state: None,
        })).unwrap();
        assert_eq!(result["tasks"][0]["id"], urgent.id);
        assert_eq!(result["tasks"][1]["id"], normal.id);
        let shown = run_task(&store, &root, &TaskCommand::Show(TaskTarget { target: urgent.id })).unwrap();
        assert_eq!(shown["warnings"][0]["code"], "priority_first");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn typed_task_show_and_claim_emit_advisories_without_blocking_agents() {
        let root = temp_board("work-advisories");
        let store = FsTaskStore::default();
        store.ensure_board(&root, "Test").unwrap();

        for (icon, expected_code, expected_phrase) in [
            ("visual-work", "visual_work_claude_opus_only", "Only Claude Opus"),
            ("visual-implementation", "visual_implementation_approved", "Any suitable agent"),
            ("design", "design_initial_draft_opus_astra_only", "INITIAL design draft"),
        ] {
            let mut draft = draft("Restricted work");
            draft.icon = Some(icon.into());
            let task = store.create(&root, draft).unwrap();

            let show = run_task(
                &store,
                &root,
                &TaskCommand::Show(TaskTarget { target: task.id.clone() }),
            )
            .unwrap();
            assert_eq!(show["task"]["id"], task.id);
            assert_eq!(show["warnings"][0]["code"], expected_code);
            assert_eq!(show["warnings"][0]["advisory_only"], true);
            assert!(show["warnings"][0]["message"].as_str().unwrap().contains(expected_phrase));
            assert_eq!(store.load(&root).unwrap().tasks.iter()
                .find(|candidate| candidate.id == task.id).unwrap().claimed_by, None);

            // The warning must not reject an ineligible model/agent identifier.
            let claim = run_task(
                &store,
                &root,
                &TaskCommand::Claim(TaskClaimArgs {
                    target: task.id.clone(),
                    agent: "generic-non-claude-non-opus".into(),
                    managed: false,
                    claim_token: None,
                }),
            )
            .unwrap();
            assert_eq!(claim["id"], task.id);
            assert_eq!(claim["claimed_by"], "generic-non-claude-non-opus");
            assert_eq!(claim["warnings"][0]["code"], expected_code);
            assert_eq!(claim["warnings"][0]["advisory_only"], true);
            assert!(store.release_claim(&root, &task.id, "generic-non-claude-non-opus").is_ok());
        }

        let ordinary = store.create(&root, draft("Ordinary work")).unwrap();
        let show = run_task(
            &store,
            &root,
            &TaskCommand::Show(TaskTarget { target: ordinary.id.clone() }),
        )
        .unwrap();
        assert!(show.get("warnings").is_none());
        let claim = run_task(
            &store,
            &root,
            &TaskCommand::Claim(TaskClaimArgs {
                target: ordinary.id,
                agent: "any-agent".into(),
                    managed: false,
                    claim_token: None,
            }),
        )
        .unwrap();
        assert!(claim.get("warnings").is_none());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn claims_are_exclusive_idempotent_and_owner_released() {
        let root = temp_board("claims");
        let store = FsTaskStore::default();
        store.ensure_board(&root, "Test").unwrap();
        let task = store.create(&root, draft("Claimable")).unwrap();

        let claimed = store.claim(&root, &task.id, "agent-a").unwrap();
        assert_eq!(claimed.claimed_by.as_deref(), Some("agent-a"));

        let same = store.claim(&root, &task.id, "agent-a").unwrap();
        assert_eq!(same.claimed_by.as_deref(), Some("agent-a"));

        let error = store.claim(&root, &task.id, "agent-b").unwrap_err();
        let mapped = map_store_error(error);
        assert_eq!(mapped.code, "already_claimed");
        assert_eq!(mapped.details["claimed_by"], "agent-a");

        let error = store.release_claim(&root, &task.id, "agent-b").unwrap_err();
        assert_eq!(map_store_error(error).code, "already_claimed");

        let released = store.release_claim(&root, &task.id, "agent-a").unwrap();
        assert_eq!(released.claimed_by, None);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn claims_follow_agent_moves_and_manual_in_progress_is_unclaimed() {
        let root = temp_board("claim-column-sync");
        let store = FsTaskStore::default();
        store.ensure_board(&root, "Claim columns").unwrap();
        let task = store.create(&root, draft("Move through columns")).unwrap();
        let move_to = |column: &str, agent: Option<&str>| TaskCommand::Move(TaskMoveArgs {
            target: task.id.clone(), column: column.into(), agent: agent.map(str::to_owned),
            managed: false, blocked_reason: None, confirm_genuine_block: None,
        });

        assert_eq!(run_task(&store, &root, &move_to("in-progress", None)).unwrap_err().code, "agent_required");
        let started = run_task(&store, &root, &move_to("in-progress", Some("agent-a"))).unwrap();
        assert_eq!(started["column"], "in-progress");
        assert_eq!(started["claimed_by"], "agent-a");
        let stopped = run_task(&store, &root, &move_to("ai-ready", None)).unwrap();
        assert_eq!(stopped["column"], "ai-ready");
        assert!(stopped["claimed_by"].is_null());

        let mut human = store.load(&root).unwrap().tasks.into_iter()
            .find(|item| item.id == task.id).unwrap();
        human.column = "in-progress".into();
        let manual = store.update_as(&root, human, task_core::MutationActor::Human).unwrap();
        assert!(manual.claimed_by.is_none());
        let mismatches = store.reconcile_claim_columns(&root, false).unwrap();
        assert_eq!(mismatches.len(), 1);
        store.reconcile_claim_columns(&root, true).unwrap();
        let ready = store.load(&root).unwrap().tasks.into_iter()
            .find(|item| item.id == task.id).unwrap();
        assert_eq!(ready.column, "ai-ready");

        let managed = store.claim_managed(&root, &task.id, "agent-a").unwrap();
        let finished = store.complete_managed(&root, &task.id, "agent-a", &managed.claim_token, &managed.task.revision).unwrap();
        assert_eq!(finished.column, "done");
        assert!(finished.claimed_by.is_none());
        assert!(!fs::read_to_string(root.join("Active").join(format!("{}.md", task.id))).unwrap()
            .contains("claim_token:"));
        let _ = fs::remove_dir_all(root);
    }
}

// Agent-facing CLI gate integration: the exact JSON error carries the one-use retry instruction.
// Tauri's human update is intentionally a different entrypoint and remains exempt.
#[cfg(test)]
mod rolling_release_cli_tests {
    use super::*;
    use clap::Parser;
    use std::{fs, time::{SystemTime, UNIX_EPOCH}};
    fn board() -> PathBuf {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!("taskboard-release-cli-{}-{nonce}", process::id()));
        FsTaskStore::default().ensure_board(&path, "Gate QA").unwrap();
        path
    }
    fn parse(root: &Path, args: &[&str]) -> Cli {
        let mut all = vec!["taskboard".to_string(), "--root".to_string(), root.to_string_lossy().into_owned()];
        all.extend(args.iter().map(|s| s.to_string()));
        Cli::try_parse_from(all).unwrap()
    }
    #[test]
    fn blocked_move_update_and_create_require_a_one_time_self_review() {
        let root = board();
        let cli = parse(&root, &["task","create","--title","Feature"]);
        let task = run(&cli).unwrap();
        let id = task["id"].as_str().unwrap();
        let first = parse(&root, &["task","move",id,"blocked","--blocked-reason","waiting for source mesh"]);
        let denied = run(&first).unwrap_err();
        assert_eq!(denied.code, "blocked_confirmation_required");
        assert!(denied.message.contains("deployment"));
        let token = denied.details["token"].as_str().unwrap().to_string();
        let wrong = parse(&root, &["task","move",id,"blocked","--blocked-reason","restart pending","--confirm-genuine-block",&token]);
        assert_eq!(run(&wrong).unwrap_err().code, "blocked_confirmation_invalid");
        let confirmed = parse(&root, &["task","move",id,"blocked","--blocked-reason","waiting for source mesh","--confirm-genuine-block",&token]);
        assert_eq!(run(&confirmed).unwrap()["column"], "blocked");
        assert_eq!(run(&confirmed).unwrap_err().code, "blocked_confirmation_invalid");

        let new = parse(&root, &["task","create","--title","Blocked from outset","--column","blocked","--blocked-reason","missing API"]);
        let challenge = run(&new).unwrap_err();
        assert_eq!(challenge.code, "blocked_confirmation_required");
        let token2 = challenge.details["token"].as_str().unwrap();
        let create = parse(&root, &["task","create","--title","Blocked from outset","--column","blocked","--blocked-reason","missing API","--confirm-genuine-block",token2]);
        assert_eq!(run(&create).unwrap()["column"], "blocked");

        let in_progress = parse(&root, &["task","create","--title","Other task"]);
        let other = run(&in_progress).unwrap();
        let id2 = other["id"].as_str().unwrap();
        let update = parse(&root, &["task","update",id2,"--add-attention","blocked","--blocked-reason","vendor missing"]);
        assert_eq!(run(&update).unwrap_err().code, "blocked_confirmation_required");
        let from_help = Cli::try_parse_from(["taskboard","task","move","--help"]).err().unwrap().to_string();
        assert!(!from_help.contains("confirm-genuine-block"));
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn release_cli_is_opaque_argument_forwardable_and_ui_store_remains_human() {
        let root = board();
        let configured = parse(&root, &["release", "enable", "true"]);
        assert!(run(&configured).is_ok());
        let create = run(&parse(&root, &["task","create","--title","Implement"])).unwrap();
        let id = create["id"].as_str().unwrap();
        // Ordinary worker completes as before; release enrollment is central.
        assert_eq!(run(&parse(&root, &["task","complete",id])).unwrap()["column"], "done");
        let result = run(&parse(&root, &["release","list"])).unwrap();
        assert_eq!(result["releases"][0]["members"][0]["task_id"], id);
        // UI/human core path may move directly without agent challenge.
        let store = FsTaskStore::default();
        let mut human = store.create(&root, TaskDraft {
            reserved_id: None, reservation_token: None,
            kind: task_core::TaskKind::Task, deployable: None,
            repeat_every_days: None, next_due: None,
            depends_on: vec![], related_to: vec![], title: "Human task".into(), column: "ai-ready".into(),
            parent: None, category: None, icon: None, attention: vec![], question: None, blocked_reason: None, description: String::new(),
            acceptance_criteria: vec![], ai_notes: String::new(), user_notes: String::new(), notes: String::new(),
        }).unwrap();
        human.column = "blocked".into();
        assert_eq!(store.update_as(&root, human, task_core::MutationActor::Human).unwrap().column, "blocked");
        fs::remove_dir_all(root).unwrap();
    }
}
