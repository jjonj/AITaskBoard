@echo off
setlocal
cargo run --quiet --manifest-path "%~dp0packages\task-cli\Cargo.toml" -- %*
