use std::{
    collections::HashMap,
    error::Error,
    fmt, fs, io,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use serde::Deserialize;
use tracing::info;

use super::MinecraftInstallError;

const DEFAULT_OFFLINE_USERNAME: &str = "SayKOPlayer";
const INSTANCE_GAME_DIR_NAME: &str = "game";

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LaunchOptions {
    pub username: Option<String>,
    pub uuid: Option<String>,
    pub min_memory_mb: Option<u64>,
    pub max_memory_mb: Option<u64>,
    #[serde(default)]
    pub extra_jvm_args: Vec<String>,
    #[serde(default)]
    pub extra_game_args: Vec<String>,
}

#[derive(Debug)]
pub enum MinecraftLaunchError {
    Install(MinecraftInstallError),
    Io(io::Error),
    InvalidManifest(String),
    Validation(String),
    MissingFile(String),
}

impl fmt::Display for MinecraftLaunchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Install(error) => write!(f, "{error}"),
            Self::Io(error) => write!(f, "filesystem error during Minecraft launch: {error}"),
            Self::InvalidManifest(message) => {
                write!(f, "invalid Minecraft launch manifest: {message}")
            }
            Self::Validation(message) => write!(f, "invalid Minecraft launch request: {message}"),
            Self::MissingFile(path) => write!(f, "Minecraft launch file is missing: {path}"),
        }
    }
}

impl Error for MinecraftLaunchError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Install(error) => Some(error),
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<MinecraftInstallError> for MinecraftLaunchError {
    fn from(error: MinecraftInstallError) -> Self {
        Self::Install(error)
    }
}

impl From<io::Error> for MinecraftLaunchError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

#[derive(Debug)]
struct LaunchContext {
    java_path: PathBuf,
    working_dir: PathBuf,
    main_class: String,
    jvm_args: Vec<String>,
    game_args: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VersionDetails {
    id: String,
    arguments: Option<Arguments>,
    #[serde(rename = "minecraftArguments")]
    minecraft_arguments: Option<String>,
    main_class: String,
    libraries: Vec<Library>,
    asset_index: AssetIndexRef,
    java_version: Option<JavaVersion>,
    #[serde(rename = "type")]
    version_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Arguments {
    #[serde(default)]
    game: Vec<Argument>,
    #[serde(default)]
    jvm: Vec<Argument>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Argument {
    String(String),
    Ruled {
        rules: Vec<Rule>,
        value: ArgumentValue,
    },
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ArgumentValue {
    String(String),
    List(Vec<String>),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JavaVersion {
    major_version: u32,
}

#[derive(Debug, Deserialize)]
struct AssetIndexRef {
    id: String,
}

#[derive(Debug, Deserialize)]
struct Library {
    downloads: Option<LibraryDownloads>,
    rules: Option<Vec<Rule>>,
}

#[derive(Debug, Deserialize)]
struct LibraryDownloads {
    artifact: Option<DownloadInfo>,
}

#[derive(Debug, Deserialize)]
struct DownloadInfo {
    path: Option<String>,
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
    arch: Option<String>,
}

pub async fn launch_instance(
    manifest: &crate::instance::InstanceManifest,
    options: LaunchOptions,
) -> Result<u32, MinecraftLaunchError> {
    let context = build_launch_context(manifest, options)?;
    spawn_minecraft(context)
}

fn build_launch_context(
    manifest: &crate::instance::InstanceManifest,
    options: LaunchOptions,
) -> Result<LaunchContext, MinecraftLaunchError> {
    let version_details = read_version_details(&manifest.minecraft_version)?;
    let data_dir = crate::config::get_config().resolved_data_dir()?;
    let working_dir = instance_game_dir(&manifest.id)?;
    fs::create_dir_all(&working_dir)?;

    let java_version = version_details.java_version.as_ref().ok_or_else(|| {
        MinecraftLaunchError::InvalidManifest(format!(
            "Minecraft version {} is missing javaVersion",
            version_details.id
        ))
    })?;
    let runtime_dir = super::java::runtime_dir(java_version.major_version)?;
    let java_path = super::java::executable_path(&runtime_dir);
    require_file(&java_path)?;

    let natives_dir = natives_dir(&data_dir, &version_details.id);
    if !natives_dir.is_dir() {
        return Err(MinecraftLaunchError::MissingFile(
            natives_dir.display().to_string(),
        ));
    }

    let classpath = build_classpath(&data_dir, &version_details)?;
    let username = launch_username(options.username.as_deref())?;
    let uuid = options
        .uuid
        .as_deref()
        .map(normalize_uuid)
        .transpose()?
        .unwrap_or_else(|| offline_uuid(&username));
    let (min_memory_mb, max_memory_mb) = memory_settings(manifest, &options)?;

    let variables = launch_variables(LaunchVariablesInput {
        version_details: &version_details,
        username: &username,
        uuid: &uuid,
        working_dir: &working_dir,
        data_dir: &data_dir,
        natives_dir: &natives_dir,
        classpath: &classpath,
    });

    let mut jvm_args = Vec::new();
    jvm_args.push(format!("-Xms{min_memory_mb}M"));
    jvm_args.push(format!("-Xmx{max_memory_mb}M"));
    jvm_args.extend(options.extra_jvm_args);
    jvm_args.extend(resolve_jvm_arguments(&version_details, &variables)?);

    let mut game_args = resolve_game_arguments(&version_details, &variables)?;
    game_args.extend(options.extra_game_args);

    Ok(LaunchContext {
        java_path,
        working_dir,
        main_class: version_details.main_class,
        jvm_args,
        game_args,
    })
}

fn spawn_minecraft(context: LaunchContext) -> Result<u32, MinecraftLaunchError> {
    let log_path = context.working_dir.join("logs").join("launcher-game.log");
    if let Some(parent) = log_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let stdout = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)?;
    let stderr = stdout.try_clone()?;

    let mut command = Command::new(&context.java_path);
    command
        .args(&context.jvm_args)
        .arg(&context.main_class)
        .args(&context.game_args)
        .current_dir(&context.working_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));

    let child = command.spawn()?;
    let pid = child.id();
    info!(
        pid,
        java = %context.java_path.display(),
        working_dir = %context.working_dir.display(),
        log = %log_path.display(),
        "Minecraft process started"
    );
    Ok(pid)
}

fn read_version_details(version: &str) -> Result<VersionDetails, MinecraftLaunchError> {
    let path = version_metadata_path(version)?;
    require_file(&path)?;
    let bytes = fs::read(&path)?;
    serde_json::from_slice::<VersionDetails>(&bytes).map_err(|error| {
        MinecraftLaunchError::InvalidManifest(format!(
            "failed to parse {}: {error}",
            path.display()
        ))
    })
}

fn version_metadata_path(version: &str) -> Result<PathBuf, MinecraftLaunchError> {
    Ok(crate::config::get_config()
        .resolved_data_dir()?
        .join("versions")
        .join(version)
        .join(format!("{version}.json")))
}

fn instance_game_dir(instance_id: &str) -> Result<PathBuf, MinecraftLaunchError> {
    Ok(crate::config::get_config()
        .resolved_install_dir()?
        .join(instance_id)
        .join(INSTANCE_GAME_DIR_NAME))
}

fn require_file(path: &Path) -> Result<(), MinecraftLaunchError> {
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => Ok(()),
        Ok(_) => Err(MinecraftLaunchError::MissingFile(
            path.display().to_string(),
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Err(
            MinecraftLaunchError::MissingFile(path.display().to_string()),
        ),
        Err(error) => Err(MinecraftLaunchError::Io(error)),
    }
}

fn build_classpath(
    data_dir: &Path,
    version_details: &VersionDetails,
) -> Result<Vec<PathBuf>, MinecraftLaunchError> {
    let mut classpath = Vec::new();
    let libraries_dir = data_dir.join("libraries");
    let features = HashMap::new();

    for library in version_details
        .libraries
        .iter()
        .filter(|library| evaluate_rules(library.rules.as_deref(), &features))
    {
        let Some(downloads) = &library.downloads else {
            continue;
        };
        let Some(artifact) = &downloads.artifact else {
            continue;
        };
        let Some(artifact_path) = &artifact.path else {
            continue;
        };

        let path = libraries_dir.join(artifact_path);
        require_file(&path)?;
        classpath.push(path);
    }

    let client_jar = data_dir
        .join("versions")
        .join(&version_details.id)
        .join(format!("{}.jar", version_details.id));
    require_file(&client_jar)?;
    classpath.push(client_jar);

    Ok(classpath)
}

struct LaunchVariablesInput<'a> {
    version_details: &'a VersionDetails,
    username: &'a str,
    uuid: &'a str,
    working_dir: &'a Path,
    data_dir: &'a Path,
    natives_dir: &'a Path,
    classpath: &'a [PathBuf],
}

fn launch_variables(input: LaunchVariablesInput<'_>) -> HashMap<String, String> {
    HashMap::from([
        ("auth_player_name".to_string(), input.username.to_string()),
        ("auth_uuid".to_string(), input.uuid.to_string()),
        ("auth_access_token".to_string(), "0".to_string()),
        ("clientid".to_string(), String::new()),
        ("auth_xuid".to_string(), String::new()),
        ("user_type".to_string(), "legacy".to_string()),
        ("version_name".to_string(), input.version_details.id.clone()),
        (
            "version_type".to_string(),
            input
                .version_details
                .version_type
                .clone()
                .unwrap_or_else(|| "release".to_string()),
        ),
        (
            "game_directory".to_string(),
            input.working_dir.display().to_string(),
        ),
        (
            "assets_root".to_string(),
            input.data_dir.join("assets").display().to_string(),
        ),
        (
            "assets_index_name".to_string(),
            input.version_details.asset_index.id.clone(),
        ),
        (
            "natives_directory".to_string(),
            input.natives_dir.display().to_string(),
        ),
        (
            "launcher_name".to_string(),
            "saykocraft-launcher".to_string(),
        ),
        ("launcher_version".to_string(), crate::VERSION.to_string()),
        ("classpath".to_string(), join_classpath(input.classpath)),
        (
            "classpath_separator".to_string(),
            classpath_separator().to_string(),
        ),
    ])
}

fn resolve_jvm_arguments(
    version_details: &VersionDetails,
    variables: &HashMap<String, String>,
) -> Result<Vec<String>, MinecraftLaunchError> {
    let Some(arguments) = &version_details.arguments else {
        return Ok(vec![
            format!(
                "-Djava.library.path={}",
                variables
                    .get("natives_directory")
                    .cloned()
                    .unwrap_or_default()
            ),
            "-cp".to_string(),
            variables.get("classpath").cloned().unwrap_or_default(),
        ]);
    };

    resolve_arguments(&arguments.jvm, variables)
}

fn resolve_game_arguments(
    version_details: &VersionDetails,
    variables: &HashMap<String, String>,
) -> Result<Vec<String>, MinecraftLaunchError> {
    if let Some(arguments) = &version_details.arguments {
        return resolve_arguments(&arguments.game, variables);
    }

    let Some(arguments) = &version_details.minecraft_arguments else {
        return Err(MinecraftLaunchError::InvalidManifest(format!(
            "Minecraft version {} has neither modern nor legacy launch arguments",
            version_details.id
        )));
    };

    Ok(arguments
        .split_whitespace()
        .map(|argument| replace_variables(argument, variables))
        .collect())
}

fn resolve_arguments(
    arguments: &[Argument],
    variables: &HashMap<String, String>,
) -> Result<Vec<String>, MinecraftLaunchError> {
    let features = HashMap::new();
    let mut resolved = Vec::new();

    for argument in arguments {
        match argument {
            Argument::String(value) => resolved.push(replace_variables(value, variables)),
            Argument::Ruled { rules, value } => {
                if evaluate_rules(Some(rules), &features) {
                    push_argument_value(value, variables, &mut resolved);
                }
            }
        }
    }

    Ok(resolved)
}

fn push_argument_value(
    value: &ArgumentValue,
    variables: &HashMap<String, String>,
    arguments: &mut Vec<String>,
) {
    match value {
        ArgumentValue::String(value) => arguments.push(replace_variables(value, variables)),
        ArgumentValue::List(values) => {
            arguments.extend(
                values
                    .iter()
                    .map(|value| replace_variables(value, variables)),
            );
        }
    }
}

fn replace_variables(value: &str, variables: &HashMap<String, String>) -> String {
    let mut resolved = value.to_string();
    for (key, replacement) in variables {
        resolved = resolved.replace(&format!("${{{key}}}"), replacement);
    }
    resolved
}

fn evaluate_rules(rules: Option<&[Rule]>, features: &HashMap<String, bool>) -> bool {
    let Some(rules) = rules else {
        return true;
    };

    let mut allowed = false;
    for rule in rules {
        if rule_matches(rule, features) {
            allowed = matches!(rule.action, RuleAction::Allow);
        }
    }

    allowed
}

fn rule_matches(rule: &Rule, features: &HashMap<String, bool>) -> bool {
    rule.os.as_ref().map_or(true, os_rule_matches)
        && rule
            .features
            .as_ref()
            .map_or(true, |required| feature_rules_match(required, features))
}

fn feature_rules_match(required: &HashMap<String, bool>, features: &HashMap<String, bool>) -> bool {
    required
        .iter()
        .all(|(key, expected)| features.get(key).copied().unwrap_or(false) == *expected)
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

fn memory_settings(
    manifest: &crate::instance::InstanceManifest,
    options: &LaunchOptions,
) -> Result<(u64, u64), MinecraftLaunchError> {
    let min_memory_mb = options.min_memory_mb.unwrap_or(manifest.minimum_ram_mb);
    let max_memory_mb = options.max_memory_mb.unwrap_or(manifest.recommended_ram_mb);

    if min_memory_mb == 0 || max_memory_mb == 0 {
        return Err(MinecraftLaunchError::Validation(
            "memory values must be greater than zero".to_string(),
        ));
    }

    if min_memory_mb > max_memory_mb {
        return Err(MinecraftLaunchError::Validation(
            "minimum memory cannot be greater than maximum memory".to_string(),
        ));
    }

    Ok((min_memory_mb, max_memory_mb))
}

fn launch_username(username: Option<&str>) -> Result<String, MinecraftLaunchError> {
    let username = username
        .map(str::trim)
        .filter(|username| !username.is_empty())
        .unwrap_or(DEFAULT_OFFLINE_USERNAME);

    if username.len() > 16
        || !username
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return Err(MinecraftLaunchError::Validation(format!(
            "invalid offline Minecraft username: {username}"
        )));
    }

    Ok(username.to_string())
}

fn normalize_uuid(uuid: &str) -> Result<String, MinecraftLaunchError> {
    let normalized = uuid.replace('-', "");
    if normalized.len() != 32 || !normalized.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(MinecraftLaunchError::Validation(format!(
            "invalid Minecraft UUID: {uuid}"
        )));
    }

    Ok(normalized)
}

fn offline_uuid(username: &str) -> String {
    let hash = crate::utils::sha1_hex(format!("OfflinePlayer:{username}").as_bytes());
    hash[..32].to_string()
}

fn join_classpath(classpath: &[PathBuf]) -> String {
    classpath
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(classpath_separator())
}

fn classpath_separator() -> &'static str {
    if cfg!(windows) {
        ";"
    } else {
        ":"
    }
}

fn natives_dir(data_dir: &Path, version: &str) -> PathBuf {
    data_dir
        .join("versions")
        .join(version)
        .join("natives")
        .join(current_native_platform_dir_name())
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
