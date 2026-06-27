use std::{
    collections::{HashMap, VecDeque},
    sync::{Mutex, OnceLock},
};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tracing::{debug, warn};

pub const DEV_CONSOLE_WINDOW_LABEL: &str = "dev-console";
pub const CONSOLE_LINE_EVENT: &str = "minecraft-console-line";
pub const CONSOLE_STATUS_EVENT: &str = "minecraft-console-status";
const CONSOLE_HISTORY_LIMIT: usize = 3000;

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsoleStream {
    Stdout,
    Stderr,
    System,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsoleProcessStatus {
    Starting,
    Started,
    Exited,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsoleLine {
    pub instance_id: String,
    pub stream: ConsoleStream,
    pub line: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsoleStatus {
    pub instance_id: String,
    pub status: ConsoleProcessStatus,
    pub pid: Option<u32>,
    pub exit_code: Option<i32>,
    pub success: Option<bool>,
    pub log_path: Option<String>,
}

static CONSOLE_EVENT_APP: OnceLock<AppHandle> = OnceLock::new();
static CONSOLE_HISTORY: OnceLock<Mutex<HashMap<String, VecDeque<ConsoleLine>>>> = OnceLock::new();

pub fn register_console_event_app(app: AppHandle) {
    if CONSOLE_EVENT_APP.set(app).is_err() {
        debug!("Console event app is already registered");
    }
}

pub fn clear_history(instance_id: &str) {
    let history = CONSOLE_HISTORY.get_or_init(|| Mutex::new(HashMap::new()));
    let Ok(mut history) = history.lock() else {
        warn!(
            instance_id,
            "Failed to lock Minecraft console history for clear"
        );
        return;
    };

    history.remove(instance_id);
}

pub fn get_history(instance_id: &str) -> Vec<ConsoleLine> {
    CONSOLE_HISTORY
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .ok()
        .and_then(|history| history.get(instance_id).cloned())
        .map(VecDeque::into_iter)
        .map(Iterator::collect)
        .unwrap_or_default()
}

pub fn emit_line(instance_id: &str, stream: ConsoleStream, line: impl Into<String>) {
    let payload = ConsoleLine {
        instance_id: instance_id.to_string(),
        stream,
        line: line.into(),
    };

    push_history(payload.clone());
    emit_to_console_window(CONSOLE_LINE_EVENT, payload);
}

pub fn emit_status(payload: ConsoleStatus) {
    emit_to_console_window(CONSOLE_STATUS_EVENT, payload);
}

fn push_history(line: ConsoleLine) {
    let history = CONSOLE_HISTORY.get_or_init(|| Mutex::new(HashMap::new()));
    let Ok(mut history) = history.lock() else {
        warn!(
            instance_id = %line.instance_id,
            "Failed to lock Minecraft console history for append"
        );
        return;
    };

    let lines = history.entry(line.instance_id.clone()).or_default();
    lines.push_back(line);

    while lines.len() > CONSOLE_HISTORY_LIMIT {
        lines.pop_front();
    }
}

fn emit_to_console_window<S>(event: &str, payload: S)
where
    S: Serialize + Clone,
{
    let Some(app) = CONSOLE_EVENT_APP.get() else {
        return;
    };

    let Some(window) = app.get_webview_window(DEV_CONSOLE_WINDOW_LABEL) else {
        return;
    };

    if let Err(error) = window.emit(event, payload) {
        warn!(%error, event, "Failed to emit Minecraft console event");
    }
}
