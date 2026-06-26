mod auth;
mod config;
mod instance;
mod minecraft;
mod profile_icon;
mod utils;

use keyring_core::{Entry, Error as KeyringError};
use serde::Serialize;
use serde_json::Value;
use std::{fs, io, path::Path};
use tauri::{AppHandle, Emitter, LogicalSize, RunEvent, Size};
use tracing::{error, info, warn};
use tracing_appender::{non_blocking::WorkerGuard, rolling};
use tracing_subscriber::prelude::*;
use tracing_subscriber::{fmt, EnvFilter};

use auth::AuthError;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Serialize)]
struct InstanceSettingsResponse {
    settings: instance::InstanceSettings,
    instance_location: String,
    minimum_ram_mb: u64,
    recommended_ram_mb: u64,
}

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

#[tauri::command]
async fn get_instance_version(id: String) -> Result<String, String> {
    instance::get_instances()
        .into_iter()
        .find(|entry| entry.id == id)
        .and_then(|entry| {
            entry
                .instance_manifest
                .map(|manifest| manifest.pack_version)
        })
        .ok_or_else(|| format!("Instance '{id}' was not found"))
}

#[tauri::command]
async fn get_instance_settings(id: String) -> Result<InstanceSettingsResponse, String> {
    let manifest = instance::get_instance_manifest(&id)
        .ok_or_else(|| format!("Instance '{id}' is not installed"))?;
    let settings = instance::get_instance_settings(&id).map_err(|e| e.to_string())?;
    let instance_location = instance::installed_instance_dir(&id)
        .map_err(|e| e.to_string())?
        .display()
        .to_string();

    Ok(InstanceSettingsResponse {
        settings,
        instance_location,
        minimum_ram_mb: manifest.minimum_ram_mb,
        recommended_ram_mb: manifest.recommended_ram_mb,
    })
}

#[tauri::command]
async fn update_instance_settings_field(
    id: String,
    key: String,
    value: Value,
) -> Result<instance::InstanceSettings, String> {
    instance::update_instance_settings_field(&id, &key, value)
}

#[tauri::command]
async fn browse_instance(id: String) -> Result<(), String> {
    let instance_dir = instance::installed_instance_dir(&id).map_err(|e| e.to_string())?;
    tauri_plugin_opener::open_path(&instance_dir, None::<&str>).map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_instance_folder_size(id: String) -> Result<u64, String> {
    let instance_dir = instance::installed_instance_dir(&id).map_err(|e| e.to_string())?;

    tauri::async_runtime::spawn_blocking(move || folder_size(&instance_dir))
        .await
        .map_err(|error| format!("Instance folder size task failed: {error}"))?
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn remove_instance(id: String) -> Result<bool, String> {
    tauri::async_runtime::spawn_blocking(move || instance::remove_local_instance(&id))
        .await
        .map_err(|error| format!("Instance remove task failed: {error}"))?
        .map_err(|error| error.to_string())
}

fn folder_size(path: &Path) -> io::Result<u64> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_file() {
        return Ok(metadata.len());
    }

    let mut size = 0;
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let metadata = entry.path().symlink_metadata()?;

        if metadata.is_dir() {
            size += folder_size(&entry.path())?;
        } else if metadata.is_file() {
            size += metadata.len();
        }
    }

    Ok(size)
}

#[tauri::command]
async fn fetch_remote_instance(id: String) -> Result<(), String> {
    instance::fetch_remote_instance(&id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn ensure_instance(app: AppHandle, id: String) -> Result<(), String> {
    let install_state = match instance::get_instance_state(&id) {
        instance::InstanceState::RequiresUpdate | instance::InstanceState::Updating => {
            instance::InstanceState::Updating
        }
        _ => instance::InstanceState::Downloading,
    };
    instance::set_instance_state_override(&id, install_state).map_err(|e| e.to_string())?;

    let progress_app = app.clone();
    let progress_sink = move |progress: minecraft::InstallProgress| {
        if let Err(error) = progress_app.emit("instance-install-progress", progress) {
            warn!(%error, "Failed to emit instance install progress");
        }
    };

    let result = minecraft::install::ensure_instance_with_progress_by_id(&id, &progress_sink)
        .await
        .map_err(|error| error.to_string());

    if let Err(error) = instance::clear_instance_state_override(&id) {
        warn!(%error, id = %id, "Failed to clear instance state override");
    }

    result
}

#[tauri::command]
async fn launch_instance(
    id: String,
    options: Option<minecraft::LaunchOptions>,
) -> Result<minecraft::LaunchResult, String> {
    let manifest = instance::get_instance_manifest(&id)
        .ok_or_else(|| format!("Instance '{id}' is not installed"))?;
    let instance_settings = instance::get_instance_settings(&id).map_err(|e| e.to_string())?;
    let mut options = options.unwrap_or_default();
    if options.max_memory_mb.is_none() {
        options.max_memory_mb = Some(instance_settings.maximum_ram_mb);
    }
    let mut extra_jvm_args = instance_settings.additional_jvm_args;
    extra_jvm_args.extend(options.extra_jvm_args);
    options.extra_jvm_args = extra_jvm_args;

    if options
        .username
        .as_deref()
        .map(str::trim)
        .filter(|username| !username.is_empty())
        .is_none()
    {
        options.username = get_username().await?;
    }

    tauri::async_runtime::spawn_blocking(move || {
        minecraft::launch::launch_instance(&manifest, options)
    })
    .await
    .map_err(|error| format!("Minecraft launch task failed: {error}"))?
    .map_err(|error| error.to_string())
}

#[tauri::command]
async fn stop_instance(id: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || minecraft::launch::stop_instance(&id))
        .await
        .map_err(|error| format!("Minecraft stop task failed: {error}"))?
        .map_err(|error| error.to_string())
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

    if let Err(e) = config::ensure_base_dir() {
        error!("Failed to ensure data directory: {e}");
    }

    tauri::Builder::default()
        .setup(|app| {
            instance::register_instance_event_app(app.handle().clone());

            if let Err(error) = tauri::async_runtime::block_on(instance::init_instances()) {
                error!(%error, "Failed to initialize instances");
            }

            Ok(())
        })
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
            get_instance_version,
            get_instance_settings,
            update_instance_settings_field,
            browse_instance,
            get_instance_folder_size,
            remove_instance,
            fetch_remote_instance,
            ensure_instance,
            launch_instance,
            stop_instance,
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
