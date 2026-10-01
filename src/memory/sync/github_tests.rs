use super::*;

#[test]
fn parses_supported_repo_urls() {
    assert_eq!(
        parse_repo("https://github.com/tinyhumansai/openhuman.git").unwrap(),
        ("tinyhumansai".into(), "openhuman".into())
    );
    assert_eq!(
        parse_repo("git@github.com:tinyhumansai/openhuman").unwrap(),
        ("tinyhumansai".into(), "openhuman".into())
    );
}

#[test]
fn maps_raw_item_coordinates() {
    assert_eq!(
        raw_coordinates("issue:42"),
        Some((RawKind::Issue, "42".into()))
    );
    assert_eq!(raw_coordinates("unknown:42"), None);
}
