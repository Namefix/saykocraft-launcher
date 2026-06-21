use std::{
    cmp::Ordering,
    fmt, fs, io,
    path::Path,
    sync::{OnceLock, RwLock},
    time::Duration,
};

use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use tracing::{debug, error, info, warn};

const CDN_URL: &str = "http://localhost:3001";

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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstanceState {
    Unknown,
    NotInstalled,
    RequiresUpdate,
    Ready,
    Launched,
    Broken,
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

    let instance_count = instances.len();
    *cached_instances = instances;
    debug!(instance_count, "Updated local instances");

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
        state: InstanceState::NotInstalled,
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

fn read_packfile_manifest(path: &Path) -> io::Result<PackFileManifest> {
    let data = fs::read(path)?;
    serde_json::from_slice::<PackFileManifest>(&data).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("failed to parse pack file manifest: {error}"),
        )
    })
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
                    InstanceState::NotInstalled,
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
        return InstanceState::NotInstalled;
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
