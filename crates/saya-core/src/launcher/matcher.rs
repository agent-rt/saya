//! Subsequence fuzzy matcher with optional MRU bias.
//!
//! Returns `Some(score)` if every character of `query` (already lowercased)
//! appears in `target` in order, else `None`. Higher scores are better.
//!
//! Base bonuses (query/target):
//! - Whole-prefix match (target starts with query): +1000
//! - Word-prefix match (any non-first word starts with query): +500
//!   This is the rule that makes "chr" → "Google Chrome" decisively beat
//!   sparse subsequence matches like "Claude Code URL Handler", which would
//!   otherwise tie on word-boundary bonuses alone.
//! - Each query char hit at a word boundary (start, after ' ' '-' '_' '.' '/'): +60
//! - Each consecutive hit after the first: +30
//!
//! MRU bonus (capped at +500 so prefix matches still win across apps):
//! - Recency band: <7d +400 / <30d +200 / <90d +50 / older 0
//! - Frequency: min(20, count) * 10  (capped +200, scaled so a heavily-used
//!   app outranks a fresh one with same base, but never beats a prefix match
//!   on a more relevant name)

use crate::database::MruInfo;

const RECENCY_LT_7D: i32 = 400;
const RECENCY_LT_30D: i32 = 200;
const RECENCY_LT_90D: i32 = 50;
const FREQ_MULT: i32 = 10;
const FREQ_CAP: i32 = 200;

pub fn score(query: &[char], target: &str, mru: Option<MruInfo>) -> Option<i32> {
    let base = base_score(query, target)?;
    Some(base + mru.map(mru_bonus).unwrap_or(0))
}

fn base_score(query: &[char], target: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    let target_chars: Vec<char> = target.chars().collect();
    let target_lower: Vec<char> = target_chars
        .iter()
        .flat_map(|c| c.to_lowercase())
        .collect();

    let target_lower_string: String = target_lower.iter().collect();
    let is_boundary = |i: usize| -> bool {
        if i == 0 {
            return true;
        }
        matches!(target_chars[i - 1], ' ' | '-' | '_' | '.' | '/')
    };

    let q_string: String = query.iter().collect();

    let mut s: i32 = 0;
    let mut t_idx: usize = 0;
    let mut last_hit: Option<usize> = None;

    for &qc in query {
        let rest = &target_lower[t_idx..];
        let Some(rel) = rest.iter().position(|&c| c == qc) else {
            return None;
        };
        let hit = t_idx + rel;

        if is_boundary(hit) {
            s += 60;
        }
        if let Some(prev) = last_hit
            && hit == prev + 1
        {
            s += 30;
        }

        last_hit = Some(hit);
        t_idx = hit + 1;
    }

    if target_lower_string.starts_with(q_string.as_str()) {
        s += 1000;
    } else if target_lower_string
        .split(|c: char| matches!(c, ' ' | '-' | '_' | '.' | '/'))
        .skip(1)
        .any(|w| w.starts_with(q_string.as_str()))
    {
        s += 500;
    }

    Some(s)
}

fn mru_bonus(m: MruInfo) -> i32 {
    let now = now_ms();
    let days = ((now - m.last_used_ms).max(0) / 86_400_000) as i32;
    let recency = match days {
        0..=6 => RECENCY_LT_7D,
        7..=29 => RECENCY_LT_30D,
        30..=89 => RECENCY_LT_90D,
        _ => 0,
    };
    let freq = ((m.count as i32) * FREQ_MULT).min(FREQ_CAP);
    recency + freq
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(s: &str) -> Vec<char> {
        s.chars().flat_map(|c| c.to_lowercase()).collect()
    }

    #[test]
    fn prefix_beats_subsequence() {
        let prefix = score(&q("saf"), "Safari", None).unwrap();
        let subseq = score(&q("saf"), "Stuff and Affairs", None).unwrap();
        assert!(prefix > subseq, "prefix={prefix} subseq={subseq}");
    }

    #[test]
    fn no_match_returns_none() {
        assert!(score(&q("xyz"), "Safari", None).is_none());
    }

    #[test]
    fn word_boundary_bonus() {
        let with_boundary = score(&q("vsc"), "Visual Studio Code", None).unwrap();
        let no_boundary = score(&q("vsc"), "Verticascadequence", None).unwrap();
        assert!(
            with_boundary > no_boundary,
            "wb={with_boundary} nb={no_boundary}"
        );
    }

    #[test]
    fn case_insensitive() {
        assert!(score(&q("SAF"), "safari", None).is_some());
        assert!(score(&q("saf"), "SAFARI", None).is_some());
    }

    #[test]
    fn mru_boosts_recent_use() {
        let without = score(&q("term"), "Terminal", None).unwrap();
        let with = score(
            &q("term"),
            "Terminal",
            Some(MruInfo { count: 5, last_used_ms: now_ms() - 60_000 }),
        )
        .unwrap();
        assert!(with > without, "without={without} with={with}");
    }

    #[test]
    fn mru_does_not_beat_prefix_on_a_better_match() {
        // Cold "Safari" should still outrank hot "Stack" when query is "saf".
        let cold_prefix = score(&q("saf"), "Safari", None).unwrap();
        let hot_subseq = score(
            &q("saf"),
            "Stack and Affairs",
            Some(MruInfo { count: 100, last_used_ms: now_ms() }),
        )
        .unwrap();
        assert!(cold_prefix > hot_subseq, "cold_prefix={cold_prefix} hot_subseq={hot_subseq}");
    }
}
