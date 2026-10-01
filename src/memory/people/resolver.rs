//! HandleResolver — deterministic mapping (Handle) → PersonId.
//!
//! Given the same store contents, resolving the same handle twice returns
//! the same `PersonId`. If the handle is unknown and `create_if_missing`
//! is set, the resolver mints a new `PersonId`, inserts a `Person` skeleton
//! with the handle attached, and returns the new id.
//!
//! `seed_from_address_book` wires the `address_book` read path into the
//! resolver so that contacts from the system address book are pre-populated
//! as `Person` rows (and their handles are registered for future resolution).

use chrono::Utc;

use crate::memory::people::address_book::{self, AddressBookError, ContactsSource};
use crate::memory::people::store::PeopleStore;
use crate::memory::people::types::{Handle, Person, PersonId};

pub struct HandleResolver<'a> {
    store: &'a PeopleStore,
}

impl<'a> HandleResolver<'a> {
    pub fn new(store: &'a PeopleStore) -> Self {
        Self { store }
    }

    /// Look up the person for a handle. Returns `None` if unknown.
    pub async fn resolve(&self, handle: &Handle) -> Result<Option<PersonId>, String> {
        let canonical = handle.canonicalize();
        self.store
            .lookup(&canonical)
            .await
            .map_err(|e| format!("lookup: {e}"))
    }

    /// Look up or mint. Display-name / email fields on the newly-minted
    /// `Person` are populated from the handle itself so the UI has
    /// something to render before any enrichment runs.
    pub async fn resolve_or_create(&self, handle: &Handle) -> Result<PersonId, String> {
        self.resolve_or_create_with_status(handle)
            .await
            .map(|(id, _created)| id)
    }

    pub async fn resolve_or_create_with_status(
        &self,
        handle: &Handle,
    ) -> Result<(PersonId, bool), String> {
        let canonical = handle.canonicalize();
        let id = PersonId::new();
        let (display_name, primary_email, primary_phone) = match &canonical {
            Handle::DisplayName(s) => (Some(s.clone()), None, None),
            Handle::Email(s) => (None, Some(s.clone()), None),
            Handle::IMessage(s) => {
                if s.contains('@') {
                    (None, Some(s.clone()), None)
                } else {
                    (None, None, Some(s.clone()))
                }
            }
        };
        let now = Utc::now();
        let person = Person {
            id,
            display_name,
            primary_email,
            primary_phone,
            handles: vec![canonical.clone()],
            created_at: now,
            updated_at: now,
        };
        self.store
            .resolve_or_insert_person(&person, &canonical)
            .await
            .map_err(|e| format!("resolve_or_insert_person: {e}"))
    }

    /// Merge: attach `other` as an alias on the person `primary` resolves to.
    /// Useful for the sync path that learns "this email and this phone
    /// belong to the same contact".
    pub async fn link(&self, primary: &Handle, other: Handle) -> Result<PersonId, String> {
        let pid = self.resolve_or_create(primary).await?;
        let other = other.canonicalize();
        self.store
            .add_alias(pid, other)
            .await
            .map_err(|e| format!("add_alias: {e}"))?;
        Ok(pid)
    }

    /// Seed the people store from the system address book.
    ///
    /// For each contact returned by `source`:
    ///   - Pick the first email or phone as the "primary" handle and look it
    ///     up or mint a `PersonId`.
    ///   - Link any additional emails / phones as aliases on the same person.
    ///   - If only a display name is present, mint via display name.
    ///
    /// Contacts that produce no handles at all are skipped. This is
    /// idempotent: re-running on the same contact list is a no-op because
    ///`lookup` finds existing handle rows.
    ///
    /// Returns `(seeded, skipped)` counts, and propagates `AddressBookError`
    /// to let callers distinguish permission-denied from other failures.
    pub async fn seed_from_address_book(
        &self,
        source: &dyn ContactsSource,
    ) -> Result<(usize, usize), AddressBookError> {
        let contacts = address_book::read_with(source)?;
        let mut seeded = 0usize;
        let mut skipped = 0usize;

        for c in contacts {
            // Build a flat list of all handles for this contact.
            let mut handles: Vec<Handle> = Vec::new();
            for email in &c.emails {
                let trimmed = email.trim();
                if !trimmed.is_empty() {
                    handles.push(Handle::Email(trimmed.to_string()));
                }
            }
            for phone in &c.phones {
                let trimmed = phone.trim();
                if !trimmed.is_empty() {
                    handles.push(Handle::IMessage(trimmed.to_string()));
                }
            }
            if let Some(ref name) = c.display_name {
                let trimmed = name.trim();
                if !trimmed.is_empty() {
                    handles.push(Handle::DisplayName(trimmed.to_string()));
                }
            }

            if handles.is_empty() {
                skipped += 1;
                continue;
            }

            // The "primary" handle is the first email if present, otherwise
            // the first phone, otherwise the display name. This gives the
            // most stable link target for future interactions.
            let primary = handles[0].clone();

            // mint or look up the primary handle
            match self.resolve_or_create(&primary).await {
                Err(e) => {
                    log::warn!(
                        "[people::resolver] seed_from_address_book: failed to upsert primary handle {:?}: {e}",
                        primary.as_key()
                    );
                    skipped += 1;
                    continue;
                }
                Ok(pid) => {
                    // link all additional handles as aliases
                    for alias in handles.into_iter().skip(1) {
                        if let Err(e) = self.store.add_alias(pid, alias.canonicalize()).await {
                            log::warn!(
                                "[people::resolver] seed_from_address_book: add_alias failed: {e}"
                            );
                        }
                    }
                    seeded += 1;
                }
            }
        }

        log::debug!(
            "[people::resolver] seed_from_address_book done: seeded={seeded} skipped={skipped}"
        );
        Ok((seeded, skipped))
    }
}

#[cfg(test)]
#[path = "resolver_tests.rs"]
mod tests;
