//! XDG paths, config, first-run token (D§4, D§13).

use std::path::PathBuf;

use directories::ProjectDirs;

/// All runtime paths for a tower instance.
#[derive(Debug, Clone)]
pub struct Paths {
    pub config_file: PathBuf,
    pub db_file: PathBuf,
    pub artifacts_dir: PathBuf,
    pub token_file: PathBuf,
    pub socket_file: PathBuf,
}

impl Paths {
    /// Resolve the default XDG layout, honoring `TOWER_HOME` override.
    pub fn resolve() -> anyhow::Result<Self> {
        if let Ok(home) = std::env::var("TOWER_HOME") {
            let home = PathBuf::from(home);
            return Ok(Self {
                config_file: home.join("config.toml"),
                db_file: home.join("tower.db"),
                artifacts_dir: home.join("artifacts"),
                token_file: home.join("token"),
                socket_file: home.join("tower.sock"),
            });
        }

        let dirs = ProjectDirs::from("", "", "tower")
            .ok_or_else(|| anyhow::anyhow!("cannot resolve XDG directories"))?;
        let runtime = std::env::var("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir().join("tower-runtime"));

        Ok(Self {
            config_file: dirs.config_dir().join("config.toml"),
            db_file: dirs.data_dir().join("tower.db"),
            artifacts_dir: dirs.data_dir().join("artifacts"),
            token_file: dirs.data_dir().join("token"),
            socket_file: runtime.join("tower.sock"),
        })
    }

    pub fn ensure_dirs(&self) -> anyhow::Result<()> {
        if let Some(p) = self.config_file.parent() {
            std::fs::create_dir_all(p)?;
        }
        if let Some(p) = self.db_file.parent() {
            std::fs::create_dir_all(p)?;
        }
        std::fs::create_dir_all(&self.artifacts_dir)?;
        if let Some(p) = self.socket_file.parent() {
            std::fs::create_dir_all(p)?;
        }
        Ok(())
    }
}

/// Load or generate the bearer token (0600, first run creates it).
#[allow(dead_code)]
pub fn load_or_create_token(paths: &Paths) -> anyhow::Result<String> {
    if paths.token_file.exists() {
        let token = std::fs::read_to_string(&paths.token_file)?;
        let token = token.trim().to_string();
        if !token.is_empty() {
            return Ok(token);
        }
    }
    // 32 random bytes hex-encoded; reuse ulid entropy twice for 52 chars.
    let token = format!("{}{}", tower_core::new_id(), tower_core::new_id())
        .chars()
        .take(52)
        .collect::<String>()
        .to_lowercase();
    std::fs::write(&paths.token_file, &token)?;
    set_owner_only(&paths.token_file)?;
    Ok(token)
}

#[allow(dead_code)]
fn set_owner_only(path: &std::path::Path) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tower_home_override() {
        std::env::set_var("TOWER_HOME", "/tmp/tower-test-home");
        let p = Paths::resolve().unwrap();
        assert_eq!(
            p.db_file,
            std::path::Path::new("/tmp/tower-test-home/tower.db")
        );
        assert_eq!(
            p.token_file,
            std::path::Path::new("/tmp/tower-test-home/token")
        );
        std::env::remove_var("TOWER_HOME");
    }
}
