//! The scrubbers' own tests live in `tinymemory-safety`. These pin what this
//! module adds: the engine policy and the public `safety::*` paths.

use super::*;
use serde_json::json;

/// A Luhn-valid 13-digit epoch-millisecond timestamp: no network issues a `17`
/// prefix, so the engine must leave it alone.
const TIMESTAMP: &str = "1700000000004";

#[test]
fn engine_policy_leaves_bare_timestamps_alone() {
    let envelope = format!("{{\"ts\": {TIMESTAMP}}}");
    assert_eq!(pii::redact_pii(&envelope).value, envelope);
    assert_eq!(sanitize_text(&envelope).value, envelope);
    let value = json!({ "ts": TIMESTAMP });
    assert_eq!(sanitize_json(&value).value, value);
}

#[test]
fn engine_policy_still_redacts_real_cards_and_secrets() {
    assert!(pii::redact_pii("card 4111111111111111")
        .value
        .contains("[REDACTED_PII_CREDIT_CARD]"));
    assert!(pii::redact_pii("4111111111111111")
        .value
        .contains("[REDACTED_PII_CREDIT_CARD]"));
    let key = format!("sk-{}", "1234567890123456789012345");
    let out = sanitize_text(&format!("key {key}"));
    assert!(!out.value.contains(&key));
    assert!(out.report.text_redactions >= 1);
}

#[test]
fn boundary_predicates_are_reexported() {
    assert!(has_likely_pii("ssn-123-45-6789"));
    assert!(pii::has_likely_pii("ssn-123-45-6789"));
    assert!(has_likely_email("user/alice@example.com"));
    assert!(!has_likely_pii("user/alice@example.com"));
    assert!(has_likely_secret("Bearer abcdefghijklmnop"));
}
