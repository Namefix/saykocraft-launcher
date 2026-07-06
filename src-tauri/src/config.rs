use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    env, fs, io,
    path::{Path, PathBuf},
    sync::{OnceLock, RwLock},
};
use tracing::{debug, error};

const LAUNCHER_DIR_NAME: &str = ".saykocraft";
const LAUNCHER_PATH_VARIABLE: &str = "$SAYKOCRAFT";

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Config {
    install_dir: String,
    data_dir: String,
    language: String,
    keep_launcher_open: bool,
    #[serde(default = "prefer_discrete_gpu_default")]
    prefer_discrete_gpu: bool,
}

impl Config {
    pub fn resolved_install_dir(&self) -> io::Result<PathBuf> {
        resolve_path(&self.install_dir)
    }

    pub fn resolved_data_dir(&self) -> io::Result<PathBuf> {
        resolve_path(&self.data_dir)
    }

    pub fn keep_launcher_open(&self) -> bool {
        self.keep_launcher_open
    }

    pub fn prefer_discrete_gpu(&self) -> bool {
        self.prefer_discrete_gpu
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            install_dir: "$SAYKOCRAFT/instances".to_string(),
            data_dir: "$SAYKOCRAFT/data".to_string(),
            language: "tr-TR".to_string(),
            keep_launcher_open: false,
            prefer_discrete_gpu: prefer_discrete_gpu_default(),
        }
    }
}

static CONFIG: OnceLock<RwLock<Config>> = OnceLock::new();

fn prefer_discrete_gpu_default() -> bool {
    true
}

pub fn app_data_dir() -> io::Result<PathBuf> {
    // Compiling for Windows
    #[cfg(target_os = "windows")]
    {
        let appdata = env::var_os("APPDATA")
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "APPDATA is not set"))?;

        return Ok(PathBuf::from(appdata).join(LAUNCHER_DIR_NAME));
    }

    // Compiling for LUUUNIX
    #[cfg(not(target_os = "windows"))]
    {
        let home = env::var_os("HOME")
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is not set"))?;

        Ok(PathBuf::from(home).join(LAUNCHER_DIR_NAME))
    }
}

pub fn launcher_log_dir() -> io::Result<PathBuf> {
    Ok(app_data_dir()?.join("logs"))
}

pub fn ensure_base_dir() -> io::Result<()> {
    // ensure base launcher dir
    let base = app_data_dir()?;
    debug!(path = %base.display(), "Ensuring launcher data directory");
    fs::create_dir_all(&base)?;

    let log_dir = launcher_log_dir()?;
    debug!(path = %log_dir.display(), "Ensuring launcher log directory");
    fs::create_dir_all(log_dir)?;

    // ensure config file
    let config_path = base.join("config.json");
    if !config_path.exists() {
        write_default_config_file(&config_path)?;
    }
    read_config_file(&config_path)?;

    let config = get_config();
    let install_dir = config.resolved_install_dir()?;
    debug!(path = %install_dir.display(), "Ensuring instances directory");
    fs::create_dir_all(install_dir)?;

    let data_dir = config.resolved_data_dir()?;
    debug!(path = %data_dir.display(), "Ensuring data directory");
    fs::create_dir_all(data_dir)?;

    Ok(())
}

pub fn resolve_path(value: &str) -> io::Result<PathBuf> {
    let base = app_data_dir()?;
    resolve_path_from(value, &base)
}

fn resolve_path_from(value: &str, base: &Path) -> io::Result<PathBuf> {
    if value.trim().is_empty() {
        return Err(invalid_path("path cannot be empty"));
    }

    if value.contains('\0') {
        return Err(invalid_path("path cannot contain null bytes"));
    }

    if value == LAUNCHER_PATH_VARIABLE {
        return Ok(base.to_path_buf());
    }

    if let Some(remainder) = value.strip_prefix(LAUNCHER_PATH_VARIABLE) {
        if !remainder.starts_with(['/', '\\']) {
            return Err(invalid_path("$SAYKOCRAFT must be followed by '/' or '\\'"));
        }

        let mut resolved = base.to_path_buf();
        for component in remainder.split(['/', '\\']) {
            match component {
                "" | "." => continue,
                ".." => {
                    return Err(invalid_path("$SAYKOCRAFT paths cannot contain '..'"));
                }
                part => {
                    validate_launcher_component(part)?;
                    resolved.push(part);
                }
            }
        }

        return Ok(resolved);
    }

    if value.contains(LAUNCHER_PATH_VARIABLE) {
        return Err(invalid_path(
            "$SAYKOCRAFT may only appear at the beginning of a path",
        ));
    }

    let path = PathBuf::from(value);
    if !path.is_absolute() {
        return Err(invalid_path(
            "path must be absolute or start with $SAYKOCRAFT",
        ));
    }

    Ok(path)
}

fn invalid_path(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

fn validate_launcher_component(component: &str) -> io::Result<()> {
    if component.chars().any(|character| {
        character.is_control() || matches!(character, '<' | '>' | ':' | '"' | '|' | '?' | '*')
    }) {
        return Err(invalid_path(
            "$SAYKOCRAFT paths contain an invalid path component",
        ));
    }

    Ok(())
}

pub fn write_default_config_file(path: &Path) -> io::Result<()> {
    if path.exists() {
        debug!(path = %path.display(), "Config file already exists, skipping write");
        return Ok(());
    }

    let config = Config::default();
    let content = serde_json::to_string_pretty(&config)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("serialize error: {}", e)))?;

    fs::write(path, content)?;
    debug!(path = %path.display(), "Written default config file");
    Ok(())
}

pub fn write_config_file(path: &Path) -> io::Result<()> {
    let cfg_lock = CONFIG.get().expect("Config not initialized");
    let cfg = cfg_lock
        .read()
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("rwlock poisoned: {}", e)))?;
    let content = serde_json::to_string_pretty(&*cfg)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("serialize error: {}", e)))?;

    // atomic write
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, content)?;
    fs::rename(&tmp, path)?;
    debug!(path = %path.display(), "Written config file");
    Ok(())
}

fn read_config_file(path: &Path) -> io::Result<()> {
    if !path.exists() {
        error!(path = %path.display(), "Config does not exist!");
        return Ok(());
    }

    let data = fs::read(path)?;
    let config: Config = serde_json::from_slice(&data).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("failed to parse config: {}", e),
        )
    })?;

    resolve_path(&config.install_dir).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid install_dir: {}", e),
        )
    })?;
    resolve_path(&config.data_dir).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid data_dir: {}", e),
        )
    })?;

    if let Err(_) = CONFIG.set(RwLock::new(config.clone())) {
        if let Some(lock) = CONFIG.get() {
            if let Ok(mut w) = lock.write() {
                *w = config;
            } else {
                error!("Failed to acquire write lock to update config");
            }
        } else {
            error!("Config OnceLock reported set error but get() returned None");
        }
    }

    Ok(())
}

/* doubt it'll be useful but lets keep it anyways :^
pub fn set_config(config: Config) -> Result<(), RwLock<Config>> {
    CONFIG.set(RwLock::new(config))
}
*/

pub fn get_config() -> Config {
    CONFIG
        .get()
        .expect("Config not initialized")
        .read()
        .expect("rwlock poisoned")
        .clone()
}

pub fn update_field(key: &str, value: Value) -> Result<Config, String> {
    let lock = CONFIG
        .get()
        .ok_or_else(|| "Config not initialized".to_string())?;
    let previous_config;
    let updated_config = {
        let mut cfg = lock
            .write()
            .map_err(|e| format!("rwlock poisoned: {}", e))?;
        previous_config = cfg.clone();

        match key {
            "install_dir" => match value {
                Value::String(s) => {
                    resolve_path(&s).map_err(|e| format!("invalid install_dir: {}", e))?;
                    cfg.install_dir = s;
                }
                _ => return Err("install_dir must be a string".to_string()),
            },
            "data_dir" => match value {
                Value::String(s) => {
                    resolve_path(&s).map_err(|e| format!("invalid data_dir: {}", e))?;
                    cfg.data_dir = s;
                }
                _ => return Err("data_dir must be a string".to_string()),
            },
            "language" => match value {
                Value::String(s) => cfg.language = s,
                _ => return Err("language must be a string".to_string()),
            },
            "keep_launcher_open" => match value {
                Value::Bool(b) => cfg.keep_launcher_open = b,
                _ => return Err("keep_launcher_open must be a boolean".to_string()),
            },
            "prefer_discrete_gpu" => match value {
                Value::Bool(b) => cfg.prefer_discrete_gpu = b,
                _ => return Err("prefer_discrete_gpu must be a boolean".to_string()),
            },
            _ => return Err(format!("Unknown config key: {}", key)),
        }

        cfg.clone()
    };

    if let Err(err) = save_config() {
        error!(error = %err, "Config saving failed");
        let mut cfg = lock
            .write()
            .map_err(|e| format!("rwlock poisoned while reverting failed save: {}", e))?;
        *cfg = previous_config;
        return Err(format!("Config saving failed: {}", err));
    }

    Ok(updated_config)
}

pub fn save_config() -> io::Result<()> {
    let base = app_data_dir()?;
    let config_path = base.join("config.json");
    write_config_file(&config_path)
}
