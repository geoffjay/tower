//! CLI client library (D§10, plan T6.1): thin transport over /v1.
//!
//! Amendment (v1): reqwest has no unix-socket support; the CLI speaks TCP
//! with the token auto-loaded from the tower data dir (or TOWER_TOKEN).
//! The unix socket remains available for agents/tools that can use UDS.
//! The token is read once at startup — no config needed, same UX.
//!
//! Paths resolve exactly like the server's (`TOWER_HOME`, else the platform
//! dirs — `~/Library/Application Support/tower` on macOS), and the address
//! comes from the same `config.toml` (`[server] bind_tcp`), so the CLI
//! finds whichever server that home configures. `TOWER_URL` overrides.

use anyhow::{anyhow, Context};

#[derive(Clone)]
pub struct Client {
    base: reqwest::Url,
    token: String,
    /// Request/response calls: bounded total time.
    http: reqwest::Client,
    /// SSE: no total deadline (streams run for hours); a read timeout
    /// longer than the server's 15s keep-alive detects a dead peer.
    sse: reqwest::Client,
}

impl Client {
    /// Connect to the server. Token resolution order: explicit arg,
    /// `TOWER_TOKEN` env, token file in the tower data dir.
    pub fn connect(token: Option<String>) -> anyhow::Result<Self> {
        let token = match token {
            Some(t) => t,
            None => match std::env::var("TOWER_TOKEN") {
                Ok(t) => t,
                Err(_) => read_token_file()?,
            },
        };
        Ok(Self::with_base(base_url()?, token))
    }

    /// Explicit server address + token (tests, embedding).
    pub fn with_base(base: reqwest::Url, token: impl Into<String>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(65))
            .build()
            .expect("reqwest client");
        let sse = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(5))
            .read_timeout(std::time::Duration::from_secs(45))
            .build()
            .expect("reqwest client");
        Self {
            base,
            token: token.into(),
            http,
            sse,
        }
    }

    pub async fn get(&self, path: &str) -> anyhow::Result<serde_json::Value> {
        let resp = self
            .http
            .get(self.url(path)?)
            .bearer_auth(&self.token)
            .send()
            .await?;
        Self::parse(resp).await
    }

    pub async fn post(
        &self,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> anyhow::Result<serde_json::Value> {
        let mut req = self.http.post(self.url(path)?).bearer_auth(&self.token);
        if let Some(b) = body {
            req = req.json(&b);
        }
        let resp = req.send().await?;
        Self::parse(resp).await
    }

    pub async fn delete(&self, path: &str) -> anyhow::Result<serde_json::Value> {
        let resp = self
            .http
            .delete(self.url(path)?)
            .bearer_auth(&self.token)
            .send()
            .await?;
        Self::parse(resp).await
    }

    /// SSE stream: raw response for line iteration.
    pub async fn stream(&self, path: &str) -> anyhow::Result<reqwest::Response> {
        let resp = self
            .sse
            .get(self.url(path)?)
            .bearer_auth(&self.token)
            .header("accept", "text/event-stream")
            .send()
            .await?;
        if !resp.status().is_success() {
            anyhow::bail!("stream {} failed: {}", path, resp.status());
        }
        Ok(resp)
    }

    /// Absolute URL of `path` on this server (e.g. links for a browser).
    pub fn url(&self, path: &str) -> anyhow::Result<reqwest::Url> {
        Ok(self.base.join(path.trim_start_matches('/'))?)
    }

    async fn parse(resp: reqwest::Response) -> anyhow::Result<serde_json::Value> {
        let status = resp.status();
        let text = resp.text().await?;
        let v: serde_json::Value = serde_json::from_str(&text)
            .with_context(|| format!("non-JSON response ({status}): {text}"))?;
        if !status.is_success() {
            let code = v["error"]["code"].as_str().unwrap_or("error");
            let msg = v["error"]["message"].as_str().unwrap_or("unknown");
            return Err(anyhow!("{code}: {msg}"));
        }
        Ok(v)
    }
}

/// (config file, token file) — same resolution as the server's `Paths`.
fn home_files() -> anyhow::Result<(std::path::PathBuf, std::path::PathBuf)> {
    if let Ok(home) = std::env::var("TOWER_HOME") {
        let home = std::path::PathBuf::from(home);
        return Ok((home.join("config.toml"), home.join("token")));
    }
    let dirs = directories::ProjectDirs::from("", "", "tower").ok_or_else(|| {
        anyhow!("cannot resolve the tower data dir; set TOWER_HOME or TOWER_TOKEN")
    })?;
    Ok((
        dirs.config_dir().join("config.toml"),
        dirs.data_dir().join("token"),
    ))
}

/// `TOWER_URL`, else `http://<[server].bind_tcp>` from config.toml (a
/// wildcard bind is reached on loopback), else the default port.
fn base_url() -> anyhow::Result<reqwest::Url> {
    if let Ok(u) = std::env::var("TOWER_URL") {
        return u.parse().with_context(|| format!("TOWER_URL {u:?}"));
    }
    let (config, _) = home_files()?;
    let bind = std::fs::read_to_string(&config)
        .ok()
        .and_then(|s| s.parse::<toml::Table>().ok())
        .and_then(|t| {
            t.get("server")?
                .get("bind_tcp")?
                .as_str()
                .map(str::to_string)
        })
        .unwrap_or_else(|| "127.0.0.1:8266".into());
    let bind = bind
        .replace("0.0.0.0:", "127.0.0.1:")
        .replace("[::]:", "127.0.0.1:");
    format!("http://{bind}")
        .parse()
        .with_context(|| format!("bind_tcp {bind:?} in {}", config.display()))
}

fn read_token_file() -> anyhow::Result<String> {
    let (_, file) = home_files()?;
    let token = std::fs::read_to_string(&file).with_context(|| {
        format!(
            "cannot read token file {} (is the server started?)",
            file.display()
        )
    })?;
    let token = token.trim().to_string();
    if token.is_empty() {
        anyhow::bail!("token file {} is empty", file.display());
    }
    Ok(token)
}
