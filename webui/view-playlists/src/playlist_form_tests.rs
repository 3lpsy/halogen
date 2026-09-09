use super::*;

#[test]
fn clearing_description_updates_optimistic_playlist() {
    let now = chrono::Utc::now();
    let mut playlist = halogen_wire::PlaylistData {
        id: 1,
        name: "Queue".into(),
        description: Some("Old description".into()),
        is_default: true,
        position: 0,
        on_remove_delete_file_server: false,
        on_remove_delete_file_client: false,
        created_at: now,
        updated_at: now,
        episode_ids: None,
        episode_playlist: None,
    };
    let patch = PlaylistUpdateData {
        description: description_field(false, "  \n "),
        ..Default::default()
    };
    assert!(patch.validate().is_ok());
    patch.apply_to(&mut playlist);
    assert_eq!(playlist.description.as_deref(), Some(""));
    assert_eq!(description_field(true, "  "), None);
    assert_eq!(description_field(false, " text ").as_deref(), Some("text"));
}
