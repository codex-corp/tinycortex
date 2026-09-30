//! Secret-detection and redaction helpers for memory writes.
//!
//! The scrubbers themselves — credential patterns, the sensitive-key
//! classifier, the JSON depth cap and the checksum-gated multilingual
//! national-ID PII module — live in the shared `tinymemory-safety` crate, which
//! the OpenHuman host and `tinymemory-core` use as well (it used to be copied
//! three times). This module keeps the engine's public `safety::*` paths and
//! pins the engine's one policy choice: a *bare* (separator-less) Luhn-valid
//! digit run is only redacted as a credit card when corroborated by a real
//! network IIN or a nearby card keyword, so 13-digit epoch-millisecond
//! timestamps in stored JSON envelopes are not corrupted (opencompany#1201).
//!
//! The write-rejection boundary ([`has_likely_pii`](tinymemory_safety::has_likely_pii)) stays stricter than
//! content scrubbing: formatted national IDs are rejected, while phone/email-like
//! text is scrubbed from content without rejecting every write that mentions
//! them.

use serde_json::Value;

pub use tinymemory_safety::{
    has_likely_email, has_likely_pii, has_likely_secret, BareCardGate, Policy, SanitizationReport,
    Sanitized,
};

/// The scrubbing policy every engine write uses.
const ENGINE_POLICY: Policy = Policy::corroborated();

/// Scrub secrets and PII from free text, returning the cleaned text plus a
/// [`SanitizationReport`].
pub fn sanitize_text(value: &str) -> Sanitized<String> {
    tinymemory_safety::sanitize_text_with(value, ENGINE_POLICY)
}

/// Recursively scrub a JSON value: sensitive keys are replaced wholesale and
/// every string value runs through [`sanitize_text`].
pub fn sanitize_json(value: &Value) -> Sanitized<Value> {
    tinymemory_safety::sanitize_json_with(value, ENGINE_POLICY)
}

/// Personal-PII detection and redaction (national IDs, financial identifiers,
/// international phone), on-device, regex + checksum only.
pub mod pii {
    use super::{Sanitized, ENGINE_POLICY};

    pub use tinymemory_safety::pii::{has_likely_email, has_likely_pii};

    /// Redact format-based multilingual PII from `text` under the engine policy.
    pub fn redact_pii(text: &str) -> Sanitized<String> {
        tinymemory_safety::pii::redact_pii_with(text, ENGINE_POLICY)
    }
}

#[cfg(test)]
#[path = "safety_tests.rs"]
mod tests;
