use std::{
    collections::HashMap,
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
};

use reqwest::Client;
use serde::Deserialize;
use tracing::{debug, info};

use super::{
    InstallPhase, InstallProgress, InstallProgressSink, MinecraftInstallError, NoopProgressSink,
};

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
    let progress = NoopProgressSink;
    ensure_minecraft_installation_with_progress("minecraft", version, &progress).await
}

pub async fn ensure_minecraft_installation_with_progress<S>(
    instance_id: &str,
    version: &str,
    progress: &S,
) -> Result<(), MinecraftInstallError>
where
    S: InstallProgressSink + ?Sized,
{
    info!(%version, "Ensuring minecraft installation for version");
    if version.trim().is_empty() {
        return Err(MinecraftInstallError::Validation(
            "Minecraft version cannot be empty".to_string(),
        ));
    }

    emit_progress(
        progress,
        instance_id,
        InstallPhase::Preparing,
        "Preparing Minecraft installation",
        0,
        0,
        None,
    );
    ensure_installation_app_dir().await?;

    let client = Client::new();
    emit_progress(
        progress,
        instance_id,
        InstallPhase::MinecraftManifest,
        "Fetching Minecraft metadata",
        0,
        0,
        None,
    );
    let version_details = fetch_version_details(&client, version).await?;
    ensure_client_jar(&client, &version_details, instance_id, progress).await?;
    ensure_libraries(&client, &version_details, instance_id, progress).await?;

    emit_progress(
        progress,
        instance_id,
        InstallPhase::Done,
        "Minecraft installation ready",
        0,
        0,
        None,
    );
    info!(version = %version_details.id, "Minecraft installation is ready");

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
    instance_id: &str,
    progress: &(impl InstallProgressSink + ?Sized),
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
        DownloadProgress {
            instance_id,
            phase: InstallPhase::MinecraftClient,
            label: format!("Downloading Minecraft {}", version_details.id),
            current_file: jar_path.display().to_string(),
        },
        progress,
    )
    .await
}

async fn ensure_libraries(
    client: &Client,
    version_details: &VersionDetails,
    instance_id: &str,
    progress: &(impl InstallProgressSink + ?Sized),
) -> Result<(), MinecraftInstallError> {
    let libraries_dir = crate::config::get_config()
        .resolved_data_dir()?
        .join("libraries");

    for library in version_details
        .libraries
        .iter()
        .filter(|library| library_is_allowed(library))
    {
        let Some(downloads) = &library.downloads else {
            continue;
        };
        let Some(artifact) = &downloads.artifact else {
            continue;
        };
        let Some(artifact_path) = &artifact.path else {
            return Err(MinecraftInstallError::MissingDownload {
                version: version_details.id.clone(),
                artifact: library.name.clone(),
            });
        };

        let destination = libraries_dir.join(artifact_path);
        download_verified_file(
            client,
            &artifact.url,
            &destination,
            artifact.size,
            &artifact.sha1,
            DownloadProgress {
                instance_id,
                phase: InstallPhase::MinecraftLibraries,
                label: format!("Downloading Minecraft library {}", library.name),
                current_file: artifact_path.clone(),
            },
            progress,
        )
        .await?;
    }

    Ok(())
}

async fn download_verified_file(
    client: &Client,
    url: &str,
    destination: &Path,
    expected_size: u64,
    expected_sha1: &str,
    progress_info: DownloadProgress<'_>,
    progress: &(impl InstallProgressSink + ?Sized),
) -> Result<(), MinecraftInstallError> {
    if is_existing_file_valid(destination, expected_size, expected_sha1)? {
        debug!(path = %destination.display(), "Skipping already valid Minecraft download");
        emit_progress(
            progress,
            progress_info.instance_id,
            progress_info.phase,
            &progress_info.label,
            expected_size,
            expected_size,
            Some(progress_info.current_file.clone()),
        );
        return Ok(());
    }

    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }

    emit_progress(
        progress,
        progress_info.instance_id,
        progress_info.phase.clone(),
        &progress_info.label,
        0,
        expected_size,
        Some(progress_info.current_file.clone()),
    );

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

    emit_progress(
        progress,
        progress_info.instance_id,
        progress_info.phase,
        &progress_info.label,
        expected_size,
        expected_size,
        Some(progress_info.current_file),
    );
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

fn library_is_allowed(library: &Library) -> bool {
    evaluate_rules(library.rules.as_deref())
}

fn evaluate_rules(rules: Option<&[Rule]>) -> bool {
    let Some(rules) = rules else {
        return true;
    };

    let mut allowed = false;
    for rule in rules {
        if rule_matches(rule) {
            allowed = matches!(rule.action, RuleAction::Allow);
        }
    }

    allowed
}

fn rule_matches(rule: &Rule) -> bool {
    rule.os.as_ref().map_or(true, os_rule_matches)
        && rule
            .features
            .as_ref()
            .map_or(true, |features| features.is_empty())
}

fn os_rule_matches(os: &RuleOs) -> bool {
    os.name
        .as_ref()
        .map_or(true, |name| name == current_minecraft_os_name())
        && os
            .arch
            .as_ref()
            .map_or(true, |arch| arch == current_minecraft_arch())
}

fn current_minecraft_os_name() -> &'static str {
    match std::env::consts::OS {
        "macos" => "osx",
        "windows" => "windows",
        "linux" => "linux",
        other => other,
    }
}

fn current_minecraft_arch() -> &'static str {
    match std::env::consts::ARCH {
        "x86" | "i386" | "i586" | "i686" => "x86",
        "x86_64" | "amd64" => "x86_64",
        other => other,
    }
}

struct DownloadProgress<'a> {
    instance_id: &'a str,
    phase: InstallPhase,
    label: String,
    current_file: String,
}

fn emit_progress(
    sink: &(impl InstallProgressSink + ?Sized),
    instance_id: &str,
    phase: InstallPhase,
    current_label: impl Into<String>,
    downloaded_bytes: u64,
    total_bytes: u64,
    current_file: Option<String>,
) {
    sink.emit(InstallProgress {
        instance_id: instance_id.to_string(),
        phase,
        current_label: current_label.into(),
        downloaded_bytes,
        total_bytes,
        current_file,
    });
}
