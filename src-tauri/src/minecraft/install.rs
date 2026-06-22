use std::{
    collections::HashMap,
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
};

use reqwest::Client;
use serde::Deserialize;
use tracing::{debug, info};

use super::MinecraftInstallError;

const VERSION_MANIFEST_URL: &str =
    "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";

#[derive(Debug, Deserialize)]
struct VersionsManifest {
    versions: Vec<VersionSummary>,
}

#[derive(Debug, Deserialize)]
struct VersionSummary {
    id: String,
    url: String,
    sha1: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VersionDetails {
    id: String,
    downloads: VersionDownloads,
    libraries: Vec<Library>,
    asset_index: AssetIndexRef,
}

#[derive(Debug, Deserialize)]
struct VersionDownloads {
    client: DownloadInfo,
}

#[derive(Debug, Deserialize)]
struct DownloadInfo {
    sha1: String,
    size: u64,
    url: String,
    path: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AssetIndexRef {
    id: String,
    sha1: String,
    size: u64,
    url: String,
}

#[derive(Debug, Deserialize)]
struct Library {
    name: String,
    downloads: Option<LibraryDownloads>,
    rules: Option<Vec<Rule>>,
    natives: Option<HashMap<String, String>>,
}

#[derive(Debug, Deserialize)]
struct LibraryDownloads {
    artifact: Option<DownloadInfo>,
    classifiers: Option<HashMap<String, DownloadInfo>>,
}

#[derive(Debug, Deserialize)]
struct Rule {
    action: RuleAction,
    os: Option<RuleOs>,
    features: Option<HashMap<String, bool>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum RuleAction {
    Allow,
    Disallow,
}

#[derive(Debug, Deserialize)]
struct RuleOs {
    name: Option<String>,
    version: Option<String>,
    arch: Option<String>,
}

pub async fn ensure_minecraft_installation(version: &str) -> Result<(), MinecraftInstallError> {
    info!(%version, "Ensuring minecraft installation for version");
    if version.trim().is_empty() {
        return Err(MinecraftInstallError::Validation(
            "Minecraft version cannot be empty".to_string(),
        ));
    }

    ensure_installation_app_dir().await?;

    let client = Client::new();
    let version_details = fetch_version_details(&client, version).await?;
    ensure_client_jar(&client, &version_details).await?;
    info!(version = %version_details.id, "Minecraft client jar is installed");

    Ok(())
}

pub async fn ensure_installation_app_dir() -> io::Result<()> {
    let base_data = crate::config::get_config().resolved_data_dir()?;

    let assets_dir = base_data.join("assets");
    fs::create_dir_all(assets_dir)?;

    let libraries_dir = base_data.join("libraries");
    fs::create_dir_all(libraries_dir)?;

    let versions_dir = base_data.join("versions");
    fs::create_dir_all(versions_dir)?;

    let runtimes_dir = base_data.join("runtimes");
    fs::create_dir_all(runtimes_dir)?;

    Ok(())
}

async fn fetch_version_details(
    client: &Client,
    version: &str,
) -> Result<VersionDetails, MinecraftInstallError> {
    let response = match client.get(VERSION_MANIFEST_URL).send().await {
        Ok(re) => re,
        Err(error) => return Err(MinecraftInstallError::Request(error)),
    };

    let status = response.status();
    if !status.is_success() {
        return Err(MinecraftInstallError::HttpStatus {
            url: VERSION_MANIFEST_URL.to_string(),
            status,
        });
    }

    let versions_manifest: VersionsManifest = response.json().await?;
    let version_summary = match versions_manifest.versions.iter().find(|&x| x.id == version) {
        Some(ve) => ve,
        None => {
            return Err(MinecraftInstallError::MissingVersion(version.to_string()));
        }
    };

    let response_version = match client.get(&version_summary.url).send().await {
        Ok(re) => re,
        Err(error) => return Err(MinecraftInstallError::Request(error)),
    };

    let status = response_version.status();
    if !status.is_success() {
        return Err(MinecraftInstallError::HttpStatus {
            url: version_summary.url.clone(),
            status,
        });
    }

    response_version
        .json::<VersionDetails>()
        .await
        .map_err(|error| MinecraftInstallError::InvalidManifest(error.to_string()))
}

async fn ensure_client_jar(
    client: &Client,
    version_details: &VersionDetails,
) -> Result<(), MinecraftInstallError> {
    let versions_dir = crate::config::get_config()
        .resolved_data_dir()?
        .join("versions")
        .join(&version_details.id);
    let jar_path = versions_dir.join(format!("{}.jar", version_details.id));
    let client_download = &version_details.downloads.client;

    download_verified_file(
        client,
        &client_download.url,
        &jar_path,
        client_download.size,
        &client_download.sha1,
    )
    .await
}

async fn download_verified_file(
    client: &Client,
    url: &str,
    destination: &Path,
    expected_size: u64,
    expected_sha1: &str,
) -> Result<(), MinecraftInstallError> {
    if is_existing_file_valid(destination, expected_size, expected_sha1)? {
        debug!(path = %destination.display(), "Skipping already valid Minecraft download");
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

    let bytes = response.bytes().await?;
    if bytes.len() as u64 != expected_size {
        return Err(MinecraftInstallError::Validation(format!(
            "downloaded file size mismatch for {}: expected {}, got {}",
            destination.display(),
            expected_size,
            bytes.len()
        )));
    }

    let actual_sha1 = crate::utils::sha1_hex(&bytes);
    if !actual_sha1.eq_ignore_ascii_case(expected_sha1) {
        return Err(MinecraftInstallError::ChecksumMismatch {
            path: destination.display().to_string(),
            expected: expected_sha1.to_string(),
            actual: actual_sha1,
        });
    }

    let partial_destination = partial_path(destination);
    fs::write(&partial_destination, &bytes)?;

    if destination.exists() {
        fs::remove_file(destination)?;
    }
    fs::rename(&partial_destination, destination)?;

    info!(path = %destination.display(), "Downloaded Minecraft file");
    Ok(())
}

fn is_existing_file_valid(
    path: &Path,
    expected_size: u64,
    expected_sha1: &str,
) -> Result<bool, MinecraftInstallError> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(MinecraftInstallError::Io(error)),
    };

    if !metadata.is_file() || metadata.len() != expected_size {
        return Ok(false);
    }

    let bytes = fs::read(path)?;
    Ok(crate::utils::sha1_matches(&bytes, expected_sha1))
}

fn partial_path(path: &Path) -> PathBuf {
    let mut partial = OsString::from(path.as_os_str());
    partial.push(".part");
    PathBuf::from(partial)
}
