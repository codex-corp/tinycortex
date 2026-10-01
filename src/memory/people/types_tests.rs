use super::*;

#[test]
fn handle_canonicalize_lowercases_emails_and_imessage() {
    assert_eq!(
        Handle::Email("  Foo@Example.COM ".into()).canonicalize(),
        Handle::Email("foo@example.com".into())
    );
    assert_eq!(
        Handle::IMessage("+1 (555) 123".into()).canonicalize(),
        Handle::IMessage("+1 (555) 123".into())
    );
    assert_eq!(
        Handle::IMessage(" Foo@Bar.com ".into()).canonicalize(),
        Handle::IMessage("foo@bar.com".into())
    );
}

#[test]
fn handle_canonicalize_collapses_display_name_whitespace() {
    assert_eq!(
        Handle::DisplayName("  Sarah   Lee  ".into()).canonicalize(),
        Handle::DisplayName("Sarah Lee".into())
    );
}

#[test]
fn handle_as_key_returns_correct_kind() {
    assert_eq!(Handle::Email("a@b.c".into()).as_key(), ("email", "a@b.c"));
    assert_eq!(Handle::IMessage("+1".into()).as_key(), ("imessage", "+1"));
    assert_eq!(
        Handle::DisplayName("X".into()).as_key(),
        ("display_name", "X")
    );
}
