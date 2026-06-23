use std::{collections::HashMap, fs, io, path::Path};

use reqwest::Client;
use serde::Deserialize;
use tracing::{debug, info};
use zip::ZipArchive;

use super::{
    InstallPhase, InstallProgress, InstallProgressSink, MinecraftInstallError, NoopProgressSink,
};

const VERSION_MANIFEST_URL: &str =
    "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";
const RESOURCE_BASE_URL: &str = "https://resources.download.minecraft.net";

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
    java_version: Option<super::java::JavaVersion>,
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
struct AssetIndex {
    objects: HashMap<String, AssetObject>,
}

#[derive(Debug, Deserialize)]
struct AssetObject {
    hash: String,
    size: u64,
}

#[derive(Debug, Deserialize)]
struct Library {
    name: String,
    downloads: Option<LibraryDownloads>,
    rules: Option<Vec<Rule>>,
    natives: Option<HashMap<String, String>>,
    extract: Option<ExtractRules>,
}

#[derive(Debug, Deserialize)]
struct LibraryDownloads {
    artifact: Option<DownloadInfo>,
    classifiers: Option<HashMap<String, DownloadInfo>>,
}

#[derive(Debug, Deserialize)]
struct ExtractRules {
    exclude: Option<Vec<String>>,
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
    super::java::ensure_runtime(
        &client,
        &version_details.id,
        version_details.java_version.as_ref(),
        instance_id,
        progress,
    )
    .await?;
    ensure_client_jar(&client, &version_details, instance_id, progress).await?;
    ensure_libraries(&client, &version_details, instance_id, progress).await?;
    ensure_assets(&client, &version_details, instance_id, progress).await?;
    ensure_native_libraries(&client, &version_details, instance_id, progress).await?;

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

    let bytes = response_version.bytes().await?;
    let version_details = serde_json::from_slice::<VersionDetails>(&bytes)
        .map_err(|error| MinecraftInstallError::InvalidManifest(error.to_string()))?;

    write_version_metadata(&version_details.id, &bytes)?;
    Ok(version_details)
}

fn write_version_metadata(version: &str, bytes: &[u8]) -> Result<(), MinecraftInstallError> {
    let version_dir = crate::config::get_config()
        .resolved_data_dir()?
        .join("versions")
        .join(version);
    fs::create_dir_all(&version_dir)?;
    fs::write(version_dir.join(format!("{version}.json")), bytes)?;
    Ok(())
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

async fn ensure_assets(
    client: &Client,
    version_details: &VersionDetails,
    instance_id: &str,
    progress: &(impl InstallProgressSink + ?Sized),
) -> Result<(), MinecraftInstallError> {
    let assets_dir = crate::config::get_config()
        .resolved_data_dir()?
        .join("assets");
    let index_path = assets_dir
        .join("indexes")
        .join(format!("{}.json", version_details.asset_index.id));

    download_verified_file(
        client,
        &version_details.asset_index.url,
        &index_path,
        version_details.asset_index.size,
        &version_details.asset_index.sha1,
        DownloadProgress {
            instance_id,
            phase: InstallPhase::MinecraftAssets,
            label: format!(
                "Downloading Minecraft asset index {}",
                version_details.asset_index.id
            ),
            current_file: index_path.display().to_string(),
        },
        progress,
    )
    .await?;

    let index_bytes = fs::read(&index_path)?;
    let asset_index: AssetIndex = serde_json::from_slice(&index_bytes)
        .map_err(|error| MinecraftInstallError::InvalidManifest(error.to_string()))?;

    for (asset_name, asset) in asset_index.objects {
        let prefix = sha1_prefix(&asset.hash, &asset_name)?;
        let destination = assets_dir.join("objects").join(prefix).join(&asset.hash);
        let url = format!("{RESOURCE_BASE_URL}/{prefix}/{}", asset.hash);

        download_verified_file(
            client,
            &url,
            &destination,
            asset.size,
            &asset.hash,
            DownloadProgress {
                instance_id,
                phase: InstallPhase::MinecraftAssets,
                label: format!("Downloading Minecraft asset {asset_name}"),
                current_file: asset_name,
            },
            progress,
        )
        .await?;
    }

    Ok(())
}

async fn ensure_native_libraries(
    client: &Client,
    version_details: &VersionDetails,
    instance_id: &str,
    progress: &(impl InstallProgressSink + ?Sized),
) -> Result<(), MinecraftInstallError> {
    let data_dir = crate::config::get_config().resolved_data_dir()?;
    let libraries_dir = data_dir.join("libraries");
    let natives_dir = data_dir
        .join("versions")
        .join(&version_details.id)
        .join("natives")
        .join(current_native_platform_dir_name());
    let mut extracted_count = 0usize;

    for library in version_details
        .libraries
        .iter()
        .filter(|library| library_is_allowed(library))
    {
        if let Some(native_classifier) = native_classifier_for_current_os(library) {
            let Some(downloads) = &library.downloads else {
                return Err(MinecraftInstallError::MissingDownload {
                    version: version_details.id.clone(),
                    artifact: format!("{} ({native_classifier})", library.name),
                });
            };
            let Some(classifiers) = &downloads.classifiers else {
                return Err(MinecraftInstallError::MissingDownload {
                    version: version_details.id.clone(),
                    artifact: format!("{} ({native_classifier})", library.name),
                });
            };
            let Some(native_download) = classifiers.get(&native_classifier) else {
                return Err(MinecraftInstallError::MissingDownload {
                    version: version_details.id.clone(),
                    artifact: format!("{} ({native_classifier})", library.name),
                });
            };

            ensure_native_archive(
                client,
                version_details,
                instance_id,
                progress,
                &libraries_dir,
                &natives_dir,
                library,
                &native_classifier,
                native_download,
            )
            .await?;
            extracted_count += 1;
        }

        if let Some(native_classifier) = native_artifact_classifier(library) {
            let Some(downloads) = &library.downloads else {
                return Err(MinecraftInstallError::MissingDownload {
                    version: version_details.id.clone(),
                    artifact: format!("{} ({native_classifier})", library.name),
                });
            };
            let Some(native_download) = &downloads.artifact else {
                return Err(MinecraftInstallError::MissingDownload {
                    version: version_details.id.clone(),
                    artifact: format!("{} ({native_classifier})", library.name),
                });
            };

            ensure_native_archive(
                client,
                version_details,
                instance_id,
                progress,
                &libraries_dir,
                &natives_dir,
                library,
                &native_classifier,
                native_download,
            )
            .await?;
            extracted_count += 1;
        }
    }

    if extracted_count == 0 {
        return Err(MinecraftInstallError::UnsupportedPlatform(format!(
            "no Minecraft native libraries were found for {} {}",
            current_minecraft_os_name(),
            crate::utils::current_arch_name()
        )));
    }

    info!(
        path = %natives_dir.display(),
        count = extracted_count,
        "Minecraft native libraries are ready"
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn ensure_native_archive(
    client: &Client,
    version_details: &VersionDetails,
    instance_id: &str,
    progress: &(impl InstallProgressSink + ?Sized),
    libraries_dir: &Path,
    natives_dir: &Path,
    library: &Library,
    native_classifier: &str,
    native_download: &DownloadInfo,
) -> Result<(), MinecraftInstallError> {
    let Some(native_path) = &native_download.path else {
        return Err(MinecraftInstallError::MissingDownload {
            version: version_details.id.clone(),
            artifact: format!("{} ({native_classifier})", library.name),
        });
    };

    let archive_path = libraries_dir.join(native_path);
    download_verified_file(
        client,
        &native_download.url,
        &archive_path,
        native_download.size,
        &native_download.sha1,
        DownloadProgress {
            instance_id,
            phase: InstallPhase::MinecraftNatives,
            label: format!("Downloading Minecraft native {}", library.name),
            current_file: native_path.clone(),
        },
        progress,
    )
    .await?;

    emit_progress(
        progress,
        instance_id,
        InstallPhase::MinecraftNatives,
        format!("Extracting Minecraft native {}", library.name),
        0,
        0,
        Some(native_classifier.to_string()),
    );
    extract_native_archive(&archive_path, natives_dir, library.extract.as_ref())?;
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

    let partial_destination = crate::utils::partial_path(destination);
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

fn library_is_allowed(library: &Library) -> bool {
    evaluate_rules(library.rules.as_deref())
}

fn native_classifier_for_current_os(library: &Library) -> Option<String> {
    let classifier = library.natives.as_ref()?.get(current_minecraft_os_name())?;
    Some(classifier.replace("${arch}", crate::utils::current_arch_bits()))
}

fn native_artifact_classifier(library: &Library) -> Option<String> {
    let classifier = library.name.split(':').nth(3)?;
    classifier
        .starts_with("natives-")
        .then(|| classifier.to_string())
}

fn extract_native_archive(
    archive_path: &Path,
    destination_dir: &Path,
    extract_rules: Option<&ExtractRules>,
) -> Result<(), MinecraftInstallError> {
    fs::create_dir_all(destination_dir)?;

    let archive_file = fs::File::open(archive_path)?;
    let mut archive = ZipArchive::new(archive_file).map_err(|error| {
        MinecraftInstallError::Archive(format!(
            "could not open native archive {}: {error}",
            archive_path.display()
        ))
    })?;

    for index in 0..archive.len() {
        let mut file = archive.by_index(index).map_err(|error| {
            MinecraftInstallError::Archive(format!(
                "could not read native archive entry from {}: {error}",
                archive_path.display()
            ))
        })?;
        let entry_name = file.name().to_string();

        if should_skip_native_entry(&entry_name, extract_rules) {
            continue;
        }

        let Some(enclosed_name) = file.enclosed_name() else {
            debug!(
                archive = %archive_path.display(),
                entry = %entry_name,
                "Skipping unsafe native archive entry"
            );
            continue;
        };
        let destination = destination_dir.join(enclosed_name);

        if file.is_dir() {
            fs::create_dir_all(&destination)?;
            continue;
        }

        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }

        let mut output = fs::File::create(&destination)?;
        io::copy(&mut file, &mut output)?;
    }

    Ok(())
}

fn should_skip_native_entry(entry_name: &str, extract_rules: Option<&ExtractRules>) -> bool {
    let entry_name = entry_name.replace('\\', "/");
    if entry_name.starts_with("META-INF/") {
        return true;
    }

    let Some(extract_rules) = extract_rules else {
        return false;
    };
    let Some(excludes) = &extract_rules.exclude else {
        return false;
    };

    excludes
        .iter()
        .any(|exclude| native_entry_matches_exclude(&entry_name, exclude))
}

fn native_entry_matches_exclude(entry_name: &str, exclude: &str) -> bool {
    let exclude = exclude.replace('\\', "/");
    if exclude.is_empty() {
        return false;
    }

    if exclude.ends_with('/') {
        entry_name.starts_with(&exclude)
    } else {
        entry_name == exclude
    }
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
            .map_or(true, |arch| arch == crate::utils::current_arch_name())
}

fn current_minecraft_os_name() -> &'static str {
    match crate::utils::current_os_name() {
        "macos" => "osx",
        "windows" => "windows",
        "linux" => "linux",
        other => other,
    }
}

fn current_native_platform_dir_name() -> String {
    format!(
        "{}-{}",
        current_minecraft_os_name(),
        crate::utils::current_arch_name()
    )
}

struct DownloadProgress<'a> {
    instance_id: &'a str,
    phase: InstallPhase,
    label: String,
    current_file: String,
}

fn sha1_prefix<'a>(hash: &'a str, asset_name: &str) -> Result<&'a str, MinecraftInstallError> {
    if hash.len() != 40 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(MinecraftInstallError::Validation(format!(
            "asset {asset_name} has an invalid SHA-1 hash: {hash}"
        )));
    }

    Ok(&hash[..2])
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
    let percentage = if total_bytes == 0 {
        None
    } else {
        let downloaded_bytes = downloaded_bytes.min(total_bytes);
        Some((downloaded_bytes as f64 / total_bytes as f64) * 100.0)
    };

    sink.emit(InstallProgress {
        instance_id: instance_id.to_string(),
        phase,
        current_label: current_label.into(),
        downloaded_bytes,
        total_bytes,
        percentage,
        current_file,
    });
}
