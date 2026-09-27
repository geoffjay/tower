//! Answering a blocked agent's approval dialog (D§8.2).
//!
//! Harness permission menus are position-dependent and not uniform, so
//! tower never sends fixed digits (claude's trust dialog lists "No, exit"
//! first; in its tool prompt option 2 is "Yes, and don't ask again" — a
//! fixed `1`/`2` would exit, or grant a *permanent* permission).
//!
//! - **deny** = `esc`: cancels/refuses in every claude menu.
//! - **approve** = move the cursor (`❯`) to the first plain "Yes" option —
//!   never one that widens the grant ("don't ask again", "always", "all …",
//!   "this session") — then `enter`. No such option on screen → `None`:
//!   the caller fails the delivery instead of guessing.

/// Keys that answer the dialog on `screen` (the pane's visible text).
pub fn answer_keys(screen: &str, approve: bool) -> Option<Vec<String>> {
    if !approve {
        return Some(vec!["esc".into()]);
    }
    let lines: Vec<&str> = screen.lines().collect();
    let cursor = lines.iter().rposition(|l| l.contains('❯'))?;
    let col = label_col(lines[cursor])?;

    // the menu: contiguous lines whose label starts in the cursor's column
    let is_option = |l: &str| label_col(l) == Some(col);
    let mut first = cursor;
    while first > 0 && is_option(lines[first - 1]) {
        first -= 1;
    }
    let mut last = cursor;
    while last + 1 < lines.len() && is_option(lines[last + 1]) {
        last += 1;
    }
    let options: Vec<String> = lines[first..=last].iter().map(|l| label(l)).collect();
    let target = first + options.iter().position(|o| is_plain_yes(o))?;

    let (key, n) = if target >= cursor {
        ("down", target - cursor)
    } else {
        ("up", cursor - target)
    };
    let mut keys = vec![key.to_string(); n];
    keys.push("enter".into());
    Some(keys)
}

/// Column where an option's label text starts: after the `❯` marker (or
/// the matching indent) and any `N.` numbering. `None` for blank lines.
fn label_col(line: &str) -> Option<usize> {
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() && (chars[i] == ' ' || chars[i] == '❯') {
        i += 1;
    }
    (i < chars.len()).then_some(i)
}

fn label(line: &str) -> String {
    let t = line.trim_start_matches([' ', '❯']).trim_end();
    // drop "N. " numbering
    match t.split_once(". ") {
        Some((n, rest)) if !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()) => {
            rest.to_string()
        }
        _ => t.to_string(),
    }
}

fn is_plain_yes(label: &str) -> bool {
    let l = label.to_lowercase();
    let yes = l == "yes" || l.starts_with("yes,") || l.starts_with("yes ");
    let widens = [
        "don't ask",
        "dont ask",
        "always",
        "all ",
        "session",
        "never ask",
        "remember",
    ]
    .iter()
    .any(|w| l.contains(w));
    yes && !widens
}

#[cfg(test)]
mod tests {
    use super::*;

    /// claude 2.x folder-trust dialog, captured live 2026-09-27.
    const TRUST: &str = " Accessing workspace:
 /private/tmp/tower-e2e
 Quick safety check: Is this a project you created or one you trust?
 Claude Code'll be able to read, edit, and execute files here.
 Security guide
 ❯ No, exit
   Yes, I trust this folder
 Enter to confirm · Esc to cancel";

    /// claude tool-permission prompt shape (numbered; option 2 widens).
    const BASH: &str = " Bash command
   echo tower-e2e > proof.txt
 Do you want to proceed?
 ❯ 1. Yes
   2. Yes, and don't ask again for echo commands in /private/tmp/tower-e2e
   3. No, and tell Claude what to do differently (esc)";

    #[test]
    fn trust_dialog_approve_moves_past_no() {
        assert_eq!(answer_keys(TRUST, true).unwrap(), ["down", "enter"]);
    }

    #[test]
    fn tool_prompt_approve_picks_plain_yes_never_dont_ask_again() {
        assert_eq!(answer_keys(BASH, true).unwrap(), ["enter"]);
        // cursor parked on the widening option → move up to plain Yes
        let parked = BASH
            .replace(" ❯ 1. Yes", "   1. Yes")
            .replace("   2. Yes, and", " ❯ 2. Yes, and");
        assert_eq!(answer_keys(&parked, true).unwrap(), ["up", "enter"]);
    }

    #[test]
    fn deny_is_escape_everywhere() {
        for s in [TRUST, BASH, ""] {
            assert_eq!(answer_keys(s, false).unwrap(), ["esc"]);
        }
    }

    #[test]
    fn no_safe_yes_means_no_answer() {
        let only_widening = " Allow edits?
 ❯ 1. Yes, allow all edits during this session
   2. No";
        assert_eq!(answer_keys(only_widening, true), None);
        assert_eq!(answer_keys("no menu on screen", true), None);
    }
}
