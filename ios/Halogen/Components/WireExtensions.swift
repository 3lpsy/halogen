import Foundation

/// Hand-written conformances for generated wire types (the generator emits
/// plain Codable enums/structs; UI needs iteration).
extension PlaylistReorderField: CaseIterable {
    public static var allCases: [PlaylistReorderField] {
        [.published, .title, .duration, .added]
    }
}
