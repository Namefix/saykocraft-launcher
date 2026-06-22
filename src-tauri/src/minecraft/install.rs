use std::{collections::HashMap, fs, io};

use reqwest::Client;
use serde::Deserialize;
use tracing::info;

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

    let _version_details = fetch_version_details(version).await?;

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

async fn fetch_version_details(version: &str) -> Result<VersionDetails, MinecraftInstallError> {
    let client = Client::new();

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
