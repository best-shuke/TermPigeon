use std::{io::ErrorKind, os::unix::fs::PermissionsExt, path::PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::fs;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ModelChoice {
    #[serde(rename = "gpt-5.6-sol")]
    Sol,
    #[serde(rename = "gpt-5.6-terra")]
    Terra,
    #[serde(rename = "gpt-5.6-luna")]
    Luna,
    #[serde(rename = "gpt-5.5")]
    Gpt55,
    #[serde(rename = "gpt-5.2")]
    Gpt52,
}

impl ModelChoice {
    pub const ALL: [Self; 5] = [Self::Sol, Self::Terra, Self::Luna, Self::Gpt55, Self::Gpt52];

    pub fn from_input(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "sol" | "5.6" | "5.6-sol" | "gpt-5.6" | "gpt-5.6-sol" => Some(Self::Sol),
            "terra" | "5.6-terra" | "gpt-5.6-terra" => Some(Self::Terra),
            "luna" | "5.6-luna" | "gpt-5.6-luna" => Some(Self::Luna),
            "5.5" | "gpt-5.5" => Some(Self::Gpt55),
            "5.2" | "gpt-5.2" => Some(Self::Gpt52),
            _ => None,
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::Sol => "gpt-5.6-sol",
            Self::Terra => "gpt-5.6-terra",
            Self::Luna => "gpt-5.6-luna",
            Self::Gpt55 => "gpt-5.5",
            Self::Gpt52 => "gpt-5.2",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Sol => "GPT-5.6 Sol",
            Self::Terra => "GPT-5.6 Terra",
            Self::Luna => "GPT-5.6 Luna",
            Self::Gpt55 => "GPT-5.5",
            Self::Gpt52 => "GPT-5.2",
        }
    }

    pub fn supports_effort(self, effort: ReasoningEffort) -> bool {
        match self {
            Self::Sol | Self::Terra => true,
            Self::Luna => effort != ReasoningEffort::Ultra,
            Self::Gpt55 | Self::Gpt52 => {
                !matches!(effort, ReasoningEffort::Max | ReasoningEffort::Ultra)
            }
        }
    }

    pub fn supports_fast(self) -> bool {
        self != Self::Gpt52
    }

    pub fn supported_efforts(self) -> Vec<ReasoningEffort> {
        ReasoningEffort::ALL
            .into_iter()
            .filter(|effort| self.supports_effort(*effort))
            .collect()
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningEffort {
    Low,
    Medium,
    High,
    Xhigh,
    Max,
    Ultra,
}

impl ReasoningEffort {
    pub const ALL: [Self; 6] = [
        Self::Low,
        Self::Medium,
        Self::High,
        Self::Xhigh,
        Self::Max,
        Self::Ultra,
    ];

    pub fn from_input(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "low" | "低" => Some(Self::Low),
            "medium" | "中" => Some(Self::Medium),
            "high" | "高" => Some(Self::High),
            "xhigh" | "extra-high" | "极高" => Some(Self::Xhigh),
            "max" | "最大" => Some(Self::Max),
            "ultra" | "终极" => Some(Self::Ultra),
            _ => None,
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Xhigh => "xhigh",
            Self::Max => "max",
            Self::Ultra => "ultra",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Low => "Low（低）",
            Self::Medium => "Medium（中）",
            Self::High => "High（高）",
            Self::Xhigh => "XHigh（极高）",
            Self::Max => "Max（最大）",
            Self::Ultra => "Ultra（自动多智能体）",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ResponseVerbosity {
    Low,
    Medium,
    High,
}

impl ResponseVerbosity {
    pub fn from_input(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "low" | "低" | "简洁" => Some(Self::Low),
            "medium" | "中" | "正常" => Some(Self::Medium),
            "high" | "高" | "详细" => Some(Self::High),
            _ => None,
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Low => "Low（简洁）",
            Self::Medium => "Medium（正常）",
            Self::High => "High（详细）",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CodexSettings {
    pub model: ModelChoice,
    pub reasoning_effort: ReasoningEffort,
    pub verbosity: ResponseVerbosity,
    pub fast_mode: bool,
}

impl Default for CodexSettings {
    fn default() -> Self {
        Self {
            model: ModelChoice::Sol,
            reasoning_effort: ReasoningEffort::Xhigh,
            verbosity: ResponseVerbosity::Medium,
            fast_mode: false,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct SessionState {
    pub thread_id: Option<String>,
    #[serde(default)]
    pub settings: CodexSettings,
}

#[derive(Clone, Debug)]
pub struct StateStore {
    path: PathBuf,
}

impl StateStore {
    pub async fn new(state_dir: PathBuf) -> Result<Self> {
        fs::create_dir_all(&state_dir)
            .await
            .with_context(|| format!("create state directory {}", state_dir.display()))?;
        fs::set_permissions(&state_dir, std::fs::Permissions::from_mode(0o700))
            .await
            .with_context(|| format!("protect state directory {}", state_dir.display()))?;
        Ok(Self {
            path: state_dir.join("session.json"),
        })
    }

    pub async fn load(&self) -> Result<SessionState> {
        let data = match fs::read(&self.path).await {
            Ok(data) => data,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(SessionState::default()),
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("read state file {}", self.path.display()));
            }
        };
        serde_json::from_slice(&data)
            .with_context(|| format!("parse state file {}", self.path.display()))
    }

    pub async fn save(&self, state: &SessionState) -> Result<()> {
        let temporary = self.path.with_extension("json.tmp");
        let data = serde_json::to_vec_pretty(state).context("serialize session state")?;
        fs::write(&temporary, data)
            .await
            .with_context(|| format!("write temporary state file {}", temporary.display()))?;
        fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o600))
            .await
            .with_context(|| format!("protect state file {}", temporary.display()))?;
        fs::rename(&temporary, &self.path)
            .await
            .with_context(|| format!("replace state file {}", self.path.display()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_state_files_receive_setting_defaults() {
        let state: SessionState = serde_json::from_str(r#"{"thread_id":null}"#).unwrap();
        assert_eq!(state.settings, CodexSettings::default());
    }

    #[test]
    fn model_capabilities_are_enforced() {
        assert!(ModelChoice::Sol.supports_effort(ReasoningEffort::Ultra));
        assert!(!ModelChoice::Luna.supports_effort(ReasoningEffort::Ultra));
        assert!(!ModelChoice::Gpt55.supports_effort(ReasoningEffort::Max));
        assert!(!ModelChoice::Gpt52.supports_fast());
    }
}
