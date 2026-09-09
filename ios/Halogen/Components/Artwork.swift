import SwiftUI

/// Square rounded artwork tile with a placeholder. URLs point at the server's
/// art cache; ArtLoader fetches them with the API bearer token.
struct Artwork: View {
    let url: URL?
    /// Scales with the UI-size pref (Dynamic Type): callers pass the base
    /// (100%) point size and the tile grows with the text around it, so a
    /// bigger UI doesn't pair big labels with shrunken-looking art.
    @ScaledMetric private var size: CGFloat

    @State private var image: UIImage?

    init(url: URL?, size: CGFloat) {
        self.url = url
        _size = ScaledMetric(wrappedValue: size, relativeTo: .body)
    }

    var body: some View {
        ZStack {
            if let image {
                Image(uiImage: image).resizable().scaledToFill()
            } else {
                Rectangle().fill(.quaternary)
                Image(systemName: "waveform")
                    .foregroundStyle(.secondary)
            }
        }
        .frame(width: size, height: size)
        .clipShape(RoundedRectangle(cornerRadius: size / 7))
        .task(id: url) {
            image = nil
            guard let url else { return }
            let loaded = await ArtLoader.shared.image(for: url)
            guard !Task.isCancelled else { return }
            image = loaded
        }
    }
}
