use super::*;

#[test]
fn proxied_backend_envelope_decodes_provider_response() {
    let response = decode_proxy_response(serde_json::json!({
        "success": true,
        "data": {
            "successful": true,
            "data": {"messages": [{"messageId": "message-1"}]},
            "error": null
        }
    }))
    .unwrap();

    assert!(response.successful);
    assert_eq!(response.data["messages"][0]["messageId"], "message-1");
}

#[test]
fn flat_proxy_response_remains_supported() {
    let response = decode_proxy_response(serde_json::json!({
        "successful": true,
        "data": {"items": [1]}
    }))
    .unwrap();

    assert!(response.successful);
    assert_eq!(response.data["items"], serde_json::json!([1]));
}
