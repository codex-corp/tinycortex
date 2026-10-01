//! Core types for the people domain.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Canonical, stable identifier for a person across handles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PersonId(pub Uuid);

impl PersonId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for PersonId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for PersonId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A handle is an opaque label by which the user or a source knows a person.
/// `IMessage(h)` is an iMessage chat handle (phone in E.164, or apple id
/// email). `Email(e)` and `DisplayName(n)` are the other two kinds the A5
/// resolver accepts.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Handle {
    IMessage(String),
    Email(String),
    DisplayName(String),
}

impl Handle {
    /// Return a canonical, case-folded, whitespace-trimmed form used both
    /// for storage and for the resolver lookup key. Emails are lowercased;
    /// iMessage handles strip surrounding whitespace and lowercase email-
    /// style handles; display names are whitespace-collapsed and trimmed.
    pub fn canonicalize(&self) -> Handle {
        match self {
            Handle::IMessage(s) => {
                let t = s.trim();
                // An apple id email handle ("foo@bar.com") is treated the
                // same regardless of case; phone-style handles ("+1…") have
                // no case. Lowercasing is safe for both.
                Handle::IMessage(t.to_lowercase())
            }
            Handle::Email(s) => Handle::Email(s.trim().to_lowercase()),
            Handle::DisplayName(s) => {
                let collapsed: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
                Handle::DisplayName(collapsed)
            }
        }
    }

    /// `(kind, value)` tuple suitable for use as a SQL key.
    pub fn as_key(&self) -> (&'static str, &str) {
        match self {
            Handle::IMessage(s) => ("imessage", s.as_str()),
            Handle::Email(s) => ("email", s.as_str()),
            Handle::DisplayName(s) => ("display_name", s.as_str()),
        }
    }
}

/// Stored representation of a person plus display metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Person {
    pub id: PersonId,
    pub display_name: Option<String>,
    pub primary_email: Option<String>,
    pub primary_phone: Option<String>,
    pub handles: Vec<Handle>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A single interaction observed with a person. The scorer aggregates
/// these. `is_outbound = true` means the user sent it; that's what drives
/// reciprocity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Interaction {
    pub person_id: PersonId,
    pub ts: DateTime<Utc>,
    pub is_outbound: bool,
    /// Token or character count used as a proxy for "depth". Clamped in
    /// scoring; callers may pass e.g. message body length.
    pub length: u32,
}

/// Per-component breakdown of a person-score in `[0,1]`. Exposed so that
/// callers (UI, nudge engine) can explain ranking.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ScoreComponents {
    pub recency: f32,
    pub frequency: f32,
    pub reciprocity: f32,
    pub depth: f32,
    /// Final composite score. `recency * frequency * reciprocity * depth`,
    /// clamped to `[0,1]`.
    pub score: f32,
}

/// Lightweight row returned from the macOS Address Book. We keep this a
/// plain data struct so `address_book::read()` can return the same shape
/// on every OS (empty on non-mac).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddressBookContact {
    pub display_name: Option<String>,
    pub emails: Vec<String>,
    pub phones: Vec<String>,
}

#[cfg(test)]
#[path = "types_tests.rs"]
mod tests;
