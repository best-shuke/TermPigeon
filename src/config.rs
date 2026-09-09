use std::{env, path::PathBuf, time::Duration};

use anyhow::{Context, Result, bail};

#[derive(Clone, Debug)]
pub struct Config {
    pub bot_token: String,
    pub owner_user_id: u64,
    pub codex_bin: PathBuf,
    pub codex_home: PathBuf,
    pub workdir: PathBuf,
    pub state_dir: PathBuf,
    pub codex_timeout: Duration,
    pub dangerously_bypass_approvals_and_sandbox: bool,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let bot_token = required("TELOXIDE_TOKEN")?;
        let owner_user_id = required("TERMPIGEON_OWNER_USER_ID")?
            .parse::<u64>()
            .context("TERMPIGEON_OWNER_USER_ID must be an unsigned integer")?;
        let codex_bin = PathBuf::from(required("TERMPIGEON_CODEX_BIN")?);
        let codex_home = PathBuf::from(required("TERMPIGEON_CODEX_HOME")?);
        let workdir = PathBuf::from(required("TERMPIGEON_WORKDIR")?);
        let state_dir = PathBuf::from(required("TERMPIGEON_STATE_DIR")?);
        let timeout_seconds = env::var("TERMPIGEON_CODEX_TIMEOUT_SECONDS")
            .unwrap_or_else(|_| "1800".to_owned())
            .parse::<u64>()
            .context("TERMPIGEON_CODEX_TIMEOUT_SECONDS must be an unsigned integer")?;
        let dangerously_bypass_approvals_and_sandbox =
            optional_bool("TERMPIGEON_DANGEROUSLY_BYPASS_APPROVALS_AND_SANDBOX", false)?;

        if timeout_seconds == 0 {
            bail!("TERMPIGEON_CODEX_TIMEOUT_SECONDS must be greater than zero");
        }
        for (name, path) in [
            ("TERMPIGEON_CODEX_BIN", &codex_bin),
            ("TERMPIGEON_CODEX_HOME", &codex_home),
            ("TERMPIGEON_WORKDIR", &workdir),
            ("TERMPIGEON_STATE_DIR", &state_dir),
        ] {
            if !path.is_absolute() {
                bail!("{name} must be an absolute path: {}", path.display());
            }
        }
        if !codex_bin.is_file() {
            bail!("Codex executable not found: {}", codex_bin.display());
        }
        if !workdir.is_dir() {
            bail!("working directory not found: {}", workdir.display());
        }

        Ok(Self {
            bot_token,
            owner_user_id,
            codex_bin,
            codex_home,
            workdir,
            state_dir,
            codex_timeout: Duration::from_secs(timeout_seconds),
            dangerously_bypass_approvals_and_sandbox,
        })
    }
}

fn required(name: &str) -> Result<String> {
    let value = env::var(name).with_context(|| format!("missing environment variable {name}"))?;
    let value = value.trim().to_owned();
    if value.is_empty() {
        bail!("environment variable {name} is empty");
    }
    Ok(value)
}

fn optional_bool(name: &str, default: bool) -> Result<bool> {
    let Ok(value) = env::var(name) else {
        return Ok(default);
    };
    parse_bool(name, &value)
}

fn parse_bool(name: &str, value: &str) -> Result<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => bail!("{name} must be true or false"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_boolean_values() {
        for value in ["1", "true", "yes", "on"] {
            assert!(parse_bool("TEST", value).unwrap());
        }
        for value in ["0", "false", "no", "off"] {
            assert!(!parse_bool("TEST", value).unwrap());
        }
        assert!(parse_bool("TEST", "maybe").is_err());
    }
}
