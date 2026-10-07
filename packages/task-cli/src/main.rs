use clap::{Args, Parser, Subcommand};
use serde_json::{json, Value};
use std::{collections::HashSet, fs, path::{Path, PathBuf}, process};
use task_core::{AI_READY_COLUMN_ID, BoardConfig, BoardSnapshot, Column, FsTaskStore, Task, TaskAttention, TaskDraft, TaskQuestion, TaskStore, TaskStoreError};

const WARTRIBES_BOARD: &str = r"B:\GDrive\SharedWithPhone\Obsidian\jjj\Wartribes\Tasks";

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
}

#[derive(Subcommand)]
enum TaskCommand {
    List(TaskListArgs),
    Show(TaskTarget),
    Search(TaskSearchArgs),
    Create(TaskCreateArgs),
    Update(TaskUpdateArgs),
    Move(TaskMoveArgs),
    Parent(TaskParentArgs),
    Complete(TaskTarget),
    Archive(TaskTarget),
    Claim(TaskClaimArgs),
    Release(TaskClaimArgs),
}

#[derive(Subcommand)]
enum ColumnCommand {
    List,
    Create(ColumnCreateArgs),
    Rename(ColumnRenameArgs),
    Reorder(ColumnReorderArgs),
    Ready(ColumnReadyArgs),
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
}

#[derive(Args)]
struct TaskMoveArgs {
    target: String,
    column: String,
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
struct TaskClaimArgs {
    target: String,
    #[arg(long)]
    agent: String,
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

fn main() {
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
    }
}

fn run_task(store: &FsTaskStore, root: &Path, command: &TaskCommand) -> Result<Value, CliError> {
    match command {
        TaskCommand::List(args) => {
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
            let tasks: Vec<&Task> = snapshot.tasks.iter()
                .filter(|task| args.include_archived || !task.archived)
                .filter(|task| column_id.as_ref().is_none_or(|column| &task.column == column))
                .filter(|task| !args.ai_ready || ready_columns.contains(task.column.as_str()))
                .filter(|task| {
                    if args.roots { task.parent.is_none() }
                    else { parent_id.as_ref().is_none_or(|parent| task.parent.as_ref() == Some(parent)) }
                })
                .collect();
            Ok(json!({ "tasks": tasks }))
        }
        TaskCommand::Show(args) => {
            let snapshot = load_checked(store, root)?;
            let task = resolve_task(&snapshot, &args.target, true)?;
            let children: Vec<&Task> = snapshot.tasks.iter()
                .filter(|candidate| candidate.parent.as_deref() == Some(task.id.as_str()))
                .collect();
            Ok(json!({ "task": task, "children": children }))
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
            let snapshot = load_checked(store, root)?;
            let column = match &args.column {
                Some(value) => resolve_column(&snapshot.config, value)?.id.clone(),
                None => default_create_column(&snapshot.config)?.id.clone(),
            };
            let parent = match &args.parent {
                Some(value) => Some(resolve_task(&snapshot, value, false)?.id.clone()),
                None => None,
            };
            let mut attention = parse_attention_values(&args.attention)?;
            let question = args.question.as_ref().map(|prompt| TaskQuestion {
                prompt: prompt.trim().to_string(),
                options: args.options.clone(),
                answer: None,
            });
            if question.is_some() && !attention.contains(&TaskAttention::Question) {
                attention.push(TaskAttention::Question);
            }
            if !args.user_notes.trim().is_empty() && !attention.contains(&TaskAttention::UserNote) {
                attention.push(TaskAttention::UserNote);
            }
            let draft = TaskDraft {
                title: args.title.clone(),
                column,
                parent,
                category: args.category.clone().filter(|value| !value.trim().is_empty()),
                attention,
                question,
                description: args.description.clone(),
                acceptance_criteria: args.acceptance_criteria.clone(),
                ai_notes: args.ai_notes.clone(),
                user_notes: args.user_notes.clone(),
                notes: String::new(),
            };
            Ok(json!(store.create(root, draft).map_err(map_store_error)?))
        }
        TaskCommand::Update(args) => {
            let snapshot = load_checked(store, root)?;
            let mut task = resolve_task(&snapshot, &args.target, false)?.clone();
            ensure_task_rewrite_safe(&task)?;
            if let Some(title) = &args.title { task.title = title.clone(); }
            if let Some(description) = &args.description { task.description = description.clone(); }
            if args.clear_category {
                task.category = None;
            } else if let Some(category) = &args.category {
                task.category = Some(category.trim().to_string()).filter(|value| !value.is_empty());
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
                if !user_notes.trim().is_empty() {
                    task.attention.push(TaskAttention::UserNote);
                }
            }
            for flag in parse_attention_values(&args.add_attention)? {
                if !task.attention.contains(&flag) {
                    task.attention.push(flag);
                }
            }
            for flag in parse_attention_values(&args.remove_attention)? {
                task.attention.retain(|current| current != &flag);
            }
            if args.clear_question {
                task.question = None;
                task.attention.retain(|flag| flag != &TaskAttention::Question);
            } else if let Some(prompt) = &args.question {
                task.question = Some(TaskQuestion {
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
            Ok(json!(store.update(root, task).map_err(map_store_error)?))
        }
        TaskCommand::Move(args) => {
            let snapshot = load_checked(store, root)?;
            let column = resolve_column(&snapshot.config, &args.column)?.id.clone();
            let mut task = resolve_task(&snapshot, &args.target, false)?.clone();
            ensure_task_rewrite_safe(&task)?;
            task.column = column;
            Ok(json!(store.update(root, task).map_err(map_store_error)?))
        }
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
        TaskCommand::Complete(args) => {
            let snapshot = load_checked(store, root)?;
            let mut terminals: Vec<&Column> = snapshot.config.columns.iter().filter(|c| c.terminal).collect();
            terminals.sort_by_key(|c| c.order);
            let terminal = terminals.first()
                .ok_or_else(|| CliError::new("invalid_column", "board has no terminal/completed column"))?;
            let mut task = resolve_task(&snapshot, &args.target, false)?.clone();
            ensure_task_rewrite_safe(&task)?;
            task.column = terminal.id.clone();
            Ok(json!(store.update(root, task).map_err(map_store_error)?))
        }
        TaskCommand::Archive(args) => {
            let snapshot = load_checked(store, root)?;
            let task = resolve_task(&snapshot, &args.target, true)?;
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
            Ok(json!(store.claim(root, &task.id, &args.agent).map_err(map_store_error)?))
        }
        TaskCommand::Release(args) => {
            let snapshot = load_checked(store, root)?;
            let task = resolve_task(&snapshot, &args.target, true)?;
            ensure_task_rewrite_safe(task)?;
            Ok(json!(store.release_claim(root, &task.id, &args.agent).map_err(map_store_error)?))
        }
    }
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
                ai_ready: false,
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
            other => Err(CliError::new(
                "usage_error",
                format!("unknown attention flag {other:?}; expected question, needs-user, user-note, or blocked"),
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
        TaskStoreError::NotFound(value) => CliError::new("task_not_found", format!("task not found: {value}")),
        TaskStoreError::Conflict(message) => CliError::new("write_conflict", message).exit(3),
        TaskStoreError::AlreadyClaimed { task_id, claimed_by } => CliError::new(
            "already_claimed",
            format!("task {task_id} is claimed by {claimed_by}"),
        ).details(json!({ "task_id": task_id, "claimed_by": claimed_by })).exit(3),
        TaskStoreError::Parse(message) => CliError::new("invalid_board_task_data", message).exit(4),
        TaskStoreError::Io(error) => CliError::new("io_error", error.to_string()).exit(5),
        TaskStoreError::Invalid(message) => {
            let lower = message.to_lowercase();
            if lower.contains("cycle") || lower.contains("parent itself") {
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
            println!("{id}\t{column}\t{title}");
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
            title: title.into(),
            column: AI_READY_COLUMN_ID.into(),
            parent: None,
            category: None,
            attention: Vec::new(),
            question: None,
            description: String::new(),
            acceptance_criteria: Vec::new(),
            ai_notes: String::new(),
            user_notes: String::new(),
            notes: String::new(),
        }
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
            ai_ready: true,
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
            column: None,
            parent: None,
            category: None,
            description: String::new(),
            acceptance_criteria: Vec::new(),
            ai_notes: String::new(),
            user_notes: "Make a proposed design first.".into(),
            attention: Vec::new(),
            question: None,
            options: Vec::new(),
        };

        run_task(&store, &root, &TaskCommand::Create(args)).unwrap();
        let task = store.load(&root).unwrap().tasks.into_iter().next().unwrap();
        assert_eq!(task.user_notes, "Make a proposed design first.");
        assert!(task.attention.contains(&TaskAttention::UserNote));

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
}
