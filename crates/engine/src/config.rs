// New in ether: slim configuration (user file, project file, CLI overrides) in place of codex-config.
//! Layers, lowest to highest precedence:
//!
//! 1. `$ETHER_HOME/config.toml` (default `~/.ether/config.toml`);
//! 2. `<cwd>/.ether/config.toml`, if present;
//! 3. [`ConfigOverrides`] from the command line.
//!
//! Each layer only sets the keys it names. The result is a plain [`EngineConfig`] value that
//! is handed to the engine; nothing here is global.

use crate::model_profile::ModelProfile;
use codex_model_provider_info::ModelProviderInfo;
use codex_model_provider_info::OPENAI_PROVIDER_ID;
use codex_model_provider_info::built_in_model_providers;
use codex_model_provider_info::merge_configured_model_providers;
use codex_protocol::config_types::ReasoningSummary;
use codex_protocol::config_types::SandboxMode;
use codex_protocol::openai_models::ReasoningEffort;
use codex_protocol::protocol::AskForApproval;
use codex_utils_absolute_path::AbsolutePathBuf;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;

const CONFIG_FILE: &str = "config.toml";
const PROJECT_DIR: &str = ".ether";
const DEFAULT_MODEL: &str = "gpt-5.1-codex";
const OPENAI_API_KEY_ENV: &str = "OPENAI_API_KEY";

/// Base instructions sent as the system prompt. Copied from Codex's `models-manager/prompt.md`.
pub const BASE_INSTRUCTIONS: &str = include_str!("../prompt.md");

/// One configuration file, as written on disk. Every key is optional.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigToml {
    pub model: Option<String>,
    pub model_provider: Option<String>,
    pub model_context_window: Option<i64>,
    pub model_reasoning_effort: Option<ReasoningEffort>,
    pub model_reasoning_summary: Option<ReasoningSummary>,
    pub approval_policy: Option<AskForApproval>,
    pub sandbox_mode: Option<SandboxMode>,
    /// Replaces the built-in base instructions.
    pub instructions: Option<String>,
    #[serde(default)]
    pub model_providers: HashMap<String, ModelProviderInfo>,
}

impl ConfigToml {
    /// Applies `other` on top of `self`; keys set in `other` win.
    fn merge(mut self, other: ConfigToml) -> ConfigToml {
        macro_rules! take {
            ($($field:ident),*) => { $( if other.$field.is_some() { self.$field = other.$field; } )* };
        }
        take!(
            model,
            model_provider,
            model_context_window,
            model_reasoning_effort,
            model_reasoning_summary,
            approval_policy,
            sandbox_mode,
            instructions
        );
        self.model_providers.extend(other.model_providers);
        self
    }
}

/// Command-line settings that override both files.
#[derive(Debug, Clone, Default)]
pub struct ConfigOverrides {
    pub model: Option<String>,
    pub model_provider: Option<String>,
    pub approval_policy: Option<AskForApproval>,
    pub sandbox_mode: Option<SandboxMode>,
    pub cwd: Option<PathBuf>,
}

/// Resolved settings for one engine.
#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub ether_home: PathBuf,
    pub cwd: AbsolutePathBuf,
    pub model: ModelProfile,
    pub model_provider_id: String,
    pub model_provider: ModelProviderInfo,
    pub reasoning_effort: Option<ReasoningEffort>,
    pub reasoning_summary: ReasoningSummary,
    pub approval_policy: AskForApproval,
    pub sandbox_mode: SandboxMode,
    pub base_instructions: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("cannot determine the home directory; set ETHER_HOME")]
    NoHome,
    #[error("cannot read {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("invalid config {path}: {source}")]
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("cannot resolve working directory: {0}")]
    Cwd(std::io::Error),
    #[error("model provider `{0}` is not defined (add it under [model_providers])")]
    UnknownProvider(String),
    #[error("model providers: {0}")]
    Providers(String),
}

/// Returns `$ETHER_HOME`, or `~/.ether`.
pub fn find_ether_home() -> Result<PathBuf, ConfigError> {
    if let Some(home) = std::env::var_os("ETHER_HOME").filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(home));
    }
    dirs::home_dir()
        .map(|home| home.join(PROJECT_DIR))
        .ok_or(ConfigError::NoHome)
}

/// Loads and resolves the configuration layers.
pub fn load(overrides: ConfigOverrides) -> Result<EngineConfig, ConfigError> {
    let ether_home = find_ether_home()?;
    let cwd = match overrides.cwd.clone() {
        Some(cwd) => cwd,
        None => std::env::current_dir().map_err(ConfigError::Cwd)?,
    };
    let cwd = std::fs::canonicalize(&cwd)
        .and_then(AbsolutePathBuf::from_absolute_path)
        .map_err(ConfigError::Cwd)?;

    let user = read_layer(&ether_home.join(CONFIG_FILE))?;
    let project = read_layer(&cwd.join(PROJECT_DIR).join(CONFIG_FILE))?;
    let merged = user.merge(project);
    resolve(merged, overrides, ether_home, cwd)
}

fn read_layer(path: &Path) -> Result<ConfigToml, ConfigError> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ConfigToml::default());
        }
        Err(source) => {
            return Err(ConfigError::Read {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    toml::from_str(&text).map_err(|source| ConfigError::Parse {
        path: path.to_path_buf(),
        source,
    })
}

fn resolve(
    toml: ConfigToml,
    overrides: ConfigOverrides,
    ether_home: PathBuf,
    cwd: AbsolutePathBuf,
) -> Result<EngineConfig, ConfigError> {
    let providers = merge_configured_model_providers(
        built_in_model_providers(/*openai_base_url*/ None),
        toml.model_providers,
    )
    .map_err(ConfigError::Providers)?;
    let model_provider_id = overrides
        .model_provider
        .or(toml.model_provider)
        .unwrap_or_else(|| OPENAI_PROVIDER_ID.to_string());
    let mut model_provider = providers
        .get(&model_provider_id)
        .cloned()
        .ok_or_else(|| ConfigError::UnknownProvider(model_provider_id.clone()))?;
    // ether authenticates with API keys only; Codex's built-in OpenAI provider expects a login.
    if model_provider.requires_openai_auth && model_provider.env_key.is_none() {
        model_provider.env_key = Some(OPENAI_API_KEY_ENV.to_string());
    }

    let slug = overrides
        .model
        .or(toml.model)
        .unwrap_or_else(|| DEFAULT_MODEL.to_string());
    let mut model = ModelProfile::for_slug(&slug);
    if let Some(context_window) = toml.model_context_window {
        model.context_window = context_window;
    }

    Ok(EngineConfig {
        ether_home,
        cwd,
        model,
        model_provider_id,
        model_provider,
        reasoning_effort: toml.model_reasoning_effort,
        reasoning_summary: toml.model_reasoning_summary.unwrap_or_default(),
        approval_policy: overrides
            .approval_policy
            .or(toml.approval_policy)
            .unwrap_or_default(),
        sandbox_mode: overrides
            .sandbox_mode
            .or(toml.sandbox_mode)
            .unwrap_or(SandboxMode::WorkspaceWrite),
        base_instructions: toml
            .instructions
            .unwrap_or_else(|| BASE_INSTRUCTIONS.to_string()),
    })
}
