//! The settings page (D§12.5): UI options the browser remembers. The
//! theme dropdown writes `tower.theme` to localStorage and flips
//! `data-theme` live — no server round trip, nothing to persist server
//! side, so the read-only UI guard (plan T3.2) is untouched. Later
//! settings (whatever they persist to) register here and in the palette's
//! [COMMANDS](crate::shell::COMMANDS).

use topcoat::Result;
use topcoat::context::Cx;
use topcoat::router::page;
use topcoat::view::*;

use crate::shell;
use crate::theme;

#[page("/ui/settings")]
pub async fn settings_page(cx: &Cx) -> Result<impl View> {
    let themes = Unescaped::new_unchecked(theme_options());
    let save = save_js();
    Ok(view! {
        <!DOCTYPE html>
        <html lang="en">
            <head>
                (shell::head(cx, "settings")?)
            </head>
            <body>
                (shell::palette(cx)?)
                <header class="bar">
                    <a class="brand" href="/ui" title="back to the agent cloud">"tower"</a>
                    <span class="quiet">"settings"</span>
                    (shell::palette_button())
                </header>
                <main class="settings">
                    <section>
                        <h1>"Settings"</h1>
                        <h2>"Appearance"</h2>
                        <label>
                            "Theme"
                            (themes)
                            <span class="saved" id="theme-saved">"saved"</span>
                        </label>
                        <p class="quiet">"Kept by this browser (localStorage); the default is "(theme::DEFAULT)"."</p>
                    </section>
                </main>
                <script>(save)</script>
            </body>
        </html>
    })
}

/// `<select>` with one `<option>` per registered theme. The current theme
/// is unknown server-side (it lives in the browser), so `selected` is
/// applied by [SAVE_JS] at boot.
fn theme_options() -> String {
    let options: Vec<String> = theme::THEMES
        .iter()
        .map(|t| format!("<option value=\"{}\">{}</option>", t.id, t.name))
        .collect();
    format!("<select id=\"theme-select\">{}</select>", options.join(""))
}

/// Theme wiring: apply the stored theme to the dropdown at boot, save +
/// apply on change. `ids` comes from [theme::ids_json] — the registry is
/// the one source of truth; `ids[0]` is the default ([theme::DEFAULT]).
fn save_js() -> Unescaped<String> {
    Unescaped::new_unchecked(format!(
        r#"
(function () {{
  const ids = {ids};
  const select = document.getElementById('theme-select');
  const saved = document.getElementById('theme-saved');
  let stored = null;
  try {{ stored = JSON.parse(localStorage.getItem('tower.theme')); }} catch (e) {{}}
  if (!ids.includes(stored)) stored = ids[0];
  select.value = stored;
  select.addEventListener('change', () => {{
    if (!ids.includes(select.value)) return;
    document.documentElement.dataset.theme = select.value;
    localStorage.setItem('tower.theme', JSON.stringify(select.value));
    saved.classList.add('show');
    setTimeout(() => saved.classList.remove('show'), 1500);
  }});
}})();
"#,
        ids = theme::ids_json()
    ))
}
