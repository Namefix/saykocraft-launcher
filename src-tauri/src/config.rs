use std::{env, fs, io, path::{Path, PathBuf}, sync::{OnceLock, RwLock}};
use serde::{Deserialize, Serialize};
use tracing::{debug, error};
use serde_json::Value;

const LAUNCHER_DIR_NAME: &str = ".saykocraft";

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Config {
	install_dir: String,
	assets_dir: String,
	language: String,
	keep_launcher_open: bool,
}

impl Default for Config {
	fn default() -> Self {
		Self {
			install_dir: "$SAYKOCRAFT/instances".to_string(),
			assets_dir: "$SAYKOCRAFT/assets".to_string(),
			language: "en-US".to_string(),
			keep_launcher_open: false,
		}
	}
}

static CONFIG: OnceLock<RwLock<Config>> = OnceLock::new();

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

pub fn ensure_data_dir() -> io::Result<()> {
	// ensure base launcher dir
	let base = app_data_dir()?;
	debug!(path = %base.display(), "Ensuring launcher data directory");
	fs::create_dir_all(&base)?;

	// ensure common subdirectories
	let instances_dir = base.join("instances");
	debug!(path = %instances_dir.display(), "Ensuring instances directory");
	fs::create_dir_all(&instances_dir)?;

	// ensure config file
	let config_path = base.join("config.json");
	if !config_path.exists() {
		write_default_config_file(&config_path)?;
	}
	read_config_file(&config_path)?;

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
	let cfg = cfg_lock.read().map_err(|e| io::Error::new(io::ErrorKind::Other, format!("rwlock poisoned: {}", e)))?;
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
		return Ok(())
	}

	let data = fs::read(path)?;
	let config: Config = serde_json::from_slice(&data)
		.map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("failed to parse config: {}", e)))?;

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
	CONFIG.get().expect("Config not initialized").read().expect("rwlock poisoned").clone()
}

pub fn update_field(key: &str, value: Value) -> Result<Config, String> {
	let lock = CONFIG.get().ok_or_else(|| "Config not initialized".to_string())?;
	let updated_config = {
		let mut cfg = lock.write().map_err(|e| format!("rwlock poisoned: {}", e))?;

		match key {
			"install_dir" => match value {
				Value::String(s) => cfg.install_dir = s,
				_ => return Err("install_dir must be a string".to_string()),
			},
			"assets_dir" => match value {
				Value::String(s) => cfg.assets_dir = s,
				_ => return Err("assets_dir must be a string".to_string()),
			},
			"language" => match value {
				Value::String(s) => cfg.language = s,
				_ => return Err("language must be a string".to_string()),
			},
			"keep_launcher_open" => match value {
				Value::Bool(b) => cfg.keep_launcher_open = b,
				_ => return Err("keep_launcher_open must be a boolean".to_string()),
			},
			_ => return Err(format!("Unknown config key: {}", key)),
		}

		cfg.clone()
	};

	if let Err(err) = save_config() {
		error!(error = %err, "Config saving failed");
	}

	Ok(updated_config)
}

pub fn save_config() -> io::Result<()> {
	let base = app_data_dir()?;
	let config_path = base.join("config.json");
	write_config_file(&config_path)
}