//! Scoring: recency × frequency × reciprocity × depth.
//!
//! Each component is deterministic given the same interaction list + `now`
//! timestamp, and each is clamped to `[0,1]`. The composite is the product;
//! clamping the product is redundant but kept for defense-in-depth.
//!
//! Weights (half-life / caps) are module constants so tests are stable.
//! They can move to config later without breaking the API.

use chrono::{DateTime, Utc};

use crate::memory::people::types::{Interaction, ScoreComponents};

/// Recency half-life in days. An interaction this many days old contributes
/// 0.5 to the recency signal; older interactions decay exponentially.
pub const RECENCY_HALF_LIFE_DAYS: f32 = 14.0;

/// Frequency is measured within this rolling window (days). Only interactions
/// more recent than `now - FREQUENCY_WINDOW_DAYS` count toward frequency.
pub const FREQUENCY_WINDOW_DAYS: u32 = 30;

/// Frequency saturates at this many interactions inside `FREQUENCY_WINDOW_DAYS`.
/// 50+ qualifying interactions yields frequency = 1.0.
pub const FREQUENCY_CAP: f32 = 50.0;

/// Depth saturates when the mean message length reaches this many chars.
pub const DEPTH_CAP_CHARS: f32 = 500.0;

/// Compute component scores for a person given their interaction list.
/// `now` is passed in so tests can fix time.
pub fn score(interactions: &[Interaction], now: DateTime<Utc>) -> ScoreComponents {
    if interactions.is_empty() {
        return ScoreComponents {
            recency: 0.0,
            frequency: 0.0,
            reciprocity: 0.0,
            depth: 0.0,
            score: 0.0,
        };
    }

    // Recency: highest-signal (= most recent) interaction drives the score.
    let newest = interactions.iter().map(|i| i.ts).max().unwrap_or(now);
    let age_days = ((now - newest).num_seconds() as f32 / 86_400.0).max(0.0);
    let recency = (-(age_days * 2f32.ln() / RECENCY_HALF_LIFE_DAYS))
        .exp()
        .clamp(0.0, 1.0);

    // Frequency: count within the rolling window, saturated at FREQUENCY_CAP.
    // Using a window (rather than total-ever) prevents an old burst of
    // messages from inflating the score of a now-silent contact.
    let window_cutoff = now - chrono::Duration::days(FREQUENCY_WINDOW_DAYS as i64);
    let window_count = interactions
        .iter()
        .filter(|i| i.ts >= window_cutoff)
        .count() as f32;
    let frequency = (window_count / FREQUENCY_CAP).clamp(0.0, 1.0);

    // Reciprocity: balance of outbound vs inbound — perfect balance = 1.0,
    // all-one-direction = 0.0. Uses all interactions (not windowed) so that
    // the long-term pattern is captured even when recent volume is low.
    let (out_n, in_n) = interactions.iter().fold((0u32, 0u32), |(o, i), x| {
        if x.is_outbound {
            (o + 1, i)
        } else {
            (o, i + 1)
        }
    });
    let reciprocity = if out_n + in_n == 0 {
        0.0
    } else {
        let o = out_n as f32;
        let i = in_n as f32;
        let min = o.min(i);
        let max = o.max(i);
        (min / max).clamp(0.0, 1.0)
    };

    // Depth: mean interaction length, saturated at DEPTH_CAP_CHARS.
    let count = interactions.len() as f32;
    let total_len: u64 = interactions.iter().map(|x| x.length as u64).sum();
    let mean_len = total_len as f32 / count.max(1.0);
    let depth = (mean_len / DEPTH_CAP_CHARS).clamp(0.0, 1.0);

    let composite = (recency * frequency * reciprocity * depth).clamp(0.0, 1.0);

    ScoreComponents {
        recency,
        frequency,
        reciprocity,
        depth,
        score: composite,
    }
}

#[cfg(test)]
#[path = "scorer_tests.rs"]
mod tests;
