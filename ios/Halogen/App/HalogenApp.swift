import SwiftUI

@main
struct HalogenApp: App {
    init() {
        // The dock: UIKit fixes the tab bar's HEIGHT, but the item content is
        // ours — bump the labels (system default ≈10pt) so the dock reads
        // larger alongside the scaled-up UI (UISize.dynamicType). Same font
        // for normal/selected: only color should change on selection.
        let items = UITabBarItemAppearance()
        let dockFont = UIFont.systemFont(ofSize: 13, weight: .medium)
        items.normal.titleTextAttributes = [.font: dockFont]
        items.selected.titleTextAttributes = [.font: dockFont]
        let bar = UITabBarAppearance()
        bar.stackedLayoutAppearance = items
        bar.inlineLayoutAppearance = items
        bar.compactInlineLayoutAppearance = items
        UITabBar.appearance().standardAppearance = bar
        UITabBar.appearance().scrollEdgeAppearance = bar

        #if DEBUG
            // E2E hook (`just ios-e2e`): start from a blank device state —
            // containers, keychain registry, defaults — so journeys are
            // hermetic without erasing the whole simulator between tests.
            if ProcessInfo.processInfo.environment["HALOGEN_RESET"] == "1" {
                let fm = FileManager.default
                if let support = try? fm.url(
                    for: .applicationSupportDirectory, in: .userDomainMask,
                    appropriateFor: nil, create: false)
                {
                    for name in ["halogen-client", "halogen-server", "device-log.json"] {
                        try? fm.removeItem(at: support.appendingPathComponent(name))
                    }
                }
                SessionStore.clear()
                EmbeddedSecrets.clear()
                if let bundle = Bundle.main.bundleIdentifier {
                    UserDefaults.standard.removePersistentDomain(forName: bundle)
                }
            }
        #endif
    }

    var body: some Scene {
        WindowGroup {
            RootView()
        }
    }
}
