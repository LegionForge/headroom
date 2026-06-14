use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_interval")]
    pub refresh_interval_secs: u64,
    #[serde(default)]
    pub system_profile: SystemProfile,
    #[serde(default)]
    pub ai: AiConfig,
}

// ── AI configuration ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
pub enum AiProvider {
    #[default]
    Claude,
    Ollama,
    #[serde(rename = "lmstudio")]
    LmStudio,
    #[serde(rename = "vllm")]
    Vllm,
    #[serde(rename = "openai")]
    OpenAi,
    /// Any other OpenAI-compatible endpoint (set base_url manually)
    #[serde(rename = "openai_compat")]
    OpenAiCompat,
}

impl AiProvider {
    pub fn default_base_url(&self) -> &'static str {
        match self {
            Self::Claude => "https://api.anthropic.com",
            Self::OpenAi => "https://api.openai.com",
            Self::Ollama => "http://localhost:11434",
            Self::LmStudio => "http://localhost:1234",
            Self::Vllm => "http://localhost:8000",
            Self::OpenAiCompat => "http://localhost:8080",
        }
    }

    pub fn default_model(&self) -> &'static str {
        match self {
            Self::Claude => "claude-sonnet-4-6",
            Self::OpenAi => "gpt-4o",
            Self::Ollama => "llama3.2",
            Self::LmStudio | Self::Vllm | Self::OpenAiCompat => "local-model",
        }
    }

    pub fn is_openai_compat(&self) -> bool {
        !matches!(self, Self::Claude)
    }
}

impl std::fmt::Display for AiProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Claude => write!(f, "Claude"),
            Self::Ollama => write!(f, "Ollama"),
            Self::LmStudio => write!(f, "LM Studio"),
            Self::Vllm => write!(f, "vLLM"),
            Self::OpenAi => write!(f, "OpenAI"),
            Self::OpenAiCompat => write!(f, "OpenAI-compatible"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiConfig {
    #[serde(default)]
    pub provider: AiProvider,
    /// Override the default base URL for this provider
    pub base_url: Option<String>,
    /// Override the default model for this provider
    pub model: Option<String>,
    pub api_key: Option<String>,
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            provider: AiProvider::Claude,
            base_url: None,
            model: None,
            api_key: std::env::var("ANTHROPIC_API_KEY").ok(),
        }
    }
}

impl AiConfig {
    pub fn resolved_base_url(&self) -> &str {
        self.base_url
            .as_deref()
            .unwrap_or(self.provider.default_base_url())
    }

    pub fn resolved_model(&self) -> &str {
        self.model
            .as_deref()
            .unwrap_or(self.provider.default_model())
    }
}

// ── System profile ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SystemProfile {
    pub cpu: String,
    pub ram_gb: u32,
    pub gpu: String,
    pub drives: Vec<DriveInfo>,
    pub use_cases: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriveInfo {
    pub label: String,
    pub drive_letter: String,
    /// "nvme", "sata", or "hdd"
    pub kind: String,
}

// ── Config loading ────────────────────────────────────────────────────────────

fn default_interval() -> u64 {
    2
}

impl Default for Config {
    fn default() -> Self {
        Self {
            refresh_interval_secs: 2,
            system_profile: SystemProfile::default(),
            ai: AiConfig::default(),
        }
    }
}

impl Config {
    pub fn load() -> Result<Self> {
        let path = config_path();
        match std::fs::read_to_string(&path) {
            Ok(content) => {
                let mut cfg: Self = toml::from_str(&content)?;
                // ANTHROPIC_API_KEY env var overrides file when using Claude
                if cfg.ai.provider == AiProvider::Claude {
                    if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
                        cfg.ai.api_key = Some(key);
                    }
                }
                Ok(cfg)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => {
                eprintln!(
                    "Warning: cannot read config at {}: {e} — using defaults",
                    path.display()
                );
                Ok(Self::default())
            }
        }
    }

    #[allow(dead_code)]
    pub fn save(&self) -> Result<()> {
        let path = config_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, toml::to_string_pretty(self)?)?;
        Ok(())
    }
}

fn config_path() -> PathBuf {
    #[cfg(target_os = "windows")]
    let base = PathBuf::from(std::env::var("APPDATA").unwrap_or_else(|_| ".".into()));

    #[cfg(not(target_os = "windows"))]
    let base = PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into())).join(".config");

    base.join("headroom").join("config.toml")
}
