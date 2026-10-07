use std::{path::PathBuf, process::Command};
use task_core::{
    BoardConfig, BoardSnapshot, FsTaskStore, Task, TaskDraft, TaskStore,
};

const WARTRIBES_BOARD: &str =
    r"B:\GDrive\SharedWithPhone\Obsidian\jjj\Wartribes\Tasks";
const FOMNI_EXE: &str =
    r"D:\SSDProjects\Tools\Omnish\main\target\release\fomni.exe";
const OBSIDIAN_EXE: &str =
    r"E:\Program Files\Obsidian\Obsidian.exe";

fn root(path: Option<String>) -> PathBuf {
    PathBuf::from(path.unwrap_or_else(|| WARTRIBES_BOARD.to_string()))
}

#[tauri::command]
fn default_board_path() -> String {
    WARTRIBES_BOARD.to_string()
}

#[tauri::command]
fn load_board(path: Option<String>) -> Result<BoardSnapshot, String> {
    FsTaskStore::default()
        .load(&root(path))
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn create_task(path: Option<String>, draft: TaskDraft) -> Result<Task, String> {
    FsTaskStore::default()
        .create(&root(path), draft)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn update_task(path: Option<String>, task: Task) -> Result<Task, String> {
    FsTaskStore::default()
        .update(&root(path), task)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn archive_task(path: Option<String>, id: String) -> Result<Task, String> {
    FsTaskStore::default()
        .archive(&root(path), &id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn save_board_config(path: Option<String>, config: BoardConfig) -> Result<BoardConfig, String> {
    FsTaskStore::default()
        .save_config(&root(path), config)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn restore_snapshot(
    path: Option<String>,
    before: BoardSnapshot,
    after: BoardSnapshot,
) -> Result<BoardSnapshot, String> {
    FsTaskStore::default()
        .restore_snapshot(&root(path), before, after)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn highlight_in_fomni(file: String) -> Result<(), String> {
    Command::new(FOMNI_EXE)
        .arg("--select")
        .arg(file)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("failed to launch Fomni: {e}"))
}

fn percent_encode_query(value: &str) -> String {
    let mut out = String::new();
    for byte in value.as_bytes() {
        match *byte {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'_'
            | b'.'
            | b'~' => out.push(*byte as char),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[tauri::command]
fn open_in_obsidian(file: String) -> Result<(), String> {
    let uri = format!("obsidian://open?path={}", percent_encode_query(&file));
    Command::new(OBSIDIAN_EXE)
        .arg(uri)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("failed to launch Obsidian: {e}"))
}

#[tauri::command]
fn show_main_window(window: tauri::WebviewWindow) -> Result<(), String> {
    window.show().map_err(|e| e.to_string())?;
    window.set_focus().map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            use tauri::Manager;
            if let Some(window) = app.get_webview_window("main") {
                let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/32x32.png"))?;
                window.set_icon(icon)?;
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            default_board_path,
            load_board,
            create_task,
            update_task,
            archive_task,
            save_board_config,
            restore_snapshot,
            highlight_in_fomni,
            open_in_obsidian,
            show_main_window
        ])
        .run(tauri::generate_context!())
        .expect("error while running TaskBoard");
}
