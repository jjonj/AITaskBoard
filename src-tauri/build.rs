fn main() {
    // Fail the build instead of shipping a production EXE that tries to load
    // http://localhost:1420. Tauri development builds are unaffected.
    if std::env::var("PROFILE").as_deref() == Ok("release")
        && std::env::var_os("CARGO_FEATURE_CUSTOM_PROTOCOL").is_none()
        && std::env::var_os("CARGO_FEATURE_TAURI_CUSTOM_PROTOCOL").is_none()
    {
        panic!("TaskBoard production build requires custom-protocol with embedded UI assets. Use cargo build --release or npm run tauri build; do not pass --no-default-features.");
    }
    tauri_build::build()
}
