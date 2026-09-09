import Foundation

extension OutboxOp {
    /// Convert the legacy platform payload to the shared Rust operation vocabulary.
    func sharedOperation() throws -> QueuedOperation {
        let encoded = try JSONEncoder().encode(kind)
        guard let wrapped = try JSONSerialization.jsonObject(with: encoded) as? [String: [String: Any]],
            let (kind, fields) = wrapped.first
        else { throw CocoaError(.coderInvalidValue) }
        let names = [
            "setCursor": "SetCursor", "setPlayed": "MarkPlayed", "addToPlaylist": "AddToPlaylist",
            "removeFromPlaylist": "RemoveFromPlaylist", "moveInPlaylist": "MoveInPlaylist",
            "reorderPlaylist": "ReorderPlaylist", "updatePlaylist": "UpdatePlaylist",
            "movePlaylist": "MovePlaylist", "subscribe": "Subscribe", "unsubscribe": "Unsubscribe",
            "triggerDownload": "TriggerDownload", "removeServerDownload": "RemoveServerDownload",
            "updatePodcastConfig": "UpdatePodcastConfig", "removePodcastConfig": "RemovePodcastConfig",
            "setAutoPlaylists": "SetPodcastAutoPlaylists",
        ]
        guard let name = names[kind] else { throw CocoaError(.coderInvalidValue) }
        var payload: [String: Any] = [:]
        for (key, value) in fields {
            if key == "data", let config = value as? [String: Any] {
                payload.merge(config) { _, new in new }
            } else {
                let snake = key.reduce(into: "") { output, character in
                    if character.isUppercase { output += "_" }
                    output += character.lowercased()
                }
                payload[snake] = value
            }
        }
        if name == "UpdatePlaylist" {
            payload["on_remove_delete_file_server"] = payload.removeValue(forKey: "delete_server_file")
            payload["on_remove_delete_file_client"] = payload.removeValue(forKey: "delete_client_file")
        }
        if ["AddToPlaylist", "RemoveFromPlaylist", "TriggerDownload", "RemoveServerDownload"].contains(name),
            let episode = payload.removeValue(forKey: "episode_id")
        {
            payload["episode_ids"] = [episode]
        }
        let data = try JSONSerialization.data(withJSONObject: [name: payload], options: [.sortedKeys])
        guard let json = String(data: data, encoding: .utf8) else { throw CocoaError(.coderInvalidValue) }
        return QueuedOperation(id: id.uuidString, operationJson: json)
    }
}

extension OutboxOp {
    /// Rebuild the UI mirror when the app stopped after journal commit but before cache persistence.
    static func restoreSharedQueue(_ json: String) throws -> [OutboxOp] {
        guard let entries = try JSONSerialization.jsonObject(with: Data(json.utf8)) as? [[Any]] else {
            throw CocoaError(.coderInvalidValue)
        }
        let names = [
            "SetCursor": "setCursor", "MarkPlayed": "setPlayed", "AddToPlaylist": "addToPlaylist",
            "RemoveFromPlaylist": "removeFromPlaylist", "MoveInPlaylist": "moveInPlaylist",
            "ReorderPlaylist": "reorderPlaylist", "UpdatePlaylist": "updatePlaylist",
            "MovePlaylist": "movePlaylist", "Subscribe": "subscribe", "Unsubscribe": "unsubscribe",
            "TriggerDownload": "triggerDownload", "RemoveServerDownload": "removeServerDownload",
            "UpdatePodcastConfig": "updatePodcastConfig", "RemovePodcastConfig": "removePodcastConfig",
            "SetPodcastAutoPlaylists": "setAutoPlaylists",
        ]
        return try entries.compactMap { entry in
            guard entry.count == 2, let record = entry[1] as? [String: Any],
                let id = record["source_id"] as? String,
                let operation = record["operation"] as? [String: [String: Any]],
                let (sharedName, source) = operation.first, let name = names[sharedName]
            else { throw CocoaError(.coderInvalidValue) }
            if let rejection = record["rejection"], !(rejection is NSNull) { return nil }
            var payload: [String: Any] = [:]
            if name == "updatePodcastConfig" {
                payload["configId"] = source["config_id"]
                payload["data"] = source.filter { $0.key != "config_id" }
            } else {
                for (key, value) in source {
                    let parts = key.split(separator: "_")
                    let camel =
                        String(parts.first ?? "")
                        + parts.dropFirst().map { $0.prefix(1).uppercased() + $0.dropFirst() }.joined()
                    payload[camel] = value
                }
                if let ids = payload.removeValue(forKey: "episodeIds") as? [Any] {
                    guard ids.count == 1 else { throw CocoaError(.coderInvalidValue) }
                    payload["episodeId"] = ids[0]
                }
                if name == "updatePlaylist" {
                    payload["deleteServerFile"] = payload.removeValue(forKey: "onRemoveDeleteFileServer")
                    payload["deleteClientFile"] = payload.removeValue(forKey: "onRemoveDeleteFileClient")
                }
            }
            let data = try JSONSerialization.data(withJSONObject: ["id": id, "kind": [name: payload]])
            return try JSONDecoder().decode(OutboxOp.self, from: data)
        }
    }
}
