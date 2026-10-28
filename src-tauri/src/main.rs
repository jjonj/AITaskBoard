// This is intentionally a console-subsystem executable so CLI invocations
// block until completion and provide stdout/stderr and reliable exit codes.
// GUI mode immediately detaches from the console before Tauri starts.
#[cfg(windows)]
extern "system" {
    fn FreeConsole() -> i32;
}

fn main() {
    // Human-invoked native Review deep link from T3. A dedicated TaskBoard
    // window opens at the exact board and item; never resolves a verdict here.
    let args: Vec<_> = std::env::args().collect();
    if args.len() == 4 && args[1] == "--open-review" {
        let payload = serde_json::json!({"path": args[2], "taskId": args[3]});
        std::env::set_var("TASKBOARD_START_REVIEW", payload.to_string());
        #[cfg(windows)] unsafe { FreeConsole(); }
        taskboard_lib::run();
        return;
    }
    // No args opens the desktop UI. Any CLI arguments use the exact same
    // implementation as packages/task-cli, including --help and --json.
    if std::env::args_os().len() > 1 {
        taskboard_cli::entry();
    } else {
        #[cfg(windows)]
        unsafe {
            FreeConsole();
        }
        taskboard_lib::run();
    }
}
