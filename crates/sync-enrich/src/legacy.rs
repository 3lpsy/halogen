/// Preserve scalar episode IDs from queues written before bulk operations existed.
pub(crate) fn normalize_operation(value: &mut serde_json::Value) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    for name in [
        "AddToPlaylist",
        "RemoveFromPlaylist",
        "TriggerDownload",
        "RemoveServerDownload",
    ] {
        let Some(fields) = object
            .get_mut(name)
            .and_then(serde_json::Value::as_object_mut)
        else {
            continue;
        };
        if !fields.contains_key("episode_ids")
            && let Some(id) = fields.remove("episode_id")
        {
            fields.insert("episode_ids".into(), serde_json::Value::Array(vec![id]));
        }
    }
}
