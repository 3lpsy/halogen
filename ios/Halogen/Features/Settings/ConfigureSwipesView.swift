import SwiftUI

/// Configure per-page episode swipe actions (the web's
/// `/settings/configure-swipes`, scoped to the browsing lists — structural
/// pages keep their built-in swipes).
struct ConfigureSwipesView: View {
    @Bindable var swipes: SwipePrefsModel

    var body: some View {
        List {
            ForEach(SwipePage.allCases) { page in
                Section(page.label) {
                    picker(page: page, edge: "Leading (full swipe)", keyPath: \.leading)
                    picker(page: page, edge: "Trailing", keyPath: \.trailing)
                }
            }
        }
        .navigationTitle("Configure swipes")
        .navigationBarTitleDisplayMode(.inline)
    }

    private func picker(
        page: SwipePage, edge: String, keyPath: WritableKeyPath<SwipePrefs.PagePrefs, SwipeAction>
    ) -> some View {
        Picker(
            edge,
            selection: Binding(
                get: { swipes.prefs.page(page)[keyPath: keyPath] },
                set: { new in
                    var prefs = swipes.prefs.page(page)
                    prefs[keyPath: keyPath] = new
                    swipes.update(page, leading: prefs.leading, trailing: prefs.trailing)
                }
            )
        ) {
            // Per-page vocabulary: remove-from-list only where the list IS a
            // playlist (web: SwipePage::allowed_actions).
            ForEach(page.allowedActions) { action in
                Text(action.label).tag(action)
            }
        }
    }
}
