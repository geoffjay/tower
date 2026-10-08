//! Named agent definitions on disk (D§4, D§10).
//!
//! A definition lives in a subfolder of the tower config dir
//! (`~/.config/tower/agents/<name>/` by default, `TOWER_HOME` honored):
//!
//! ```text
//! ~/.config/tower/agents/foo/PROMPT.md   # first prompt
//! ~/.config/tower/agents/foo/config.toml # spawn arguments (kind, workdir, ...)
//! ```
//!
//! `tower spawn --name foo` reads the definition and expands it into the
//! ordinary `/v1/agents` spawn body; explicit CLI flags override it.

use std::path::PathBuf;

use serde::Deserialize;

/// Spawn arguments defined in `<name>/config.toml`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct AgentDef {
    /// Harness kind (pi, claude, ...)
    pub kind: Option<String>,
    /// Working directory for the agent
    pub workdir: Option<String>,
    /// Isolated git worktree
    pub worktree: bool,
}

/// A resolved on-disk definition.
#[derive(Debug, Clone)]
pub struct ResolvedDef {
    pub def: AgentDef,
    pub prompt: Option<String>,
}

/// Load `agents/<name>/` from the config dir. `agents_dir` is the directory
/// that holds one subfolder per definition (e.g. `~/.config/tower/agents`).
pub fn load(agents_dir: &std::path::Path, name: &str) -> anyhow::Result<ResolvedDef> {
    let dir = def_dir(agents_dir, name)?;
    let def_path = dir.join("config.toml");
    let mut def = AgentDef::default();
    if def_path.exists() {
        def = toml::from_str(&std::fs::read_to_string(&def_path)?)?;
    }
    let prompt_path = dir.join("PROMPT.md");
    let prompt = if prompt_path.exists() {
        Some(std::fs::read_to_string(prompt_path)?)
    } else {
        None
    };
    if def == AgentDef::default() && prompt.is_none() {
        anyhow::bail!(
            "agent definition '{}' has no config.toml and no PROMPT.md (expected in {})",
            name,
            dir.display()
        );
    }
    Ok(ResolvedDef { def, prompt })
}

/// Names of all definitions in `agents_dir`, sorted.
pub fn names(agents_dir: &std::path::Path) -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(agents_dir) {
        for e in entries.flatten() {
            if e.path().is_dir() {
                if let Some(n) = e.file_name().to_str() {
                    out.push(n.to_owned());
                }
            }
        }
    }
    out.sort();
    out
}

/// Definition folder, validated to exist and be a directory (no traversal).
fn def_dir(agents_dir: &std::path::Path, name: &str) -> anyhow::Result<PathBuf> {
    if name.is_empty()
        || name.contains('/')
        || name.contains('\\')
        || name == "."
        || name == ".."
        || name != name.trim()
    {
        anyhow::bail!("invalid agent definition name '{name}'");
    }
    let dir = agents_dir.join(name);
    if !dir.is_dir() {
        let known = names(agents_dir);
        anyhow::bail!(
            "no agent definition named '{name}' in {}{}",
            agents_dir.display(),
            if known.is_empty() {
                String::new()
            } else {
                format!(" (known: {})", known.join(", "))
            }
        );
    }
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_def(root: &std::path::Path, name: &str, config: Option<&str>, prompt: Option<&str>) {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        if let Some(c) = config {
            std::fs::write(dir.join("config.toml"), c).unwrap();
        }
        if let Some(p) = prompt {
            std::fs::write(dir.join("PROMPT.md"), p).unwrap();
        }
    }

    fn tmp() -> PathBuf {
        let d = std::env::temp_dir().join(format!("tower-defs-{}", tower_core::new_id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn loads_config_and_prompt() {
        let root = tmp();
        write_def(
            &root,
            "foo",
            Some("kind = \"pi\"\nworktree = true\n"),
            Some("hello"),
        );
        let r = load(&root, "foo").unwrap();
        assert_eq!(r.def.kind.as_deref(), Some("pi"));
        assert!(r.def.worktree);
        assert_eq!(r.prompt.as_deref(), Some("hello"));
    }

    #[test]
    fn prompt_only_is_valid() {
        let root = tmp();
        write_def(&root, "bar", None, Some("just a prompt"));
        let r = load(&root, "bar").unwrap();
        assert_eq!(r.def, AgentDef::default());
        assert_eq!(r.prompt.as_deref(), Some("just a prompt"));
    }

    #[test]
    fn empty_definition_is_rejected() {
        let root = tmp();
        write_def(&root, "empty", None, None);
        assert!(load(&root, "empty").is_err());
    }

    #[test]
    fn missing_definition_lists_known_names() {
        let root = tmp();
        write_def(&root, "foo", None, Some("p"));
        let err = load(&root, "nope").unwrap_err().to_string();
        assert!(err.contains("known: foo"), "err: {err}");
    }

    #[test]
    fn name_traversal_is_rejected() {
        let root = tmp();
        let err = load(&root, "../etc").unwrap_err().to_string();
        assert!(err.contains("invalid agent definition name"));
    }

    #[test]
    fn names_lists_sorted_dirs_only() {
        let root = tmp();
        write_def(&root, "b", None, Some("p"));
        write_def(&root, "a", None, Some("p"));
        std::fs::write(root.join("stray.txt"), "x").unwrap();
        assert_eq!(names(&root), vec!["a", "b"]);
    }
}
