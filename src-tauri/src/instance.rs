use std::{fs, io, path::Path, sync::{OnceLock, RwLock}};

use serde::{Serialize, Deserialize};
use tracing::{debug, error, warn};

const INSTANCE_MANIFEST_FILE: &str = "instance.json";
const SUPPORTED_INSTANCE_SCHEMA_VERSION: u32 = 1;

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
    Fabric
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileManifestReference {
    pub url: String,
    pub sha256: String
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
    pub file_manifest: FileManifestReference
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

static INSTANCES: OnceLock<RwLock<Vec<InstanceManifest>>> = OnceLock::new();

pub fn scan_local_instances() -> io::Result<()>  {
    let instance_dir = crate::config::get_config().resolved_install_dir()?;

    let mut scanned_instances = Vec::new();

    for entry in fs::read_dir(instance_dir)? {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                warn!(%error, "Skipped unreadable instance directory entry");
                continue;
            }
        };

        let path = entry.path();
        if !path.is_dir() {continue};
        let instance_path = path.join(INSTANCE_MANIFEST_FILE);

        let instance = match read_local_instance(&path, &instance_path) {
            Ok(instance) => instance,
            Err(error) => {
                warn!(path = %instance_path.display(), %error, "Skipped instance with invalid manifest");
                continue;
            }
        };

        if scanned_instances.iter().any(|x: &InstanceManifest| x.id == instance.id) {
            warn!(id = %instance.id, "Rejected duplicate instance entry");
            continue;
        }

        debug!(instance = %instance.name, "Found instance");
        scanned_instances.push(instance);
    }

    replace_instances(scanned_instances)
}

fn replace_instances(instances: Vec<InstanceManifest>) -> io::Result<()> {
    let lock = INSTANCES.get_or_init(|| RwLock::new(Vec::new()));

    let mut cached_instances = lock
        .write()
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("rwlock poisoned: {e}")))?;

    let instance_count = instances.len();
    *cached_instances = instances;
    debug!(instance_count, "Updated local instances");

    Ok(())
}

fn read_local_instance(root: &Path, manifest_path: &Path) -> io::Result<InstanceManifest> {
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

    let instance = read_instance_manifest(manifest_path)?;
    validate_instance_manifest(root, &instance)?;

    Ok(instance)
}

fn validate_instance_manifest(root: &Path, instance: &InstanceManifest) -> io::Result<()> {
    if instance.schema_version != SUPPORTED_INSTANCE_SCHEMA_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unsupported instance schema version: {}", instance.schema_version),
        ));
    }

    if instance.id.trim().is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "instance id cannot be empty",
        ));
    }

    if instance.minimum_ram_mb > instance.recommended_ram_mb {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "minimum RAM cannot be greater than recommended RAM",
        ));
    }

    let folder_name = root.file_name().and_then(|name| name.to_str()).ok_or_else(|| {
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

pub fn get_instances() -> Vec<InstanceManifest> {
	INSTANCES.get_or_init(|| RwLock::new(Vec::new())).read().expect("rwlock poisoned").clone()
}
