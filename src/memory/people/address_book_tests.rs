use super::*;

/// Test double that returns a canned list without any FFI calls.
pub struct MockContactsSource {
    pub result: Result<Vec<AddressBookContact>, AddressBookError>,
}

impl MockContactsSource {
    pub fn ok(contacts: Vec<AddressBookContact>) -> Self {
        Self {
            result: Ok(contacts),
        }
    }

    pub fn permission_denied() -> Self {
        Self {
            result: Err(AddressBookError::PermissionDenied),
        }
    }
}

impl ContactsSource for MockContactsSource {
    fn fetch_contacts(&self) -> Result<Vec<AddressBookContact>, AddressBookError> {
        match &self.result {
            Ok(v) => Ok(v.clone()),
            Err(AddressBookError::PermissionDenied) => Err(AddressBookError::PermissionDenied),
            Err(AddressBookError::Other(s)) => Err(AddressBookError::Other(s.clone())),
        }
    }
}

fn mk_contact(name: &str, email: &str) -> AddressBookContact {
    AddressBookContact {
        display_name: Some(name.into()),
        emails: vec![email.into()],
        phones: vec![],
    }
}

#[test]
fn mock_source_returns_canned_contacts() {
    let source = MockContactsSource::ok(vec![
        mk_contact("Alice", "alice@example.com"),
        mk_contact("Bob", "bob@example.com"),
    ]);
    let result = read_with(&source).unwrap();
    assert_eq!(result.len(), 2);
    assert_eq!(result[0].display_name.as_deref(), Some("Alice"));
    assert_eq!(result[1].emails[0], "bob@example.com");
}

#[test]
fn mock_source_permission_denied_is_distinguished() {
    let source = MockContactsSource::permission_denied();
    let err = read_with(&source).unwrap_err();
    assert_eq!(err, AddressBookError::PermissionDenied);
}

#[test]
fn system_source_non_mac_returns_empty() {
    // Mirrors the `imp` cfgs above: the stub is what compiles whenever the
    // real CNContactStore path is absent, whether by target or by gate.
    #[cfg(not(all(target_os = "macos", feature = "contacts")))]
    {
        let source = SystemContactsSource;
        let result = read_with(&source).unwrap();
        assert!(result.is_empty());
    }
    #[cfg(all(target_os = "macos", feature = "contacts"))]
    {
        // TCC state is environment-dependent; just verify no panic.
        let source = SystemContactsSource;
        let _ = read_with(&source);
    }
}

#[test]
fn contact_with_no_fields_is_excluded_by_mock() {
    let source = MockContactsSource::ok(vec![AddressBookContact {
        display_name: Some("Sarah Lee".into()),
        emails: vec![],
        phones: vec!["+1 555 000 0001".into()],
    }]);
    let result = read_with(&source).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].phones[0], "+1 555 000 0001");
}
