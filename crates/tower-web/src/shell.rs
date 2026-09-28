//! Shared page shell (D§12.5): `<head>` with the theme boot script and the
//! command palette chrome. The boot script runs before first paint and
//! applies the browser's stored theme, so a reload never flashes the
//! default; the server can't know the theme (it lives in localStorage),
//! and a cookie would need a mutating endpoint the read-only UI must not
//! grow (plan T3.2).

use topcoat::view::Unescaped;
use topcoat::view::*;

use crate::theme;

pub(crate) const CSS: &str = include_str!("style.css");

/// The theme boot script: `data-theme` on `<html>` from localStorage
/// before first paint, so a reload never flashes the wrong theme. Junk
/// or missing storage falls back to the registered default.
pub(crate) fn theme_boot() -> Unescaped<String> {
    Unescaped::new_unchecked(format!(
        r#"
try {{
  var t = JSON.parse(localStorage.getItem("tower.theme"));
  if (!{ids}.includes(t)) t = {default_:?};
  document.documentElement.dataset.theme = t;
}} catch (e) {{
  document.documentElement.dataset.theme = {default_:?};
}}
"#,
        ids = theme::ids_json(),
        default_ = theme::DEFAULT,
    ))
}

/// `<head>` for every page: charset, viewport, title, the runtime script,
/// the theme variables for every registered theme, and the boot script.
/// Trusted static strings only — no user input reaches this.
pub(crate) fn head<'a>(
    cx: &'a topcoat::context::Cx,
    title: &'a str,
) -> topcoat::Result<topcoat::view::BoxView<'a>> {
    let mut all = String::new();
    for t in theme::THEMES {
        all.push_str(theme::css(t.id).expect("registered theme"));
    }
    all.push_str(theme::STATE_COLOR_FIXES);
    let all = Unescaped::new_unchecked(all);
    // trusted, compile-time CSS; escaping would turn `>` into `&gt;`,
    // which a <style> element does not decode
    let css = Unescaped::new_unchecked(CSS);
    let boot = theme_boot();
    use topcoat::view::ViewExt;
    Ok(view! { cx =>
        <meta charset="utf-8">
        <meta name="viewport" content="width=device-width, initial-scale=1">
        <title>(format!("tower · {title}"))</title>
        topcoat::runtime::script()
        <style>(all)</style>
        <style>(css)</style>
        <script>(boot)</script>
    }
    .boxed())
}

/// Every page's command list: what the palette searches. A new page
/// registers itself here. `(title, hint, url)`.
pub(crate) const COMMANDS: [(&str, &str, &str); 2] = [
    ("Agent cloud", "fleet health at a glance", "/ui"),
    ("Settings", "theme and UI options", "/ui/settings"),
];

/// Palette wiring, embedded as trusted static JS. Kept in one string so
/// the flow reads top-to-bottom; commands come from [COMMANDS].
/// `window.__towerCmds` is set by the script tag in [palette], which
/// runs before this one (script order = document order).
pub(crate) const PALETTE_JS: &str = r#"
(function () {
  const cmds = window.__towerCmds;
  const dlg = document.getElementById('palette');
  const q = document.getElementById('palette-q');
  const box = document.getElementById('palette-results');
  let sel = 0;

  const matches = (cmd, text) => {
    if (!text) return true;
    const hay = (cmd[0] + ' ' + cmd[1]).toLowerCase();
    let i = 0;
    for (const ch of text.toLowerCase()) {
      i = hay.indexOf(ch, i);
      if (i === -1) return false;
      i++;
    }
    return true;
  };
  const hits = () => cmds.filter((c) => matches(c, q.value));

  const render = () => {
    const list = hits();
    if (sel >= list.length) sel = Math.max(0, list.length - 1);
    box.replaceChildren(...list.map((c, i) => {
      const b = document.createElement('button');
      b.type = 'button';
      b.className = 'cmd';
      b.setAttribute('role', 'option');
      if (i === sel) b.setAttribute('aria-selected', 'true');
      const t = document.createElement('span');
      t.textContent = c[0];
      const h = document.createElement('small');
      h.textContent = c[1];
      b.append(t, h);
      b.addEventListener('click', () => location.assign(c[2]));
      return b;
    }));
    if (!list.length) {
      const p = document.createElement('p');
      p.className = 'none';
      p.textContent = 'No matching command';
      box.append(p);
    }
  };

  const move = (d) => {
    const n = hits().length;
    if (!n) return;
    sel = Math.min(Math.max(sel + d, 0), n - 1);
    render();
  };
  const go = () => {
    const list = hits();
    if (list[sel]) location.assign(list[sel][2]);
  };
  const open = () => {
    if (dlg.open) return;
    dlg.showModal();
    q.focus();
    sel = 0;
    render();
  };

  q.addEventListener('input', () => { sel = 0; render(); });
  dlg.addEventListener('close', () => { q.value = ''; sel = 0; render(); });
  // Cmd+K (macOS) / Ctrl+K (Linux, Windows); a second press closes.
  document.addEventListener('keydown', (e) => {
    if (e.key === 'k' && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      if (dlg.open) dlg.close(); else open();
    }
  });
  // Delegated: the header button lives inside a live region on the
  // cloud page and is re-morphed on every render.
  document.addEventListener('click', (e) => {
    if (e.target.closest('[data-palette-open]')) open();
  });
  dlg.addEventListener('keydown', (e) => {
    if (e.key === 'ArrowDown') { e.preventDefault(); move(1); }
    else if (e.key === 'ArrowUp') { e.preventDefault(); move(-1); }
    else if (e.key === 'Enter') { e.preventDefault(); go(); }
  });
  render();
})();
"#;

/// The script tag parts around the commands array: sets `__towerCmds`
/// before [PALETTE_JS] runs (script order = document order).
pub(crate) const PALETTE_JS_HEAD: &str = "<script>window.__towerCmds = ";
pub(crate) const PALETTE_JS_TAIL: &str = ";</script>";

/// The header button that opens the palette (wired by [PALETTE_JS]).
pub(crate) fn palette_button() -> Unescaped<&'static str> {
    Unescaped::new_unchecked(
        r#"<button class="kbtn" type="button" data-palette-open title="command palette (Cmd/Ctrl+K)"><kbd>⌘K</kbd></button>"#,
    )
}

/// The command palette: a `<dialog>` opened with Cmd/Ctrl+K or a
/// [palette_button], fuzzy-searching [COMMANDS]. Plain JS — no signals,
/// no server round trip (D§12.5). Render once per page, outside live
/// regions (its scripts run at parse time only).
pub(crate) fn palette(cx: &topcoat::context::Cx) -> topcoat::Result<topcoat::view::BoxView<'_>> {
    let head = Unescaped::new_unchecked(PALETTE_JS_HEAD);
    let cmds = Unescaped::new_unchecked(command_json());
    let tail = Unescaped::new_unchecked(PALETTE_JS_TAIL);
    let js = Unescaped::new_unchecked(PALETTE_JS);
    Ok(view! { cx =>
        <dialog class="palette" id="palette" aria-label="command palette">
            <div>
                <input id="palette-q" type="text" placeholder="Search commands…"
                    autocomplete="off" spellcheck="false">
                <div class="results" id="palette-results" role="listbox"></div>
            </div>
        </dialog>
        (head)(cmds)(tail)
        <script>(js)</script>
    }
    .boxed())
}

/// JSON array of [COMMANDS] for the palette script. Trusted static data;
/// embedded in a script literal, so quotes are escaped.
fn command_json() -> String {
    let items: Vec<String> = COMMANDS
        .iter()
        .map(|(t, h, u)| {
            format!(
                "[\"{}\", \"{}\", \"{}\"]",
                t.replace('\\', "\\\\").replace('"', "\\\""),
                h.replace('\\', "\\\\").replace('"', "\\\""),
                u.replace('\\', "\\\\").replace('"', "\\\"")
            )
        })
        .collect();
    format!("[{}]", items.join(", "))
}
