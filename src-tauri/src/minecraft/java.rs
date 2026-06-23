use std::{
    fs, io,
    path::{Component, Path, PathBuf},
};

use flate2::read::GzDecoder;
use reqwest::Client;
use serde::Deserialize;
use tar::Archive as TarArchive;
use tracing::{debug, info};
use zip::ZipArchive;

use super::{InstallPhase, InstallProgress, InstallProgressSink, MinecraftInstallError};

const ADOPTIUM_API_BASE_URL: &str = "https://api.adoptium.net/v3";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct JavaVersion {
    component: Option<String>,
    major_version: u32,
}

#[derive(Debug, Deserialize)]
struct AdoptiumRelease {
    binary: AdoptiumBinary,
}

#[derive(Debug, Deserialize)]
struct AdoptiumBinary {
    package: AdoptiumPackage,
}

#[derive(Debug, Deserialize)]
struct AdoptiumPackage {
    name: String,
    link: String,
    checksum: String,
    size: u64,
}

pub(crate) async fn ensure_runtime<S>(
    client: &Client,
    minecraft_version: &str,
    java_version: Option<&JavaVersion>,
    instance_id: &str,
    progress: &S,
) -> Result<PathBuf, MinecraftInstallError>
where
    S: InstallProgressSink + ?Sized,
{
    let java_version = java_version.ok_or_else(|| {
        MinecraftInstallError::InvalidManifest(format!(
            "Minecraft version {minecraft_version} is missing javaVersion"
        ))
    })?;
    let component = java_version.component.as_deref().unwrap_or("java-runtime");
    let runtime_dir = runtime_dir(java_version.major_version)?;
    let java_path = executable_path(&runtime_dir);

    if runtime_is_ready(&runtime_dir) {
        emit_progress(
            progress,
            instance_id,
            format!(
                "Java {} runtime is ready ({component})",
                java_version.major_version
            ),
            1,
            1,
            Some(java_path.display().to_string()),
        );
        return Ok(java_path);
    }

    emit_progress(
        progress,
        instance_id,
        format!(
            "Fetching Java {} runtime metadata ({component})",
            java_version.major_version
        ),
        0,
        0,
        None,
    );

    let package = fetch_adoptium_package(client, java_version.major_version).await?;
    let archive_path = crate::config::get_config()
        .resolved_data_dir()?
        .join("runtimes")
        .join("archives")
        .join(&package.name);

    download_verified_archive(
        client,
        &package,
        &archive_path,
        DownloadProgress {
            instance_id,
            label: format!("Downloading Java {} runtime", java_version.major_version),
            current_file: package.name.clone(),
        },
        progress,
    )
    .await?;

    emit_progress(
        progress,
        instance_id,
        format!("Extracting Java {} runtime", java_version.major_version),
        0,
        0,
        Some(package.name.clone()),
    );
    extract_runtime_archive(&archive_path, &runtime_dir, &package.name)?;
    write_runtime_marker(&runtime_dir, java_version, &package)?;

    if !java_path.is_file() {
        return Err(MinecraftInstallError::Archive(format!(
            "Java executable was not found after extracting runtime: {}",
            java_path.display()
        )));
    }

    emit_progress(
        progress,
        instance_id,
        format!("Java {} runtime is ready", java_version.major_version),
        1,
        1,
        Some(java_path.display().to_string()),
    );
    info!(
        path = %java_path.display(),
        major = java_version.major_version,
        "Java runtime is ready"
    );

    Ok(java_path)
}

pub fn runtime_dir(major_version: u32) -> Result<PathBuf, MinecraftInstallError> {
    Ok(crate::config::get_config()
        .resolved_data_dir()?
        .join("runtimes")
        .join(format!("temurin-{major_version}-jre"))
        .join(crate::utils::current_platform_dir_name()))
}

pub fn executable_path(runtime_dir: &Path) -> PathBuf {
    if cfg!(windows) {
        runtime_dir.join("bin").join("java.exe")
    } else {
        runtime_dir.join("bin").join("java")
    }
}

fn runtime_is_ready(runtime_dir: &Path) -> bool {
    executable_path(runtime_dir).is_file() && runtime_marker_path(runtime_dir).is_file()
}

async fn fetch_adoptium_package(
    client: &Client,
    major_version: u32,
) -> Result<AdoptiumPackage, MinecraftInstallError> {
    let os = current_adoptium_os()?;
    let architecture = current_adoptium_architecture()?;
    let url = format!(
        "{ADOPTIUM_API_BASE_URL}/assets/latest/{major_version}/hotspot?architecture={architecture}&heap_size=normal&image_type=jre&os={os}&vendor=eclipse"
    );

    let response = client.get(&url).send().await?;
    let status = response.status();
    if !status.is_success() {
        return Err(MinecraftInstallError::HttpStatus { url, status });
    }

    let releases = response
        .json::<Vec<AdoptiumRelease>>()
        .await
        .map_err(|error| {
            MinecraftInstallError::InvalidManifest(format!(
                "invalid Adoptium Java runtime manifest: {error}"
            ))
        })?;

    let package = releases
        .into_iter()
        .next()
        .ok_or_else(|| {
            MinecraftInstallError::UnsupportedPlatform(format!(
                "no Temurin JRE {major_version} HotSpot runtime was found for {os}/{architecture}"
            ))
        })?
        .binary
        .package;

    validate_package(&package, major_version)?;
    Ok(package)
}

fn validate_package(
    package: &AdoptiumPackage,
    major_version: u32,
) -> Result<(), MinecraftInstallError> {
    if package.link.trim().is_empty()
        || package.name.trim().is_empty()
        || package.checksum.trim().is_empty()
        || package.size == 0
    {
        return Err(MinecraftInstallError::InvalidManifest(format!(
            "Adoptium Java runtime package is missing required fields for Java {major_version}"
        )));
    }

    Ok(())
}

async fn download_verified_archive(
    client: &Client,
    package: &AdoptiumPackage,
    destination: &Path,
    progress_info: DownloadProgress<'_>,
    progress: &(impl InstallProgressSink + ?Sized),
) -> Result<(), MinecraftInstallError> {
    if existing_file_matches(destination, package.size, &package.checksum)? {
        debug!(path = %destination.display(), "Skipping already valid Java runtime archive");
        emit_progress(
            progress,
            progress_info.instance_id,
            &progress_info.label,
            package.size,
            package.size,
            Some(progress_info.current_file),
        );
        return Ok(());
    }

    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }

    emit_progress(
        progress,
        progress_info.instance_id,
        &progress_info.label,
        0,
        package.size,
        Some(progress_info.current_file.clone()),
    );

    let response = client.get(&package.link).send().await?;
    let status = response.status();
    if !status.is_success() {
        return Err(MinecraftInstallError::HttpStatus {
            url: package.link.clone(),
            status,
        });
    }

    let bytes = response.bytes().await?;
    if bytes.len() as u64 != package.size {
        return Err(MinecraftInstallError::Validation(format!(
            "downloaded Java runtime archive size mismatch for {}: expected {}, got {}",
            destination.display(),
            package.size,
            bytes.len()
        )));
    }

    let actual_sha256 = crate::utils::sha256_hex(&bytes);
    if !actual_sha256.eq_ignore_ascii_case(&package.checksum) {
        return Err(MinecraftInstallError::ChecksumMismatch {
            path: destination.display().to_string(),
            expected: package.checksum.clone(),
            actual: actual_sha256,
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
        &progress_info.label,
        package.size,
        package.size,
        Some(progress_info.current_file),
    );
    info!(path = %destination.display(), "Downloaded Java runtime archive");
    Ok(())
}

fn existing_file_matches(
    path: &Path,
    expected_size: u64,
    expected_sha256: &str,
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
    Ok(crate::utils::sha256_matches(&bytes, expected_sha256))
}

fn extract_runtime_archive(
    archive_path: &Path,
    runtime_dir: &Path,
    archive_name: &str,
) -> Result<(), MinecraftInstallError> {
    let staging_dir = crate::utils::partial_path(runtime_dir);
    if staging_dir.exists() {
        fs::remove_dir_all(&staging_dir)?;
    }
    fs::create_dir_all(&staging_dir)?;

    match archive_format(archive_name)? {
        ArchiveFormat::Zip => extract_zip(archive_path, &staging_dir)?,
        ArchiveFormat::TarGz => extract_tar_gz(archive_path, &staging_dir)?,
    }

    let runtime_root = find_runtime_root(&staging_dir)?;
    if runtime_dir.exists() {
        fs::remove_dir_all(runtime_dir)?;
    }
    if let Some(parent) = runtime_dir.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::rename(&runtime_root, runtime_dir)?;

    if staging_dir.exists() {
        fs::remove_dir_all(&staging_dir)?;
    }

    Ok(())
}

enum ArchiveFormat {
    Zip,
    TarGz,
}

fn archive_format(archive_name: &str) -> Result<ArchiveFormat, MinecraftInstallError> {
    if archive_name.ends_with(".zip") {
        Ok(ArchiveFormat::Zip)
    } else if archive_name.ends_with(".tar.gz") || archive_name.ends_with(".tgz") {
        Ok(ArchiveFormat::TarGz)
    } else {
        Err(MinecraftInstallError::Archive(format!(
            "unsupported Java runtime archive format: {archive_name}"
        )))
    }
}

fn extract_zip(archive_path: &Path, destination_dir: &Path) -> Result<(), MinecraftInstallError> {
    fs::create_dir_all(destination_dir)?;

    let archive_file = fs::File::open(archive_path)?;
    let mut archive = ZipArchive::new(archive_file).map_err(|error| {
        MinecraftInstallError::Archive(format!(
            "could not open ZIP archive {}: {error}",
            archive_path.display()
        ))
    })?;

    for index in 0..archive.len() {
        let mut file = archive.by_index(index).map_err(|error| {
            MinecraftInstallError::Archive(format!(
                "could not read ZIP archive entry from {}: {error}",
                archive_path.display()
            ))
        })?;

        let Some(enclosed_name) = file.enclosed_name() else {
            debug!(
                archive = %archive_path.display(),
                entry = %file.name(),
                "Skipping unsafe ZIP archive entry"
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

fn extract_tar_gz(
    archive_path: &Path,
    destination_dir: &Path,
) -> Result<(), MinecraftInstallError> {
    fs::create_dir_all(destination_dir)?;

    let archive_file = fs::File::open(archive_path)?;
    let decoder = GzDecoder::new(archive_file);
    let mut archive = TarArchive::new(decoder);
    let entries = archive.entries().map_err(|error| {
        MinecraftInstallError::Archive(format!(
            "could not read tar.gz archive {}: {error}",
            archive_path.display()
        ))
    })?;

    for entry in entries {
        let mut entry = entry.map_err(|error| {
            MinecraftInstallError::Archive(format!(
                "could not read tar.gz archive entry from {}: {error}",
                archive_path.display()
            ))
        })?;
        let entry_path = entry
            .path()
            .map_err(|error| {
                MinecraftInstallError::Archive(format!(
                    "could not read tar.gz archive entry path from {}: {error}",
                    archive_path.display()
                ))
            })?
            .into_owned();

        if !is_safe_relative_path(&entry_path) {
            debug!(
                archive = %archive_path.display(),
                entry = %entry_path.display(),
                "Skipping unsafe tar.gz archive entry"
            );
            continue;
        }

        entry.unpack_in(destination_dir).map_err(|error| {
            MinecraftInstallError::Archive(format!(
                "could not extract tar.gz archive entry {} from {}: {error}",
                entry_path.display(),
                archive_path.display()
            ))
        })?;
    }

    Ok(())
}

fn find_runtime_root(staging_dir: &Path) -> Result<PathBuf, MinecraftInstallError> {
    if executable_path(staging_dir).is_file() {
        return Ok(staging_dir.to_path_buf());
    }

    let mut candidates = Vec::new();
    for entry in fs::read_dir(staging_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() && executable_path(&path).is_file() {
            candidates.push(path);
        }
    }

    match candidates.len() {
        1 => Ok(candidates.remove(0)),
        0 => Err(MinecraftInstallError::Archive(format!(
            "Java runtime archive did not contain a bin/java executable under {}",
            staging_dir.display()
        ))),
        _ => Err(MinecraftInstallError::Archive(format!(
            "Java runtime archive contained multiple candidate runtime roots under {}",
            staging_dir.display()
        ))),
    }
}

fn write_runtime_marker(
    runtime_dir: &Path,
    java_version: &JavaVersion,
    package: &AdoptiumPackage,
) -> Result<(), MinecraftInstallError> {
    let marker = format!(
        "provider=adoptium\nimage_type=jre\njvm=hotspot\nmajor={}\ncomponent={}\narchive={}\nsha256={}\n",
        java_version.major_version,
        java_version.component.as_deref().unwrap_or("java-runtime"),
        package.name,
        package.checksum
    );
    fs::write(runtime_marker_path(runtime_dir), marker)?;
    Ok(())
}

fn runtime_marker_path(runtime_dir: &Path) -> PathBuf {
    runtime_dir.join(".saykocraft-runtime")
}

fn current_adoptium_os() -> Result<&'static str, MinecraftInstallError> {
    match crate::utils::current_os_name() {
        "macos" => Ok("mac"),
        "windows" => Ok("windows"),
        "linux" => Ok("linux"),
        other => Err(MinecraftInstallError::UnsupportedPlatform(format!(
            "Temurin Java runtime downloads are not configured for OS: {other}"
        ))),
    }
}

fn current_adoptium_architecture() -> Result<&'static str, MinecraftInstallError> {
    match crate::utils::current_arch_name() {
        "x86_64" => Ok("x64"),
        "aarch64" => Ok("aarch64"),
        other => Err(MinecraftInstallError::UnsupportedPlatform(format!(
            "Temurin Java runtime downloads are not configured for architecture: {other}"
        ))),
    }
}

fn is_safe_relative_path(path: &Path) -> bool {
    !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_) | Component::CurDir))
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
        phase: InstallPhase::JavaRuntime,
        current_label: current_label.into(),
        downloaded_bytes,
        total_bytes,
        percentage,
        current_file,
    });
}
