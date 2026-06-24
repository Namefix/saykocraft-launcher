use std::{error::Error, fmt, io};

use reqwest::StatusCode;

#[derive(Debug)]
pub enum MinecraftInstallError {
    Io(io::Error),
    Request(reqwest::Error),
    HttpStatus {
        url: String,
        status: StatusCode,
    },
    InvalidManifest(String),
    Validation(String),
    MissingVersion(String),
    MissingDownload {
        version: String,
        artifact: String,
    },
    ChecksumMismatch {
        path: String,
        expected: String,
        actual: String,
    },
    Archive(String),
    Process(String),
    UnsupportedPlatform(String),
}

impl fmt::Display for MinecraftInstallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "filesystem error during Minecraft installation: {error}"),
            Self::Request(error) => {
                write!(f, "network error during Minecraft installation: {error}")
            }
            Self::HttpStatus { url, status } => {
                write!(
                    f,
                    "Minecraft download request failed with status {status}: {url}"
                )
            }
            Self::InvalidManifest(message) => write!(f, "invalid Minecraft manifest: {message}"),
            Self::Validation(message) => write!(f, "invalid Minecraft install request: {message}"),
            Self::MissingVersion(version) => {
                write!(
                    f,
                    "Minecraft version was not found in the version manifest: {version}"
                )
            }
            Self::MissingDownload { version, artifact } => {
                write!(
                    f,
                    "Minecraft version {version} is missing download artifact: {artifact}"
                )
            }
            Self::ChecksumMismatch {
                path,
                expected,
                actual,
            } => write!(
                f,
                "Minecraft download checksum mismatch for {path}: expected {expected}, got {actual}"
            ),
            Self::Archive(message) => {
                write!(f, "Minecraft archive extraction failed: {message}")
            }
            Self::Process(message) => {
                write!(f, "Minecraft installer process failed: {message}")
            }
            Self::UnsupportedPlatform(message) => {
                write!(f, "unsupported Minecraft install platform: {message}")
            }
        }
    }
}

impl Error for MinecraftInstallError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Request(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for MinecraftInstallError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<reqwest::Error> for MinecraftInstallError {
    fn from(error: reqwest::Error) -> Self {
        Self::Request(error)
    }
}
