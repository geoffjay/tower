//! CLI client library (D§10, plan T6.1): thin transport over /v1.
//!
//! Amendment (v1): reqwest has no unix-socket support; the CLI speaks TCP
//! with the token auto-loaded from the tower data dir (or TOWER_TOKEN).
//! The unix socket remains available for agents/tools that can use UDS.
//! The token is read once at startup — no config needed, same UX.

use anyhow::{anyhow, Context};

#[derive(Clone)]
pub struct Client {
    base: reqwest::Url,
    token: String,
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
        Ok(Self {
            base: "http://127.0.0.1:8266".parse().unwrap(),
            token,
        })
    }

    pub async fn get(&self, path: &str) -> anyhow::Result<serde_json::Value> {
        let resp = self
            .client()
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
        let mut req = self.client().post(self.url(path)?).bearer_auth(&self.token);
        if let Some(b) = body {
            req = req.json(&b);
        }
        let resp = req.send().await?;
        Self::parse(resp).await
    }

    /// SSE stream: raw response for line iteration.
    pub async fn stream(&self, path: &str) -> anyhow::Result<reqwest::Response> {
        let resp = self
            .client()
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

    fn client(&self) -> reqwest::Client {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(65))
            .build()
            .expect("reqwest client")
    }

    fn url(&self, path: &str) -> anyhow::Result<reqwest::Url> {
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

fn read_token_file() -> anyhow::Result<String> {
    let file = if let Ok(home) = std::env::var("TOWER_HOME") {
        std::path::PathBuf::from(home).join("token")
    } else {
        match std::env::var("XDG_DATA_HOME") {
            Ok(x) => std::path::PathBuf::from(x).join("tower").join("token"),
            Err(_) => {
                let home = std::env::var("HOME").context("$HOME unset; set TOWER_TOKEN")?;
                std::path::PathBuf::from(home)
                    .join(".local/share/tower")
                    .join("token")
            }
        }
    };
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
