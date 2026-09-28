//! Themes (D§12.5): one CSS custom-property set per theme, selected in
//! Settings and remembered by the browser. The default is tower dark
//! (the original colors); the Tokyo Night palettes come from
//! <https://wixdaq.github.io/Tokyo-Night-Website/palette.html>.
//!
//! State colors keep their meaning in every theme (amber = blocked, red =
//! dead, …) — only the chrome colors change per theme, so the cloud stays
//! readable in light mode: points stay saturated, text stays dark-on-light.

/// Every theme: `(id, display name)`. Order = dropdown order; [DEFAULT] first.
pub const THEMES: [Theme; 3] = [
    Theme {
        id: "tower-dark",
        name: "Tower Dark (default)",
    },
    Theme {
        id: "tokyo-night-storm",
        name: "Tokyo Night Storm",
    },
    Theme {
        id: "tokyo-night-light",
        name: "Tokyo Night Light",
    },
];

/// The default theme id.
pub const DEFAULT: &str = "tower-dark";

/// One selectable theme.
pub struct Theme {
    pub id: &'static str,
    pub name: &'static str,
}

/// JSON array of every registered theme id, for the browser scripts. One
/// source of truth: the boot and settings scripts use this list, so a
/// new theme can't be forgotten there.
pub fn ids_json() -> String {
    format!(
        "[{}]",
        THEMES
            .iter()
            .map(|t| format!("\"{}\"", t.id))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

/// The CSS for one theme's chrome variables, or `None` for an unknown id.
pub fn css(id: &str) -> Option<&'static str> {
    match id {
        "tower-dark" => Some(
            r#"
:root, [data-theme="tower-dark"] {
  --bg: #0b0e14;
  --fg: #c9d1d9;
  --dim: #6e7681;
  --hi: #fff;
  --card: #151a23;
  --line: #262d38;
  --accent: #3b82f6;
  --on-accent: #fff;
}
"#,
        ),
        // Tokyo Night Storm: bg/card from the #1a1b26/#24283b background
        // family, fg #c0caf5, dim #565f89, line #414868; accent = blue
        // #7aa2f7 with the dark bg as its text color.
        "tokyo-night-storm" => Some(
            r#"
[data-theme="tokyo-night-storm"] {
  --bg: #1a1b26;
  --fg: #c0caf5;
  --dim: #565f89;
  --hi: #c0caf5;
  --card: #24283b;
  --line: #414868;
  --accent: #7aa2f7;
  --on-accent: #1a1b26;
}
"#,
        ),
        // Tokyo Night Light: bg/card from the #d5d6db/#e6e7ed family, fg
        // #343b58, dim #6c6e75; accent = blue #2959aa with light text.
        "tokyo-night-light" => Some(
            r#"
[data-theme="tokyo-night-light"] {
  --bg: #d5d6db;
  --fg: #343b58;
  --dim: #6c6e75;
  --hi: #343b58;
  --card: #e6e7ed;
  --line: #b7b9c0;
  --accent: #2959aa;
  --on-accent: #e6e7ed;
}
"#,
        ),
        _ => None,
    }
}

/// CSS that adapts state colors for light themes (readable dark text on
/// light background, e.g. white point labels).
pub const STATE_COLOR_FIXES: &str = r#"
[data-theme="tokyo-night-light"] .pt:hover .label,
[data-theme="tokyo-night-light"] .pt[data-selected] .label {
  fill: #343b58;
}
[data-theme="tokyo-night-light"] .pt[data-selected] .core {
  stroke: #343b58;
}
[data-theme="tokyo-night-light"] .snippet {
  background: #00000014;
}
[data-theme="tokyo-night-light"] .panel {
  box-shadow: 0 12px 40px #343b5826;
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_lists_default_first_and_every_theme_has_css() {
        assert_eq!(THEMES[0].id, DEFAULT);
        for t in THEMES {
            assert!(css(t.id).is_some(), "{} has no css", t.id);
        }
    }

    #[test]
    fn ids_json_lists_every_theme_quoted() {
        assert_eq!(
            ids_json(),
            "[\"tower-dark\", \"tokyo-night-storm\", \"tokyo-night-light\"]"
        );
    }

    #[test]
    fn only_the_default_theme_applies_without_an_attribute() {
        // Same specificity: a second `:root` block would override the
        // default by source order alone.
        for t in THEMES {
            let root = css(t.id).unwrap().contains(":root");
            assert_eq!(root, t.id == DEFAULT, "{}", t.id);
        }
    }
}
