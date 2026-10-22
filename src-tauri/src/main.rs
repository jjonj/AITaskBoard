// This is intentionally a console-subsystem executable so CLI invocations
// block until completion and provide stdout/stderr and reliable exit codes.
// GUI mode immediately detaches from the console before Tauri starts.
#[cfg(windows)]
extern "system" {
    fn FreeConsole() -> i32;
}

fn main() {
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
