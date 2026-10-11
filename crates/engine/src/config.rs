// New in ether: slim configuration (user file, project file, CLI overrides) in place of codex-config.
//! Layers, lowest to highest precedence:
//!
//! 1. `$ETHER_HOME/config.toml` (default `~/.ether/config.toml`);
//! 2. `<cwd>/.ether/config.toml`, if present;
//! 3. [`ConfigOverrides`] from the command line.
//!
//! Each layer only sets the keys it names. The result is a plain [`EngineConfig`] value that
//! is handed to the engine; nothing here is global.
//!
//! The project file comes with the repository, so it is not trusted: it may set only the model
//! and UI keys ([`ConfigToml::project_safe`]). Keys that change what runs unasked or where
//! requests (and API keys) go (providers, approval policy, sandbox, base instructions) are
//! ignored there, with a warning the engine shows at session start.

use crate::model_profile::ModelProfile;
use codex_model_provider_info::ModelProviderInfo;
use codex_model_provider_info::WireApi;
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
const DEFAULT_MODEL: &str = "glm-5.3";
const OPENAI_API_KEY_ENV: &str = "OPENAI_API_KEY";

/// Zro's OpenAI-compatible gateway, which serves GLM (the endpoint the `zro` launcher gives pi).
pub const ZRO_PROVIDER_ID: &str = "zro";
const ZRO_BASE_URL: &str = "https://zro.moonmath.ai/v1";
const ZRO_API_KEY_ENV: &str = "ZRO_API_KEY";

fn zro_provider() -> ModelProviderInfo {
    ModelProviderInfo {
        name: "Zro".to_string(),
        base_url: Some(ZRO_BASE_URL.to_string()),
        env_key: Some(ZRO_API_KEY_ENV.to_string()),
        env_key_instructions: Some(
            "Export the API key that `zro` (or pi) uses: `export ZRO_API_KEY=...`".to_string(),
        ),
        wire_api: WireApi::ChatCompletions,
        ..ModelProviderInfo::default()
    }
}

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
    #[serde(default)]
    pub ui: UiToml,
}

/// The `[ui]` table.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiToml {
    /// Offer the `render_ui` tool (generative UI). Default: on, unless the TUI draws HTML views.
    pub render_tool: Option<bool>,
}

impl ConfigToml {
    /// The keys a repository's own config file may set; returns the names of the others,
    /// which are dropped.
    fn project_safe(mut self) -> (ConfigToml, Vec<&'static str>) {
        let mut ignored = Vec::new();
        if self.model_provider.take().is_some() {
            ignored.push("model_provider");
        }
        if !std::mem::take(&mut self.model_providers).is_empty() {
            ignored.push("model_providers");
        }
        if self.approval_policy.take().is_some() {
            ignored.push("approval_policy");
        }
        if self.sandbox_mode.take().is_some() {
            ignored.push("sandbox_mode");
        }
        if self.instructions.take().is_some() {
            ignored.push("instructions");
        }
        (self, ignored)
    }

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
        if other.ui.render_tool.is_some() {
            self.ui.render_tool = other.ui.render_tool;
        }
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
    /// `[ui] render_tool`: whether the `render_ui` tool is offered. Unset, it is offered unless
    /// the front end draws views itself (`display_instructions`).
    pub render_ui_tool: Option<bool>,
    /// What the front end can display, told to the model in the `<terminal_display>` context
    /// section. Set by the front end before the engine starts; fixed for the session.
    pub display_instructions: Option<String>,
    /// Offer `request_user_input`: only for a front end that answers
    /// `EventMsg::RequestUserInput` with `Op::UserInputAnswer` (none does yet).
    pub user_input_tool: bool,
    /// Warnings shown to the user when the session starts (for example ignored config keys).
    pub startup_warnings: Vec<String>,
}

impl EngineConfig {
    /// Whether the `render_ui` tool is offered.
    pub fn offers_render_ui(&self) -> bool {
        self.render_ui_tool
            .unwrap_or(self.display_instructions.is_none())
    }
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
    let project_path = cwd.join(PROJECT_DIR).join(CONFIG_FILE);
    let (project, ignored) = read_layer(&project_path)?.project_safe();
    let merged = user.merge(project);
    let mut config = resolve(merged, overrides, ether_home, cwd)?;
    if !ignored.is_empty() {
        config.startup_warnings.push(format!(
            "ignored {} in {}: a project's config may set only model and UI keys; set them in \
             {} or on the command line",
            ignored.join(", "),
            project_path.display(),
            config.ether_home.join(CONFIG_FILE).display()
        ));
    }
    Ok(config)
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
    let mut built_in = built_in_model_providers(/*openai_base_url*/ None);
    built_in.insert(ZRO_PROVIDER_ID.to_string(), zro_provider());
    let providers = merge_configured_model_providers(built_in, toml.model_providers)
        .map_err(ConfigError::Providers)?;
    let model_provider_id = overrides
        .model_provider
        .or(toml.model_provider)
        .unwrap_or_else(|| ZRO_PROVIDER_ID.to_string());
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
        render_ui_tool: toml.ui.render_tool,
        display_instructions: None,
        user_input_tool: false,
        startup_warnings: Vec::new(),
    })
}
