use super::*;

fn configured() -> ClientConfig {
    ClientConfig {
        access_token: Some("saved-token".into()),
        ..Default::default()
    }
}

#[test]
fn unavailable_successor_keeps_credentials_and_prior_auth_state() {
    for previously_expired in [false, true] {
        let mut config = configured();
        assert_eq!(
            apply_successor_refresh(&mut config, previously_expired, Err(RefreshError::Unusable)),
            previously_expired
        );
        assert_eq!(config.access_token.as_deref(), Some("saved-token"));
    }
    assert!(apply_successor_refresh(
        &mut ClientConfig::default(),
        false,
        Err(RefreshError::Unusable)
    ));
}

#[test]
fn rejected_successor_clears_credentials() {
    let mut config = configured();
    assert!(apply_successor_refresh(
        &mut config,
        false,
        Err(RefreshError::Expired)
    ));
    assert!(config.access_token.is_none());
}

#[test]
fn refreshed_successor_replaces_credentials_and_clears_reauth() {
    let mut config = configured();
    assert!(!apply_successor_refresh(
        &mut config,
        true,
        Ok("fresh-token".into())
    ));
    assert_eq!(config.access_token.as_deref(), Some("fresh-token"));
}

#[test]
fn only_auth_rejection_expires_successor() {
    for status in [401, 403, 408, 429, 500, 503] {
        let mut config = configured();
        let error = classify_refresh_error(ApiError::Server {
            status,
            message: "mock response".into(),
        });
        let expired = apply_successor_refresh(&mut config, false, Err(error));
        assert_eq!(expired, matches!(status, 401 | 403));
        assert_eq!(config.access_token.is_none(), expired);
    }
    let mut config = configured();
    assert!(!apply_successor_refresh(
        &mut config,
        false,
        Err(classify_refresh_error(ApiError::Decode(
            "interrupted response".into()
        )))
    ));
    assert_eq!(config.access_token.as_deref(), Some("saved-token"));
}

#[test]
fn active_account_prefill_uses_server_and_kind() {
    use crate::accounts::StoredAccount;
    let mut registry = Accounts::default();
    for (key, username) in [
        (AccountKey::remote(1, 10), "first"),
        (AccountKey::remote(1, 20), "second"),
        (AccountKey::embedded(1), "local"),
    ] {
        registry.upsert(StoredAccount {
            id: key.id,
            kind: key.kind,
            server: key.server,
            username: username.into(),
            needs_reauth: false,
        });
        registry.set_active(Some(key));
        assert_eq!(registry.active_account().unwrap().username, username);
    }
    registry.set_active(None);
    assert!(registry.active_account().is_none());
}
