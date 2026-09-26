//! Server config (DESIGN.md §4). All keys optional; defaults in code.

use serde::Deserialize;

use crate::paths::Paths;

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub server: ServerConfig,
    pub herdr: HerdrConfig,
    #[serde(rename = "harness")]
    pub harnesses: HarnessConfigs,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    pub bind_socket: bool,
    pub bind_tcp: String,
    pub event_retention_days: u32,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind_socket: true,
            bind_tcp: "127.0.0.1:8266".into(),
            event_retention_days: 14,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct HerdrConfig {
    pub socket_path: String,
    pub cli_fallback: bool,
}

impl Default for HerdrConfig {
    fn default() -> Self {
        Self {
            socket_path: "~/.config/herdr/herdr.sock".into(),
            cli_fallback: true,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct HarnessConfigs {
    pub claude: HarnessConfig,
    pub pi: HarnessConfig,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct HarnessConfig {
    pub permission_mode: Option<String>,
    pub args: Vec<String>,
}

impl Config {
    /// Load from the XDG config file; missing file = defaults.
    pub fn load(paths: &Paths) -> anyhow::Result<Self> {
        if !paths.config_file.exists() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(&paths.config_file)?;
        let cfg: Config = toml::from_str(&raw)?;
        Ok(cfg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_gives_defaults() {
        let p = Paths::resolve().unwrap();
        let cfg = Config::load(&p).unwrap();
        assert_eq!(cfg.server.bind_tcp, "127.0.0.1:8266");
        assert!(cfg.server.bind_socket);
        assert!(cfg.herdr.cli_fallback);
    }
}
