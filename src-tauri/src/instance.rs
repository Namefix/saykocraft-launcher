use std::{
    cmp::Ordering,
    collections::HashMap,
    fmt, fs, io,
    path::Path,
    sync::{OnceLock, RwLock},
    time::Duration,
};

use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};
use tracing::{debug, error, info, warn};

const CDN_URL: &str = "http://localhost:3001";
const INSTANCE_STATE_CHANGED_EVENT: &str = "instance-state-changed";

const INSTANCE_MANIFEST_FILE: &str = "instance.json";
const REMOTE_INSTANCE_IDS: &[&str] = &["saykocraft-earth"];
const REMOTE_INSTANCE_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const SUPPORTED_INSTANCE_SCHEMA_VERSION: u32 = 0;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModLoader {
    #[serde(rename = "type")]
    pub loader_type: ModLoaderType,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModLoaderType {
    NeoForge,
    Forge,
    Fabric,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileManifestReference {
    pub url: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstanceManifest {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub pack_version: String,
    pub minecraft_version: String,
    pub loader: Option<ModLoader>,
    pub server_address: String,
    pub minimum_ram_mb: u64,
    pub recommended_ram_mb: u64,
    pub file_manifest: FileManifestReference,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackFileManifest {
    pub schema_version: u32,
    pub instance_id: String,
    pub pack_version: String,
    pub files: Vec<PackFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackFile {
    pub path: String,
    pub url: String,
    pub sha256: String,
    pub size: u64,
    pub update_policy: UpdatePolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdatePolicy {
    Replace,
    InstallIfMissing,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstanceEntry {
    pub id: String,
    pub state: InstanceState,
    pub instance_manifest: Option<InstanceManifest>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
#[serde(rename_all = "snake_case")]
pub enum InstanceState {
    Unknown,
    NotDownloaded,
    Downloading,
    RequiresUpdate,
    Updating,
    Ready,
    Launched,
    Broken,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceStateChanged {
    pub id: String,
    pub state: InstanceState,
    pub state_code: u8,
}

#[derive(Debug)]
pub enum RemoteInstanceError {
    Request(reqwest::Error),
    HttpStatus(StatusCode),
    InvalidManifest(reqwest::Error),
    Validation(String),
}

impl fmt::Display for RemoteInstanceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Request(error) => write!(f, "failed to retrieve remote instance: {error}"),
            Self::HttpStatus(status) => {
                write!(f, "remote instance request failed with status {status}")
            }
            Self::InvalidManifest(error) => {
                write!(f, "failed to parse remote instance manifest: {error}")
            }
            Self::Validation(error) => write!(f, "invalid remote instance manifest: {error}"),
        }
    }
}

impl std::error::Error for RemoteInstanceError {}

static INSTANCES: OnceLock<RwLock<Vec<InstanceEntry>>> = OnceLock::new();
static INSTANCE_STATE_OVERRIDES: OnceLock<RwLock<HashMap<String, InstanceState>>> = OnceLock::new();
static INSTANCE_EVENT_APP: OnceLock<AppHandle> = OnceLock::new();

pub fn register_instance_event_app(app: AppHandle) {
    if INSTANCE_EVENT_APP.set(app).is_err() {
        debug!("Instance state event app is already registered");
    }
}

fn read_local_instances() -> io::Result<Vec<InstanceEntry>> {
    let instance_dir = crate::config::get_config().resolved_install_dir()?;

    let mut scanned_instances: Vec<InstanceEntry> = Vec::new();

    for entry in fs::read_dir(instance_dir)? {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                warn!(%error, "Skipped unreadable instance directory entry");
                continue;
            }
        };

        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let instance_path = path.join(INSTANCE_MANIFEST_FILE);

        let instance_entry = match read_local_instance(&path, &instance_path) {
            Ok(instance_entry) => instance_entry,
            Err(error) => {
                warn!(path = %instance_path.display(), %error, "Skipped instance with invalid manifest");
                continue;
            }
        };

        if scanned_instances
            .iter()
            .any(|entry| entry.id == instance_entry.id)
        {
            warn!(id = %instance_entry.id, "Rejected duplicate instance entry");
            continue;
        }

        debug!(instance = %instance_entry.id, "Found instance");
        scanned_instances.push(instance_entry);
    }

    Ok(scanned_instances)
}

fn replace_instances(instances: Vec<InstanceEntry>) -> io::Result<()> {
    let lock = INSTANCES.get_or_init(|| RwLock::new(Vec::new()));

    let mut cached_instances = lock
        .write()
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("rwlock poisoned: {e}")))?;

    let changed_instance_ids = instances
        .iter()
        .filter(|instance| {
            cached_instances
                .iter()
                .find(|cached| cached.id == instance.id)
                .map_or(true, |cached| cached.state != instance.state)
        })
        .map(|instance| instance.id.clone())
        .collect::<Vec<_>>();

    let instance_count = instances.len();
    *cached_instances = instances;
    drop(cached_instances);

    debug!(instance_count, "Updated local instances");
    for instance_id in changed_instance_ids {
        emit_instance_state_changed(&instance_id, get_instance_state(&instance_id));
    }

    Ok(())
}

fn read_local_instance(root: &Path, manifest_path: &Path) -> io::Result<InstanceEntry> {
    match fs::metadata(manifest_path) {
        Ok(metadata) if metadata.is_file() => {}
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "instance manifest path is not a file",
            ));
        }
        Err(error) => return Err(error),
    }

    let instance_manifest = read_instance_manifest(manifest_path)?;
    validate_instance_manifest(root, &instance_manifest)?;

    Ok(InstanceEntry {
        id: instance_manifest.id.clone(),
        state: InstanceState::NotDownloaded,
        instance_manifest: Some(instance_manifest),
    })
}

fn validate_instance_manifest(root: &Path, instance: &InstanceManifest) -> io::Result<()> {
    validate_instance_fields(instance)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;

    let folder_name = root
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "instance folder name is not valid UTF-8",
            )
        })?;

    if folder_name != instance.id {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "instance id '{}' does not match folder name '{}'",
                instance.id, folder_name
            ),
        ));
    }

    Ok(())
}

fn validate_instance_fields(instance: &InstanceManifest) -> Result<(), String> {
    if instance.schema_version != SUPPORTED_INSTANCE_SCHEMA_VERSION {
        return Err(format!(
            "unsupported instance schema version: {}",
            instance.schema_version,
        ));
    }

    if instance.id.trim().is_empty() {
        return Err("instance id cannot be empty".to_string());
    }

    if instance.minimum_ram_mb > instance.recommended_ram_mb {
        return Err("minimum RAM cannot be greater than recommended RAM".to_string());
    }

    Ok(())
}

fn read_instance_manifest(path: &Path) -> io::Result<InstanceManifest> {
    let data = fs::read(path)?;
    serde_json::from_slice::<InstanceManifest>(&data).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("failed to parse instance manifest: {error}"),
        )
    })
}

fn write_instance_manifest(path: &Path, instance: &InstanceManifest) -> io::Result<()> {
    let content = serde_json::to_string_pretty(instance).map_err(|error| {
        io::Error::new(
            io::ErrorKind::Other,
            format!("failed to serialize instance manifest: {error}"),
        )
    })?;

    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, content)?;
    fs::rename(&tmp, path)?;
    debug!(path = %path.display(), "Written instance manifest");

    Ok(())
}

pub async fn fetch_pack_file_manifest(instance: &InstanceManifest) -> io::Result<PackFileManifest> {
    let client = Client::builder()
        .timeout(REMOTE_INSTANCE_REQUEST_TIMEOUT)
        .build()
        .map_err(|error| {
            error!(%error, "Error creating pack file manifest client");
            io::Error::new(io::ErrorKind::Other, error)
        })?;
    let url = &instance.file_manifest.url;

    let response = client.get(url).send().await.map_err(|error| {
        error!(%url, %error, "Error retrieving pack file manifest");
        io::Error::new(io::ErrorKind::Other, error)
    })?;

    let status = response.status();
    if !status.is_success() {
        error!(%url, %status, "Pack file manifest request failed");
        return Err(io::Error::new(
            io::ErrorKind::Other,
            format!("pack file manifest request failed with status {status}"),
        ));
    }

    let data = response.bytes().await.map_err(|error| {
        error!(%url, %error, "Error reading pack file manifest response");
        io::Error::new(io::ErrorKind::Other, error)
    })?;

    if !crate::utils::sha256_matches(data.as_ref(), &instance.file_manifest.sha256) {
        error!(
            %url,
            expected_sha256 = %instance.file_manifest.sha256,
            "Pack file manifest checksum mismatch"
        );
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "pack file manifest checksum mismatch",
        ));
    }

    let pack_file_manifest =
        serde_json::from_slice::<PackFileManifest>(data.as_ref()).map_err(|error| {
            error!(%url, %error, "Error parsing pack file manifest");
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("failed to parse pack file manifest: {error}"),
            )
        })?;

    validate_pack_file_manifest(instance, &pack_file_manifest).map_err(|error| {
        error!(%url, instance_id = %instance.id, %error, "Pack file manifest failed validation");
        io::Error::new(io::ErrorKind::InvalidData, error)
    })?;

    Ok(pack_file_manifest)
}

fn validate_pack_file_manifest(
    instance: &InstanceManifest,
    pack_file_manifest: &PackFileManifest,
) -> Result<(), String> {
    if pack_file_manifest.schema_version != SUPPORTED_INSTANCE_SCHEMA_VERSION {
        return Err(format!(
            "unsupported pack file manifest schema version: {}",
            pack_file_manifest.schema_version,
        ));
    }

    if pack_file_manifest.instance_id != instance.id {
        return Err(format!(
            "pack file manifest instance id '{}' does not match instance id '{}'",
            pack_file_manifest.instance_id, instance.id,
        ));
    }

    if pack_file_manifest.pack_version != instance.pack_version {
        return Err(format!(
            "pack file manifest version '{}' does not match instance version '{}'",
            pack_file_manifest.pack_version, instance.pack_version,
        ));
    }

    Ok(())
}

pub fn get_instances() -> Vec<InstanceEntry> {
    INSTANCES
        .get_or_init(|| RwLock::new(Vec::new()))
        .read()
        .expect("rwlock poisoned")
        .clone()
}

async fn get_remote_instance(instance_id: &str) -> Result<InstanceManifest, RemoteInstanceError> {
    let client = Client::builder()
        .timeout(REMOTE_INSTANCE_REQUEST_TIMEOUT)
        .build()
        .map_err(|error| {
            error!(%error, "Error creating remote instance client");
            RemoteInstanceError::Request(error)
        })?;
    let url = format!("{}/instances/{}/instance.json", CDN_URL, instance_id);

    let response = client.get(&url).send().await.map_err(|error| {
        error!(%url, %error, "Error retrieving remote instance");
        RemoteInstanceError::Request(error)
    })?;

    let status = response.status();
    if !status.is_success() {
        error!(%url, %status, "Remote instance request failed");
        return Err(RemoteInstanceError::HttpStatus(status));
    }

    let instance = response.json::<InstanceManifest>().await.map_err(|error| {
        error!(%url, %error, "Error parsing remote instance manifest");
        RemoteInstanceError::InvalidManifest(error)
    })?;

    validate_remote_instance(instance_id, &instance).map_err(|error| {
        error!(%url, instance_id = %instance.id, %error, "Remote instance manifest failed validation");
        RemoteInstanceError::Validation(error)
    })?;

    Ok(instance)
}

pub async fn init_instances() -> io::Result<()> {
    let mut instances = read_local_instances()?;

    for &instance_id in REMOTE_INSTANCE_IDS {
        let remote_instance = get_remote_instance(instance_id).await;
        let local_instance = instances.iter_mut().find(|entry| entry.id == instance_id);

        match (remote_instance, local_instance) {
            (Err(error), None) => {
                error!(id = %instance_id, %error, "Remote and local instances are both absent");
                instances.push(empty_instance_entry(instance_id, InstanceState::Unknown));
            }
            (Ok(_), None) => {
                debug!(id = %instance_id, "Instance not installed");
                instances.push(empty_instance_entry(
                    instance_id,
                    InstanceState::NotDownloaded,
                ));
            }
            (Err(error), Some(local_instance)) => {
                warn!(id = %instance_id, %error, "Remote instance is unavailable, but exists locally");
                local_instance.state = InstanceState::Unknown;
            }
            (Ok(remote_instance), Some(local_instance)) => {
                local_instance.state = determine_instance_state(
                    instance_id,
                    &remote_instance,
                    local_instance.instance_manifest.as_ref(),
                );
                debug!(id = %instance_id, state = %local_instance.state.clone() as u8, "Instance initialized");
            }
        }
    }

    replace_instances(instances)
}

fn validate_remote_instance(instance_id: &str, instance: &InstanceManifest) -> Result<(), String> {
    validate_instance_fields(instance)?;

    if instance.id != instance_id {
        return Err(format!(
            "remote instance id '{}' does not match requested id '{}'",
            instance.id, instance_id
        ));
    }

    Ok(())
}

fn empty_instance_entry(id: &str, state: InstanceState) -> InstanceEntry {
    InstanceEntry {
        id: id.to_string(),
        state,
        instance_manifest: None,
    }
}

fn determine_instance_state(
    instance_id: &str,
    remote_instance: &InstanceManifest,
    local_instance: Option<&InstanceManifest>,
) -> InstanceState {
    let Some(local_instance) = local_instance else {
        return InstanceState::NotDownloaded;
    };

    let version_difference =
        crate::utils::compare_semver(&remote_instance.pack_version, &local_instance.pack_version);

    match version_difference {
        Some(Ordering::Greater) => {
            info!(id = %instance_id, version = %remote_instance.pack_version, "Update available for instance");
            InstanceState::RequiresUpdate
        }
        Some(Ordering::Equal) => InstanceState::Ready,
        Some(Ordering::Less) => {
            warn!(
                id = %instance_id,
                local_version = %local_instance.pack_version,
                remote_version = %remote_instance.pack_version,
                "Local instance version is newer than remote"
            );
            InstanceState::Unknown
        }
        None => {
            error!(
                id = %instance_id,
                local_version = %local_instance.pack_version,
                remote_version = %remote_instance.pack_version,
                "Failed to compare instance versions"
            );
            InstanceState::Unknown
        }
    }
}

pub fn get_instance_state(id: &str) -> InstanceState {
    if let Some(state) = get_instance_state_override(id) {
        return state;
    }

    let instances = get_instances();

    match instances.iter().find(|&x| x.id == id) {
        Some(i) => i.state.clone(),
        None => InstanceState::Unknown,
    }
}

fn get_instance_state_override(id: &str) -> Option<InstanceState> {
    INSTANCE_STATE_OVERRIDES
        .get_or_init(|| RwLock::new(HashMap::new()))
        .read()
        .ok()
        .and_then(|states| states.get(id).cloned())
}

pub fn set_instance_state_override(id: &str, state: InstanceState) -> io::Result<()> {
    let previous_state = get_instance_state(id);
    let states = INSTANCE_STATE_OVERRIDES.get_or_init(|| RwLock::new(HashMap::new()));
    let mut states = states
        .write()
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("rwlock poisoned: {e}")))?;

    states.insert(id.to_string(), state);
    drop(states);

    if previous_state != state {
        emit_instance_state_changed(id, state);
    }

    Ok(())
}

pub fn clear_instance_state_override(id: &str) -> io::Result<()> {
    let states = INSTANCE_STATE_OVERRIDES.get_or_init(|| RwLock::new(HashMap::new()));
    let mut states = states
        .write()
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("rwlock poisoned: {e}")))?;

    let removed_state = states.remove(id);
    drop(states);

    if removed_state.is_some() {
        emit_instance_state_changed(id, get_instance_state(id));
    }

    Ok(())
}

fn emit_instance_state_changed(id: &str, state: InstanceState) {
    let Some(app) = INSTANCE_EVENT_APP.get() else {
        return;
    };

    let payload = InstanceStateChanged {
        id: id.to_string(),
        state,
        state_code: state as u8,
    };

    if let Err(error) = app.emit(INSTANCE_STATE_CHANGED_EVENT, payload) {
        warn!(%error, id, "Failed to emit instance state change");
    }
}

fn remote_instance_error_to_io(error: RemoteInstanceError) -> io::Error {
    io::Error::new(io::ErrorKind::Other, error)
}

pub async fn fetch_remote_instance(instance_id: &str) -> io::Result<()> {
    let install_dir = crate::config::get_config().resolved_install_dir()?;

    let instance = get_remote_instance(instance_id)
        .await
        .map_err(remote_instance_error_to_io)?;
    let instance_dir = install_dir.join(&instance.id);
    let manifest_path = instance_dir.join(INSTANCE_MANIFEST_FILE);

    fs::create_dir_all(&instance_dir)?;
    write_instance_manifest(&manifest_path, &instance)?;
    validate_instance_manifest(&instance_dir, &instance)?;

    info!(id = %instance.id, version = %instance.pack_version, "Fetched remote instance manifest");

    let mut instances = read_local_instances()?;

    if let Some(local_instance) = instances.iter_mut().find(|entry| entry.id == instance.id) {
        local_instance.state = determine_instance_state(
            &instance.id,
            &instance,
            local_instance.instance_manifest.as_ref(),
        );
    }

    replace_instances(instances)
}
