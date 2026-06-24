use std::{
    fs, io,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use reqwest::Client;
use serde::Deserialize;
use tracing::{debug, info};

use crate::instance::InstanceManifest;

use super::{InstallPhase, InstallProgress, InstallProgressSink, MinecraftInstallError};

const NEOFORGE_MAVEN_BASE_URL: &str = "https://maven.neoforged.net/releases";
const NEOFORGE_VERSION_PREFIX: &str = "neoforge-";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LoaderVersionDetails {
    id: String,
    inherits_from: Option<String>,
}

pub(super) async fn ensure_loader<S>(
    client: &Client,
    instance: &InstanceManifest,
    loader_version: &str,
    java_path: &Path,
    progress: &S,
) -> Result<(), MinecraftInstallError>
where
    S: InstallProgressSink + ?Sized,
{
    if loader_version.trim().is_empty() {
        return Err(MinecraftInstallError::Validation(
            "NeoForge version cannot be empty".to_string(),
        ));
    }

    let expected_version_id = neoforge_version_id(loader_version);
    if neoforge_install_is_ready(&expected_version_id, &instance.minecraft_version)? {
        emit_progress(
            progress,
            &instance.id,
            format!("NeoForge {loader_version} is ready"),
            1,
            1,
            Some(expected_version_id),
        );
        return Ok(());
    }

    emit_progress(
        progress,
        &instance.id,
        format!("Preparing NeoForge {loader_version}"),
        0,
        0,
        None,
    );

    let installer_path =
        ensure_neoforge_installer(client, loader_version, &instance.id, progress).await?;
    run_neoforge_installer(
        java_path,
        &installer_path,
        loader_version,
        &instance.id,
        progress,
    )?;

    if !neoforge_install_is_ready(&expected_version_id, &instance.minecraft_version)? {
        return Err(MinecraftInstallError::Validation(format!(
            "NeoForge installer completed, but {expected_version_id} was not installed correctly"
        )));
    }

    emit_progress(
        progress,
        &instance.id,
        format!("NeoForge {loader_version} is ready"),
        1,
        1,
        Some(expected_version_id),
    );
    info!(
        instance = %instance.id,
        loader_version,
        "NeoForge installation is ready"
    );

    Ok(())
}

async fn ensure_neoforge_installer(
    client: &Client,
    loader_version: &str,
    instance_id: &str,
    progress: &(impl InstallProgressSink + ?Sized),
) -> Result<PathBuf, MinecraftInstallError> {
    let installer_url = neoforge_installer_url(loader_version);
    let checksum_url = format!("{installer_url}.sha1");
    let expected_sha1 = fetch_sha1_checksum(client, &checksum_url).await?;
    let installer_path = neoforge_installer_path(loader_version)?;

    download_verified_sha1_file(
        client,
        &installer_url,
        &installer_path,
        &expected_sha1,
        DownloadProgress {
            instance_id,
            label: format!("Downloading NeoForge {loader_version} installer"),
            current_file: installer_path.display().to_string(),
        },
        progress,
    )
    .await?;

    Ok(installer_path)
}

fn run_neoforge_installer(
    java_path: &Path,
    installer_path: &Path,
    loader_version: &str,
    instance_id: &str,
    progress: &(impl InstallProgressSink + ?Sized),
) -> Result<(), MinecraftInstallError> {
    let data_dir = crate::config::get_config().resolved_data_dir()?;
    ensure_launcher_profiles(&data_dir)?;

    let log_path = data_dir
        .join("logs")
        .join(format!("neoforge-{loader_version}-installer.log"));
    if let Some(parent) = log_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let stdout = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)?;
    let stderr = stdout.try_clone()?;

    emit_progress(
        progress,
        instance_id,
        format!("Installing NeoForge {loader_version}"),
        0,
        0,
        Some(log_path.display().to_string()),
    );

    let status = Command::new(java_path)
        .arg("-jar")
        .arg(installer_path)
        .arg("--install-client")
        .arg(&data_dir)
        .current_dir(&data_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .status()?;

    if !status.success() {
        return Err(MinecraftInstallError::Process(format!(
            "NeoForge installer exited with status {status}. See {}",
            log_path.display()
        )));
    }

    Ok(())
}

fn ensure_launcher_profiles(data_dir: &Path) -> Result<(), MinecraftInstallError> {
    fs::create_dir_all(data_dir)?;

    let profile_path = data_dir.join("launcher_profiles.json");
    match fs::metadata(&profile_path) {
        Ok(metadata) if metadata.is_file() => return Ok(()),
        Ok(_) => {
            return Err(MinecraftInstallError::Validation(format!(
                "launcher profile path is not a file: {}",
                profile_path.display()
            )));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(MinecraftInstallError::Io(error)),
    }

    let profile = serde_json::json!({
        "profiles": {},
        "selectedProfile": "",
        "clientToken": "saykocraft-launcher",
        "authenticationDatabase": {},
        "launcherVersion": {
            "name": "saykocraft-launcher",
            "format": 21,
            "profilesFormat": 2
        }
    });
    let content = serde_json::to_vec_pretty(&profile)
        .map_err(|error| MinecraftInstallError::InvalidManifest(error.to_string()))?;
    let partial_path = crate::utils::partial_path(&profile_path);

    fs::write(&partial_path, content)?;
    fs::rename(&partial_path, &profile_path)?;

    Ok(())
}

fn neoforge_install_is_ready(
    version_id: &str,
    minecraft_version: &str,
) -> Result<bool, MinecraftInstallError> {
    let path = version_metadata_path(version_id)?;
    if !path.is_file() {
        return Ok(false);
    }

    let bytes = fs::read(&path)?;
    let details: LoaderVersionDetails = serde_json::from_slice(&bytes)
        .map_err(|error| MinecraftInstallError::InvalidManifest(error.to_string()))?;

    Ok(details.id == version_id
        && details.inherits_from.as_deref() == Some(minecraft_version)
        && neoforge_patched_client_path(version_id)?.is_file())
}

fn neoforge_version_id(loader_version: &str) -> String {
    format!("{NEOFORGE_VERSION_PREFIX}{loader_version}")
}

fn neoforge_installer_url(loader_version: &str) -> String {
    format!(
        "{NEOFORGE_MAVEN_BASE_URL}/net/neoforged/neoforge/{loader_version}/neoforge-{loader_version}-installer.jar"
    )
}

fn neoforge_installer_path(loader_version: &str) -> Result<PathBuf, MinecraftInstallError> {
    Ok(crate::config::get_config()
        .resolved_data_dir()?
        .join("libraries")
        .join("net")
        .join("neoforged")
        .join("neoforge")
        .join(loader_version)
        .join(format!("neoforge-{loader_version}-installer.jar")))
}

fn neoforge_patched_client_path(version_id: &str) -> Result<PathBuf, MinecraftInstallError> {
    let loader_version = version_id
        .strip_prefix(NEOFORGE_VERSION_PREFIX)
        .unwrap_or(version_id);

    Ok(crate::config::get_config()
        .resolved_data_dir()?
        .join("libraries")
        .join("net")
        .join("neoforged")
        .join("neoforge")
        .join(loader_version)
        .join(format!("neoforge-{loader_version}-client.jar")))
}

fn version_metadata_path(version: &str) -> Result<PathBuf, MinecraftInstallError> {
    Ok(crate::config::get_config()
        .resolved_data_dir()?
        .join("versions")
        .join(version)
        .join(format!("{version}.json")))
}

async fn fetch_sha1_checksum(client: &Client, url: &str) -> Result<String, MinecraftInstallError> {
    let response = client.get(url).send().await?;
    let status = response.status();
    if !status.is_success() {
        return Err(MinecraftInstallError::HttpStatus {
            url: url.to_string(),
            status,
        });
    }

    let text = response.text().await?;
    let checksum = text
        .split_whitespace()
        .next()
        .ok_or_else(|| {
            MinecraftInstallError::InvalidManifest(format!("empty SHA-1 checksum response: {url}"))
        })?
        .to_string();

    if checksum.len() != 40 || !checksum.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(MinecraftInstallError::InvalidManifest(format!(
            "invalid SHA-1 checksum response from {url}: {checksum}"
        )));
    }

    Ok(checksum)
}

async fn download_verified_sha1_file(
    client: &Client,
    url: &str,
    destination: &Path,
    expected_sha1: &str,
    progress_info: DownloadProgress<'_>,
    progress: &(impl InstallProgressSink + ?Sized),
) -> Result<(), MinecraftInstallError> {
    if is_existing_file_sha1_valid(destination, expected_sha1)? {
        let size = fs::metadata(destination)?.len();
        debug!(path = %destination.display(), "Skipping already valid NeoForge download");
        emit_progress(
            progress,
            progress_info.instance_id,
            &progress_info.label,
            size,
            size,
            Some(progress_info.current_file.clone()),
        );
        return Ok(());
    }

    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }

    let response = client.get(url).send().await?;
    let status = response.status();
    if !status.is_success() {
        return Err(MinecraftInstallError::HttpStatus {
            url: url.to_string(),
            status,
        });
    }

    let total_bytes = response.content_length().unwrap_or(0);
    emit_progress(
        progress,
        progress_info.instance_id,
        &progress_info.label,
        0,
        total_bytes,
        Some(progress_info.current_file.clone()),
    );

    let bytes = response.bytes().await?;
    let actual_sha1 = crate::utils::sha1_hex(&bytes);
    if !actual_sha1.eq_ignore_ascii_case(expected_sha1) {
        return Err(MinecraftInstallError::ChecksumMismatch {
            path: destination.display().to_string(),
            expected: expected_sha1.to_string(),
            actual: actual_sha1,
        });
    }

    write_downloaded_bytes(destination, &bytes)?;

    emit_progress(
        progress,
        progress_info.instance_id,
        &progress_info.label,
        bytes.len() as u64,
        total_bytes,
        Some(progress_info.current_file),
    );
    debug!(path = %destination.display(), "Downloaded NeoForge file");
    Ok(())
}

fn write_downloaded_bytes(destination: &Path, bytes: &[u8]) -> Result<(), MinecraftInstallError> {
    let partial_destination = crate::utils::partial_path(destination);
    fs::write(&partial_destination, bytes)?;

    if destination.exists() {
        fs::remove_file(destination)?;
    }
    fs::rename(&partial_destination, destination)?;

    Ok(())
}

fn is_existing_file_sha1_valid(
    path: &Path,
    expected_sha1: &str,
) -> Result<bool, MinecraftInstallError> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(MinecraftInstallError::Io(error)),
    };

    if !metadata.is_file() {
        return Ok(false);
    }

    let bytes = fs::read(path)?;
    Ok(crate::utils::sha1_matches(&bytes, expected_sha1))
}

struct DownloadProgress<'a> {
    instance_id: &'a str,
    label: String,
    current_file: String,
}

fn emit_progress(
    sink: &(impl InstallProgressSink + ?Sized),
    instance_id: &str,
    current_label: impl Into<String>,
    downloaded_bytes: u64,
    total_bytes: u64,
    current_file: Option<String>,
) {
    let percentage = if total_bytes == 0 {
        None
    } else {
        let downloaded_bytes = downloaded_bytes.min(total_bytes);
        Some((downloaded_bytes as f64 / total_bytes as f64) * 100.0)
    };

    sink.emit(InstallProgress {
        instance_id: instance_id.to_string(),
        phase: InstallPhase::NeoForge,
        current_label: current_label.into(),
        downloaded_bytes,
        total_bytes,
        percentage,
        overall_percentage: None,
        current_file,
    });
}
