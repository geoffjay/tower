//! Text formatting shared by the views: clocks, ages, countdowns.

use chrono::TimeZone;

/// Timezone used to render wall-clock times. Tests pin `Fixed` so frames
/// are deterministic; the binary uses `Local` (DST-correct per timestamp).
#[derive(Debug, Clone, Copy)]
pub enum Tz {
    Local,
    Fixed(chrono::FixedOffset),
}

impl Tz {
    pub fn utc() -> Self {
        Tz::Fixed(chrono::FixedOffset::east_opt(0).expect("zero offset"))
    }

    /// `HH:MM:SS`, or `YYYY-MM-DD HH:MM` with the date.
    pub fn clock(&self, ms: i64, with_date: bool) -> String {
        let fmt = if with_date {
            "%Y-%m-%d %H:%M"
        } else {
            "%H:%M:%S"
        };
        match self {
            Tz::Local => chrono::Local
                .timestamp_millis_opt(ms)
                .single()
                .map(|t| t.format(fmt).to_string()),
            Tz::Fixed(off) => off
                .timestamp_millis_opt(ms)
                .single()
                .map(|t| t.format(fmt).to_string()),
        }
        .unwrap_or_else(|| ms.to_string())
    }
}

/// Coarse elapsed time: `40s`, `5m`, `2h`, `3d`.
pub fn age(ms: i64) -> String {
    let s = (ms / 1000).max(0);
    match s {
        0..=59 => format!("{s}s"),
        60..=3599 => format!("{}m", s / 60),
        3600..=86_399 => format!("{}h", s / 3600),
        _ => format!("{}d", s / 86_400),
    }
}

/// Time left until a deadline: `42s`, `4m05s`, `1h02m`; `expired` at or
/// past zero. Rounds up so a live countdown never shows `0s` early.
pub fn countdown(ms_left: i64) -> String {
    if ms_left <= 0 {
        return "expired".into();
    }
    let s = (ms_left + 999) / 1000;
    match s {
        0..=59 => format!("{s}s"),
        60..=3599 => format!("{}m{:02}s", s / 60, s % 60),
        _ => format!("{}h{:02}m", s / 3600, (s % 3600) / 60),
    }
}

/// Cut to `max` chars with a trailing ellipsis.
pub fn trunc(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// First line of a message's text parts (for one-row summaries).
pub fn summary(parts: &[tower_core::Part]) -> String {
    parts
        .iter()
        .find_map(|p| p.text.as_deref())
        .and_then(|t| t.lines().find(|l| !l.trim().is_empty()))
        .unwrap_or("")
        .trim()
        .to_string()
}

/// Wire name of a serde enum (`input-required`, `approval`, ...).
pub fn wire<T: serde::Serialize>(v: T) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn countdown_rounds_up_and_expires_at_zero() {
        assert_eq!(countdown(41_001), "42s");
        assert_eq!(countdown(1), "1s");
        assert_eq!(countdown(0), "expired");
        assert_eq!(countdown(-5), "expired");
        assert_eq!(countdown(245_000), "4m05s");
        assert_eq!(countdown(3_720_000), "1h02m");
    }

    #[test]
    fn trunc_counts_chars_not_bytes() {
        assert_eq!(trunc("héllo wörld", 6), "héllo…");
        assert_eq!(trunc("short", 10), "short");
    }
}
