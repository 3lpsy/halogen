mod podcast_tests {
    use validator::ValidationErrorsKind;

    use crate::podcast::*;
    use crate::*;

    #[test]
    fn it_rejects_invalid_podcast_data() {
        let create = PodcastStoreData {
            title: "a".repeat(257),
            description: None,
            feed_url: "not-a-url".to_string(),
            art_url: None,
            author: None,
            podcast_config_id: None,
        };
        let r = create.validate();
        assert!(r.is_err());
        let errs = r.unwrap_err().into_errors();
        assert!(errs.contains_key("title"));
        assert!(errs.contains_key("feed_url"));

        let ValidationErrorsKind::Field(title_errs) = errs.get("title").unwrap() else {
            unreachable!("We know this is a Field variant")
        };
        assert!(title_errs.len() == 1);
        assert!(&title_errs[0].code == "length");
    }

    #[test]
    fn it_accepts_valid_podcast_data() {
        let create = PodcastStoreData {
            title: "My Podcast".to_string(),
            description: Some("A fine show".to_string()),
            feed_url: "https://feeds.example.com/show.xml".to_string(),
            art_url: Some("https://cdn.example.com/art.png".to_string()),
            author: Some("Jane".to_string()),
            podcast_config_id: None,
        };
        assert!(create.validate().is_ok());
    }
}

mod user_tests {
    use crate::user::*;
    use crate::*;

    #[test]
    fn user_store_matching_passwords_validates_ok() {
        let data = UserStoreData {
            username: "alice".to_string(),
            password: "supersecret".to_string(),
            password_confirm: "supersecret".to_string(),
            is_admin: None,
        };
        assert!(data.validate().is_ok());
    }

    #[test]
    fn user_store_mismatched_passwords_fails_must_match() {
        let data = UserStoreData {
            username: "alice".to_string(),
            password: "supersecret".to_string(),
            password_confirm: "different1".to_string(),
            is_admin: None,
        };
        let errs = data.validate().unwrap_err();
        let map = errs.into_errors();
        // The mismatch surfaces on `password_confirm` with the `must_match` code.
        let validator::ValidationErrorsKind::Field(field_errs) =
            map.get("password_confirm").expect("password_confirm error")
        else {
            unreachable!("expected Field variant");
        };
        assert!(
            field_errs.iter().any(|e| e.code == "must_match"),
            "expected a must_match error, got {field_errs:?}"
        );
    }

    #[test]
    fn password_update_matching_validates_ok() {
        let data = PasswordUpdateData {
            password: "supersecret".to_string(),
            password_confirm: "supersecret".to_string(),
        };
        assert!(data.validate().is_ok());
    }

    #[test]
    fn password_update_mismatch_fails_must_match() {
        let data = PasswordUpdateData {
            password: "supersecret".to_string(),
            password_confirm: "nomatch12".to_string(),
        };
        let errs = data.validate().unwrap_err();
        assert!(errs.into_errors().contains_key("password_confirm"));
    }
}

mod enums_tests {
    use crate::enums::*;

    #[test]
    fn download_status_roundtrip_all_variants() {
        let variants = [
            (DownloadStatus::NotDownloaded, "NOT_DOWNLOADED"),
            (DownloadStatus::Downloading, "DOWNLOADING"),
            (DownloadStatus::Downloaded, "DOWNLOADED"),
            (DownloadStatus::DownloadError, "DOWNLOAD_ERROR"),
            (DownloadStatus::DownloadBroken, "DOWNLOAD_BROKEN"),
            (
                DownloadStatus::DownloadUnauthorized,
                "DOWNLOAD_UNAUTHORIZED",
            ),
            (
                DownloadStatus::DownloadRemoteNotFound,
                "DOWNLOAD_REMOTE_NOT_FOUND",
            ),
        ];
        for (variant, s) in variants {
            assert_eq!(variant.as_str(), s);
            assert_eq!(variant.to_string(), s);
            assert_eq!(DownloadStatus::from_string(s), variant);
        }
    }

    #[test]
    fn download_status_unknown_falls_back_to_default() {
        assert_eq!(
            DownloadStatus::from_string("WHATEVER"),
            DownloadStatus::default()
        );
        assert_eq!(DownloadStatus::default(), DownloadStatus::NotDownloaded);
        assert_eq!(
            DownloadStatus::from_string(""),
            DownloadStatus::NotDownloaded
        );
    }

    #[test]
    fn playback_status_roundtrip_all_variants() {
        let variants = [
            (PlaybackStatus::Unplayed, "UNPLAYED"),
            (PlaybackStatus::Played, "PLAYED"),
            (PlaybackStatus::Finished, "FINISHED"),
        ];
        for (variant, s) in variants {
            assert_eq!(variant.as_str(), s);
            assert_eq!(variant.to_string(), s);
            assert_eq!(PlaybackStatus::from_string(s), variant);
        }
    }

    #[test]
    fn playback_status_unknown_falls_back_to_default() {
        assert_eq!(
            PlaybackStatus::from_string("nope"),
            PlaybackStatus::default()
        );
        assert_eq!(PlaybackStatus::default(), PlaybackStatus::Unplayed);
        assert_eq!(PlaybackStatus::from_string(""), PlaybackStatus::Unplayed);
    }
}

mod polling_outcome_tests {
    use crate::polling::*;

    #[test]
    fn outcome_classification() {
        assert_eq!(outcome_for(0, 0, 0, true), PodcastPollOutcome::Skipped);
        assert_eq!(outcome_for(0, 0, 1, false), PodcastPollOutcome::Error);
        assert_eq!(outcome_for(2, 0, 0, false), PodcastPollOutcome::Polled);
        // Partial errors alongside real work still count as polled.
        assert_eq!(outcome_for(1, 0, 1, false), PodcastPollOutcome::Polled);
    }
}

mod discover_tests {
    use crate::discover::*;

    #[test]
    fn provider_wire_form_is_lowercase() {
        assert_eq!(
            serde_json::to_string(&DiscoverProvider::Gpodder).unwrap(),
            "\"gpodder\""
        );
        assert_eq!(DiscoverProvider::Itunes.as_str(), "itunes");
    }

    #[test]
    fn search_data_roundtrips() {
        let data = DiscoverSearchData {
            items: vec![DiscoverResultItem {
                id: "abc123".into(),
                provider: DiscoverProvider::Itunes,
                title: "The Daily".into(),
                feed_url: "https://example.com/feed.xml".into(),
                description: String::new(),
                author: Some("NYT".into()),
            }],
            errors: vec![DiscoverProviderError {
                provider: DiscoverProvider::Gpodder,
                message: "timeout".into(),
            }],
        };
        let json = serde_json::to_string(&data).unwrap();
        let back: DiscoverSearchData = serde_json::from_str(&json).unwrap();
        assert_eq!(data, back);
    }

    #[test]
    fn search_data_tolerates_missing_optional_fields() {
        // No `errors`, no `description`, no `author` — all default.
        let json = r#"{"items":[{"id":"x","provider":"gpodder","title":"T","feed_url":"u"}]}"#;
        let data: DiscoverSearchData = serde_json::from_str(json).unwrap();
        assert_eq!(data.items.len(), 1);
        assert!(data.items[0].description.is_empty());
        assert!(data.items[0].author.is_none());
        assert!(data.errors.is_empty());
    }
}
