//! Themes (D§12.5): one CSS custom-property set per theme, selected in
//! Settings and remembered by the browser. The default is tower dark
//! (the original colors); the Tokyo Night palettes come from
//! <https://wixdaq.github.io/Tokyo-Night-Website/palette.html>.
//!
//! A theme sets both the chrome colors (`--bg --fg --dim --hi --card
//! --line --accent --on-accent`) and the agent state colors (`--working
//! --blocked --idle --done --dead --launching --unknown`, D§12.1). The
//! hue *meaning* is fixed across themes (amber = blocked/needs you, blue
//! = working, …) so the glance channels never change; the exact values
//! come from each theme's palette.

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
        // The original colors. State hues: working=blue, blocked=amber,
        // idle=gray, done=green, dead=red, launching=teal, unknown=violet.
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
  --working: #3b82f6;
  --blocked: #f59e0b;
  --idle: #6b7280;
  --done: #22c55e;
  --dead: #ef4444;
  --launching: #14b8a6;
  --unknown: #8b5cf6;
}
"#,
        ),
        // Tokyo Night Storm: chrome from the #1a1b26/#24283b background
        // family, fg #c0caf5, dim #565f89, line #414868, accent blue
        // #7aa2f7. State colors from the Storm palette, same hue meaning:
        // blue #7aa2f7, amber #e0af68, gray #565f89, green #9ece6a,
        // red #f7768e, teal #2ac3de, violet #bb9af7.
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
  --working: #7aa2f7;
  --blocked: #e0af68;
  --idle: #565f89;
  --done: #9ece6a;
  --dead: #f7768e;
  --launching: #2ac3de;
  --unknown: #bb9af7;
}
"#,
        ),
        // Tokyo Night Light: chrome from the #d5d6db/#e6e7ed family, fg
        // #343b58, dim #6c6e75, accent blue #2959aa. State colors from
        // the Light palette, same hue meaning: blue #2959aa,
        // amber #8f5e15, gray #6c6e75, green #385f0d, red #8c4351,
        // teal #006c86, violet #5a3e8e.
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
  --working: #2959aa;
  --blocked: #8f5e15;
  --idle: #6c6e75;
  --done: #385f0d;
  --dead: #8c4351;
  --launching: #006c86;
  --unknown: #5a3e8e;
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
