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

/// Cloud zoom wiring, embedded as trusted static JS (the palette-script
/// pattern, D§12.3 zoom): the server renders `#cloud`'s viewBox as the
/// fit frame; this script scales that frame around an anchor point —
/// wheel toward the cursor, `+`/`-`/reset via the `.zoom` overlay (the
/// buttons live inside a live region and are re-morphed on every
/// render, so clicks are delegated at the document level). No
/// `stopPropagation`: the background-click clear-selection and point
/// clicks keep working. A MutationObserver re-applies the zoom after
/// each Topcoat morph replaces the server's viewBox, so zoom persists
/// across refreshes; self-writes are skipped by value.
pub(crate) const ZOOM_JS: &str = r#"
(function () {
  var MIN = 0.25, MAX = 8;
  var svg = document.getElementById('cloud');
  if (!svg) return;
  var base = null; // the server's fit frame, replaced on each morph
  var view = null; // the effective frame: base narrowed by the zoom

  // viewBox "x y w h" -> {x, y, w, h}; null when absent or malformed
  function parse(s) {
    var p = (s || '').split(/\s+/).map(Number);
    if (p.length !== 4 || p.some(isNaN) || p[2] <= 0 || p[3] <= 0) return null;
    return { x: p[0], y: p[1], w: p[2], h: p[3] };
  }
  function viewBox() {
    return view.x + ' ' + view.y + ' ' + view.w + ' ' + view.h;
  }
  function write() {
    if (!view) return;
    // same value = a self-write from an earlier zoom step; skip so the
    // observer never loops and never rebases its own zoom away
    if (svg.getAttribute('viewBox') !== viewBox())
      svg.setAttribute('viewBox', viewBox());
  }

  // narrow the view by k around anchor (ax, ay) in view coordinates;
  // k > 1 zooms in. The drift, tooltips, and hit-testing are untouched:
  // only the viewBox changes, never the positions (D§12.3).
  function zoomAt(k, ax, ay) {
    view.x = ax - (ax - view.x) / k;
    view.y = ay - (ay - view.y) / k;
    view.w /= k;
    view.h /= k;
  }
  function scale() {
    return base ? base.w / view.w : 1;
  }

  // a server morph replaced the viewBox with a fresh fit frame: adopt
  // it as the new base and re-project the old view onto it, so the
  // zoom and the anchor stay steady across refreshes. The frames this
  // script itself writes are skipped by value — only a real server
  // frame (fit) rebases, or the zoom would compound into the base.
  function rebase() {
    var attr = svg.getAttribute('viewBox');
    if (!view || attr === viewBox()) return; // our own write
    var fresh = parse(attr);
    if (!fresh) return;
    if (!base) { base = fresh; }
    // fractions of the old base where the old view's center sits
    var fx = (view.x + view.w / 2 - base.x) / base.w;
    var fy = (view.y + view.h / 2 - base.y) / base.h;
    var k = base.w / view.w; // the kept zoom
    base = fresh;
    view = { x: fresh.x, y: fresh.y, w: fresh.w, h: fresh.h };
    zoomAt(k, fresh.x + fx * fresh.w, fresh.y + fy * fresh.h);
    write();
  }

  function wheel(e) {
    e.preventDefault(); // zoom, don't scroll — and never stopPropagation:
    if (!view) return;   // the bg click (clear selection) must still run
    // ctrlKey = trackpad pinch; plain wheel zooms in steps. Factor from
    // gesture speed, capped so one event can never jump far
    var step = e.ctrlKey ? 1 + Math.min(Math.abs(e.deltaY) / 100, 0.25)
                         : 1 + Math.min(Math.abs(e.deltaY) / 400, 0.1);
    var k = e.deltaY < 0 ? step : 1 / step;
    var next = scale() * k;
    if (next < MIN || next > MAX) return;
    // pointer position -> user coordinates, so the zoom centers on it
    var pt = svg.createSVGPoint();
    pt.x = e.clientX;
    pt.y = e.clientY;
    pt = pt.matrixTransform(svg.getScreenCTM().inverse());
    zoomAt(k, pt.x, pt.y);
    write();
  }

  function button(k) {
    return function () {
      if (!view) return;
      if (k === 0) { view = { x: base.x, y: base.y, w: base.w, h: base.h }; write(); return; } // reset: the fit frame
      var target = scale() * k;
      if (target < MIN || target > MAX) return;
      var c = { x: view.x + view.w / 2, y: view.y + view.h / 2 };
      zoomAt(target / scale(), c.x, c.y);
      write();
    };
  }

  svg.addEventListener('wheel', wheel, { passive: false });
  // the buttons live inside a live region (re-morphed every render);
  // delegate their clicks, like the palette button
  document.addEventListener('click', function (e) {
    var z = e.target.closest('[data-zoom-in], [data-zoom-out], [data-zoom-reset]');
    if (!z) return;
    if (z.hasAttribute('data-zoom-in')) button(1.35)();
    else if (z.hasAttribute('data-zoom-out')) button(1 / 1.35)();
    else button(0)();
  });
  new MutationObserver(rebase).observe(svg, {
    attributes: true,
    attributeFilter: ['viewBox'],
  });
  // boot: adopt the server's first frame as both base and view (a copy:
  // zooming mutates `view` in place and must never touch `base`)
  var first = parse(svg.getAttribute('viewBox'));
  if (first) {
    base = first;
    view = { x: first.x, y: first.y, w: first.w, h: first.h };
  }
  rebase();
})();
"#;

/// The cloud's zoom controls, as one raw node: the `data-*` attributes
/// are wiring for [ZOOM_JS], delegated like [palette_button]'s are for
/// [PALETTE_JS]. Rendered inside a live region on the cloud page, so
/// no state lives here.
pub(crate) fn zoom_controls() -> Unescaped<&'static str> {
    Unescaped::new_unchecked(
        r#"<div class="zoom">
  <button type="button" data-zoom-in title="zoom in — wheel up or pinch out">+</button>
  <button type="button" data-zoom-out title="zoom out — wheel down or pinch in">−</button>
  <button type="button" data-zoom-reset title="reset zoom to fit">⤢</button>
</div>"#,
    )
}

/// The zoom script, as an unescaped node. Trusted static JS — no user
/// input reaches this (D§12.3 zoom).
pub(crate) fn zoom_script() -> Unescaped<String> {
    Unescaped::new_unchecked(format!("<script>{}</script>", ZOOM_JS))
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
