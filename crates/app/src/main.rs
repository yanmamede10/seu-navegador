#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(target_os = "windows")]
mod win32;

#[cfg(target_os = "windows")]
fn main() {
    use seu_engine::cef_backend::CefEngine;
    use seu_engine::EngineConfig;
    use seu_storage::Storage;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use tracing::{error, info, warn};

    // Must run before any logging/storage initialization so CEF subprocesses exit cleanly.
    if let Some(exit_code) = CefEngine::execute_subprocess_if_needed() {
        std::process::exit(exit_code);
    }

    let base_dir = app_data_dir();
    init_logging(&base_dir.join("logs"));
    info!("Starting Seu Navegador v{}", env!("CARGO_PKG_VERSION"));

    let db_path = base_dir.join("seu-navegador.db");
    let storage = match Storage::open(&db_path) {
        Ok(db) => Some(db),
        Err(err) => {
            warn!("Could not open SQLite database at {:?}: {err}", db_path);
            None
        }
    };
    let initial_settings = win32::load_persisted_settings(&storage);
    let shared_storage = Arc::new(Mutex::new(storage));

    let host = Arc::new(win32::Win32WindowHost::new(shared_storage.clone()));
    let config = EngineConfig {
        root_cache_path: base_dir.join("profile"),
        user_agent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36".to_string(),
        enable_sandbox: false,
    };

    if let Err(err) = CefEngine::run(config, initial_settings, host, shared_storage) {
        error!("Fatal engine error: {err}");
        std::process::exit(1);
    }

    fn app_data_dir() -> PathBuf {
        let root = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        let dir = root.join("SeuNavegador");
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    fn init_logging(log_dir: &PathBuf) {
        let _ = std::fs::create_dir_all(log_dir);
        let log_file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_dir.join("seu-navegador.log"));

        let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

        match log_file {
            Ok(file) => {
                let _ = tracing_subscriber::fmt()
                    .with_env_filter(env_filter)
                    .with_ansi(false)
                    .with_writer(Mutex::new(file))
                    .try_init();
            }
            Err(_) => {
                let _ = tracing_subscriber::fmt()
                    .with_env_filter(env_filter)
                    .try_init();
            }
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("Seu Navegador v1 desktop binary targets Windows x64.");
}
