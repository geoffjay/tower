//! Server config (D§4). All keys optional; defaults in code.

use serde::Deserialize;

use crate::paths::Paths;

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub server: ServerConfig,
    pub herdr: HerdrConfig,
    pub inbox: InboxConfig,
    #[serde(rename = "harness")]
    pub harnesses: HarnessConfigs,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct InboxConfig {
    pub blocked_deadline_s: i64,
}

impl Default for InboxConfig {
    fn default() -> Self {
        Self {
            blocked_deadline_s: 24 * 60 * 60,
        }
    }
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
pub struct HarnessConfigs {
    #[serde(default)]
    pub claude: HarnessConfig,
    #[serde(default)]
    pub pi: HarnessConfig,
    #[serde(default)]
    pub omp: HarnessConfig,
    #[serde(default)]
    pub opencode: HarnessConfig,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct HarnessConfig {
    pub permission_mode: Option<String>,
    pub args: Vec<String>,
}

impl HarnessConfigs {
    /// Extra args configured for a harness kind (`[harness.<kind>] args`).
    /// Unknown kinds have no section, so they get none.
    pub fn args_for(&self, kind: &str) -> &[String] {
        match kind {
            "claude" => &self.claude.args,
            "pi" => &self.pi.args,
            "omp" => &self.omp.args,
            "opencode" => &self.opencode.args,
            _ => &[],
        }
    }
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
        assert_eq!(cfg.inbox.blocked_deadline_s, 24 * 60 * 60);
    }

    #[test]
    fn blocked_inbox_deadline_can_be_configured() {
        let cfg: Config = toml::from_str("[inbox]\nblocked_deadline_s = 7200\n").unwrap();
        assert_eq!(cfg.inbox.blocked_deadline_s, 7200);
    }

    #[test]
    fn harness_args_are_read_per_kind() {
        let cfg: Config = toml::from_str(
            "[harness.omp]\nargs = [\"--smol\"]\n[harness.pi]\nargs = [\"--plan\"]\n\
             [harness.opencode]\nargs = [\"--auto\"]\n",
        )
        .unwrap();
        assert_eq!(cfg.harnesses.args_for("omp"), ["--smol"]);
        assert_eq!(cfg.harnesses.args_for("pi"), ["--plan"]);
        assert_eq!(cfg.harnesses.args_for("opencode"), ["--auto"]);
        assert_eq!(cfg.harnesses.args_for("claude"), Vec::<String>::new());
        assert_eq!(cfg.harnesses.args_for("other"), Vec::<String>::new());
    }
}
