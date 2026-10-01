use super::*;
use crate::memory::people::types::PersonId;
use chrono::Duration;

fn mk(ts: DateTime<Utc>, outbound: bool, length: u32) -> Interaction {
    Interaction {
        person_id: PersonId::new(),
        ts,
        is_outbound: outbound,
        length,
    }
}

#[test]
fn empty_interactions_score_zero() {
    let s = score(&[], Utc::now());
    assert_eq!(s.score, 0.0);
    assert_eq!(s.recency, 0.0);
    assert_eq!(s.frequency, 0.0);
}

#[test]
fn recency_half_life_matches_config() {
    let now = Utc::now();
    let half_ago = now - Duration::days(RECENCY_HALF_LIFE_DAYS as i64);
    let s = score(&[mk(half_ago, true, 100)], now);
    // Half-life point → recency ≈ 0.5 (allow small float slack).
    assert!((s.recency - 0.5).abs() < 0.05, "got {}", s.recency);
}

#[test]
fn all_components_clamped_to_unit_interval() {
    let now = Utc::now();
    let interactions: Vec<Interaction> = (0..200)
        .map(|i| mk(now - Duration::hours(i), i % 2 == 0, 10_000))
        .collect();
    let s = score(&interactions, now);
    for c in [s.recency, s.frequency, s.reciprocity, s.depth, s.score] {
        assert!((0.0..=1.0).contains(&c), "component out of range: {c}");
    }
    // 200 interactions all within a few days → window_count ≥ FREQUENCY_CAP
    assert_eq!(s.frequency, 1.0);
    assert_eq!(s.depth, 1.0);
}

#[test]
fn one_sided_conversation_has_zero_reciprocity() {
    let now = Utc::now();
    let v: Vec<_> = (0..5)
        .map(|i| mk(now - Duration::hours(i), true, 100))
        .collect();
    let s = score(&v, now);
    assert_eq!(s.reciprocity, 0.0);
    assert_eq!(
        s.score, 0.0,
        "composite must be zero when any factor is zero"
    );
}

#[test]
fn deterministic_given_same_inputs() {
    let now = Utc::now();
    let v = vec![
        mk(now - Duration::days(1), true, 100),
        mk(now - Duration::days(2), false, 150),
        mk(now - Duration::days(3), true, 200),
    ];
    let a = score(&v, now);
    let b = score(&v, now);
    assert_eq!(a.score, b.score);
    assert_eq!(a.recency, b.recency);
}

#[test]
fn old_burst_does_not_inflate_frequency_score() {
    // 100 interactions from 90 days ago (outside FREQUENCY_WINDOW_DAYS=30)
    // should contribute 0 to frequency; 1 interaction today should give
    // 1/FREQUENCY_CAP.
    let now = Utc::now();
    let mut v: Vec<Interaction> = (0..100)
        .map(|i| mk(now - Duration::days(90 + i), true, 100))
        .collect();
    // Add one recent interaction to avoid zero reciprocity forcing score=0
    v.push(mk(now - Duration::hours(1), false, 100));
    let s = score(&v, now);
    // Only 1 interaction falls within the 30-day window.
    let expected_frequency = 1.0 / FREQUENCY_CAP;
    assert!(
        (s.frequency - expected_frequency).abs() < 0.001,
        "frequency should be {expected_frequency}, got {}",
        s.frequency
    );
}

#[test]
fn interactions_exactly_at_window_boundary_are_included() {
    let now = Utc::now();
    // Interaction exactly FREQUENCY_WINDOW_DAYS ago — should be included
    // (boundary is inclusive via >=).
    let boundary = now - Duration::days(FREQUENCY_WINDOW_DAYS as i64);
    let v = vec![
        mk(boundary, true, 100),
        mk(now - Duration::hours(1), false, 100),
    ];
    let s = score(&v, now);
    let expected = 2.0 / FREQUENCY_CAP;
    assert!(
        (s.frequency - expected).abs() < 0.001,
        "expected {expected} got {}",
        s.frequency
    );
}
