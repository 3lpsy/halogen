import SwiftUI

struct ExpandablePodcastDescription: View {
    let description: String
    @State private var expanded = false

    var body: some View {
        let preview = HTMLText.preview(description)
        if !preview.isEmpty {
            VStack(alignment: .leading, spacing: 6) {
                if expanded {
                    HTMLDescription(html: description)
                } else {
                    Text(preview).lineLimit(3)
                }
                Button(expanded ? "Show less" : "Show more") { expanded.toggle() }
                    .font(.subheadline)
                    .accessibilityLabel(expanded ? "Collapse podcast description" : "Expand podcast description")
            }
        }
    }
}
