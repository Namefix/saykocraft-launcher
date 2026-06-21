mod auth;
mod config;
mod instance;
mod profile_icon;
mod utils;

use keyring_core::{Entry, Error as KeyringError};
use serde_json::Value;
use tauri::{AppHandle, Emitter, LogicalSize, RunEvent, Size};
use tracing::{error, info, warn};
use tracing_appender::{non_blocking::WorkerGuard, rolling};
use tracing_subscriber::prelude::*;
use tracing_subscriber::{fmt, EnvFilter};

use auth::AuthError;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

#[tauri::command]
fn window_close(window: tauri::WebviewWindow) {
    let _ = window.close();
}

#[tauri::command]
fn window_minimize(window: tauri::WebviewWindow) {
    let _ = window.minimize();
}

#[tauri::command]
fn set_launcher_window(window: tauri::WebviewWindow) -> Result<(), String> {
    let size = Size::Logical(LogicalSize::new(1280.0, 768.0));
    window.set_size(size).map_err(|e| e.to_string())?;
    window
        .set_min_size(Some(Size::Logical(LogicalSize::new(1280.0, 768.0))))
        .map_err(|e| e.to_string())?;
    window.center().map_err(|e| e.to_string())?;

    Ok(())
}

#[tauri::command]
fn set_login_window(window: tauri::WebviewWindow) -> Result<(), String> {
    let size = Size::Logical(LogicalSize::new(512.0, 640.0));
    window.set_size(size).map_err(|e| e.to_string())?;
    window
        .set_min_size(Some(Size::Logical(LogicalSize::new(512.0, 640.0))))
        .map_err(|e| e.to_string())?;
    window.center().map_err(|e| e.to_string())?;

    Ok(())
}

#[tauri::command]
async fn get_username() -> Result<Option<String>, String> {
    let username_entry = Entry::new("saykocraft-launcher", "username")
        .map_err(|e| format!("Couldn't access keyring: {}", e))?;

    match username_entry.get_password() {
        Ok(username) => Ok(Some(username)),
        Err(KeyringError::NoEntry) => Ok(None),
        Err(e) => Err(format!("Failed to read username: {}", e)),
    }
}

#[tauri::command]
fn get_launcher_version() -> String {
    return VERSION.to_string();
}

#[tauri::command]
async fn get_profile_icon(app: AppHandle) -> Result<Option<String>, String> {
    let Some(username) = get_username().await? else {
        return Ok(None);
    };

    let icon = profile_icon::get_profile_icon(&app, &username)
        .await
        .map_err(|status| format!("Failed to get profile icon: {status}"))?;

    Ok(Some(icon))
}

#[tauri::command]
async fn reset_profile_icon_cache(app: AppHandle) -> Result<(), String> {
    profile_icon::reset_profile_icon_cache(&app)
        .map_err(|status| format!("Failed to reset profile icon cache: {status}"))
}

#[tauri::command]
async fn check_session(app: tauri::AppHandle) -> Result<(), String> {
    try_extend_session(app).await
}

async fn try_extend_session(app: AppHandle) -> Result<(), String> {
    let Some(session_token) = auth::get_session_token().await? else {
        app.emit("session-status", "null")
            .map_err(|e| e.to_string())?;
        return Ok(());
    };

    match auth::extend_session(&session_token).await {
        Ok(true) => {
            info!("Session extended");
            // proceed to launcher
            app.emit("session-status", "valid")
                .map_err(|e| e.to_string())?;
        }
        Ok(false) => {
            info!("Session expired or invalid");
            // stay on login page
            app.emit("session-status", "invalid")
                .map_err(|e| e.to_string())?;
        }
        Err(AuthError::Network(message)) => {
            warn!("Failed to extend session: {message}");
            app.emit("session-status", "network-error")
                .map_err(|e| e.to_string())?;
        }
        Err(e) => {
            warn!(?e, "Failed to extend session");
            app.emit("session-status", "invalid")
                .map_err(|e| e.to_string())?;
        }
    }

    Ok(())
}

#[tauri::command]
async fn reset_data(app: tauri::AppHandle) {
    let token_entry = match Entry::new("saykocraft-launcher", "session") {
        Ok(entry) => entry,
        Err(e) => {
            warn!("Couldn't access keyring: {}", e);
            let ___ = reset_profile_icon_cache(app).await;
            return;
        }
    };

    let username_entry = match Entry::new("saykocraft-launcher", "username") {
        Ok(entry) => entry,
        Err(e) => {
            warn!("Couldn't access keyring: {}", e);
            let ___ = reset_profile_icon_cache(app).await;
            return;
        }
    };

    match token_entry.get_password() {
        Ok(token) => {
            let _ = auth::logout_session(&token).await;
        }
        Err(e) => warn!(err = %e, "Failed to read token, not sending logout request"),
    }

    let _ = token_entry.delete_credential();
    let __ = username_entry.delete_credential();

    let ___ = reset_profile_icon_cache(app).await;
}

#[tauri::command]
async fn get_config() -> config::Config {
    config::get_config()
}

#[tauri::command]
async fn save_config() -> Result<(), String> {
    config::save_config().map_err(|e| e.to_string())
}

#[tauri::command]
async fn update_config_field(key: String, value: Value) -> Result<config::Config, String> {
    config::update_field(&key, value).map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_instance_state(id: String) -> u8 {
    instance::get_instance_state(&id) as u8
}

fn init_tracing() -> WorkerGuard {
    let file_appender = rolling::daily("./logs", "launcher.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    let console_layer = fmt::layer().with_writer(std::io::stdout).with_ansi(true);

    let file_layer = fmt::layer().with_writer(non_blocking).with_ansi(false);

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    let subscriber = tracing_subscriber::registry()
        .with(filter)
        .with(console_layer)
        .with(file_layer);

    if tracing::subscriber::set_global_default(subscriber).is_err() {
        eprintln!("Tracing already initialized");
    }

    guard
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let _log_guard = init_tracing();

    info!("Starting SayKOCraft Launcher");

    if let Err(e) = keyring::use_native_store(true) {
        error!("Failed to initialize keyring store: {e}");
    }

    if let Err(e) = config::ensure_data_dir() {
        error!("Failed to ensure data directory: {e}");
    }

    if let Err(error) = tauri::async_runtime::block_on(instance::init_instances()) {
        error!(%error, "Failed to initialize instances");
    }

    tauri::Builder::default()
        .plugin(
            tauri_plugin_log::Builder::new()
                .targets([
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Stdout),
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::LogDir {
                        file_name: Some("app.log".to_string()),
                    }),
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Webview),
                ])
                .build(),
        )
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            greet,
            window_close,
            window_minimize,
            set_launcher_window,
            auth::login,
            auth::get_session_token,
            check_session,
            reset_data,
            get_username,
            get_profile_icon,
            reset_profile_icon_cache,
            get_launcher_version,
            set_login_window,
            get_config,
            save_config,
            update_config_field,
            get_instance_state,
        ])
        .build(tauri::generate_context!())
        .expect("error while running tauri application")
        .run(|_app_handle, event| match event {
            RunEvent::Exit => {
                if let Err(err) = config::save_config() {
                    error!(error = %err, "Config saving failed");
                }
            }
            _ => {}
        });
}
