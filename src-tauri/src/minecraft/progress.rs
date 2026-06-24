use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct InstallProgress {
    pub instance_id: String,
    pub phase: InstallPhase,
    pub current_label: String,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub percentage: Option<f64>,
    pub overall_percentage: Option<f64>,
    pub current_file: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallPhase {
    Preparing,
    MinecraftManifest,
    JavaRuntime,
    MinecraftClient,
    MinecraftLibraries,
    MinecraftAssets,
    MinecraftNatives,
    NeoForge,
    ModpackFiles,
    Finalizing,
    Done,
}

pub trait InstallProgressSink {
    fn emit(&self, progress: InstallProgress);
}

pub struct NoopProgressSink;

impl InstallProgressSink for NoopProgressSink {
    fn emit(&self, _progress: InstallProgress) {}
}

impl<F> InstallProgressSink for F
where
    F: Fn(InstallProgress) + Send + Sync,
{
    fn emit(&self, progress: InstallProgress) {
        self(progress);
    }
}
