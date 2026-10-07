use std::{env, path::PathBuf};
use task_core::{FsTaskStore, TaskStore};

fn main() {
    let Some(path) = env::args().nth(1) else {
        eprintln!("usage: inspect_board <board-path>");
        std::process::exit(2);
    };
    let root = PathBuf::from(path);
    match FsTaskStore::default().load(&root) {
        Ok(board) => {
            println!(
                "{}: {} active, {} archived, {} columns, {} warnings",
                board.config.name,
                board.tasks.iter().filter(|task| !task.archived).count(),
                board.tasks.iter().filter(|task| task.archived).count(),
                board.config.columns.len(),
                board.warnings.len()
            );
            for warning in board.warnings {
                println!("warning: {warning}");
            }
        }
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}
