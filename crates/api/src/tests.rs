use crate::ApiClient;
use url::Url;

#[test]
fn client_new() {
    let client = ApiClient::new(Url::parse("http://localhost:8080").unwrap());
    assert!(client.token().is_none());
}

#[test]
fn client_set_token() {
    let client = ApiClient::new(Url::parse("http://localhost:8080").unwrap());
    client.set_token(Some("abc123".to_string()));
    assert_eq!(client.token(), Some("abc123".to_string()));
}
