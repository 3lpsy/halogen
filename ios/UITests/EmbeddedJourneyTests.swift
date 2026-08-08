import XCTest

/// The embedded-account journey: boot the on-device server from a blank
/// library, subscribe to a fixture feed served by the recipe's host server,
/// and watch episodes arrive through the embedded poll.
final class EmbeddedJourneyTests: JourneyCase {
    func testEmbeddedFlagshipJourney() {
        launch(autoconnect: "local")
        // Embedded boot provisions a database + admin on first run.
        requireSessionUp(timeout: 60)
        snap("01-embedded-up")

        // Subscribe by URL to a fixture feed (data/tests/*.xml, served from
        // the host server's public dir — the only outbound dependency).
        app.tabBars.buttons["Podcasts"].tap()
        require(app.buttons["podcast-add"], "add-podcast button").tap()
        let title = require(app.textFields["Title"], "title field", timeout: 10)
        title.tap()
        title.typeText("Acquired")
        let url = app.textFields.matching(
            NSPredicate(format: "placeholderValue CONTAINS 'Feed URL'")
        ).firstMatch
        require(url, "feed url field", timeout: 5).tap()
        url.typeText("\(Self.serverBase)/feeds/transistor_acquired.xml")
        snap("02-create-form")
        require(anyElement("podcast-create-submit"), "create submit").tap()

        // The new podcast row appears…
        require(app.staticTexts["Acquired"], "created podcast row", timeout: 30)
        snap("03-podcast-created")

        // Episodes only arrive on a feed poll, so trigger one from the admin
        // Polling page (embedded users are admins) — unhiding it via Configure
        // dock first, which exercises that surface too.
        require(app.tabBars.buttons["More"], "More tab").tap()
        require(app.buttons["Settings"], "Settings entry", timeout: 10).tap()
        scrollTo(app.buttons["Configure dock"], "Configure dock entry").tap()
        scrollTo(anyElement("nav-toggle-polling"), "polling visibility toggle").tap()
        snap("04-polling-unhidden")
        app.navigationBars.buttons.firstMatch.tap()  // back to Settings
        app.navigationBars.buttons.firstMatch.tap()  // back to More
        require(app.buttons["Polling"], "Polling entry", timeout: 10).tap()
        require(anyElement("poll-now"), "poll-now button", timeout: 10).tap()
        snap("05-poll-triggered")

        // …then the fixture feed's episodes land in the podcast.
        app.tabBars.buttons["Podcasts"].tap()
        require(app.staticTexts["Acquired"], "podcast row", timeout: 10).tap()
        require(
            app.buttons.matching(identifier: "row-play").firstMatch, "polled episodes",
            timeout: 90)
        snap("06-episodes-polled")

        // They play locally (embedded = stream from the on-device server).
        tap(
            app.buttons.matching(identifier: "row-play").firstMatch,
            until: "mini-play", "mini player")
        snap("07-playing")

        // Multi-user: create a second device user (admin by policy, secret
        // app-managed) — the app creates it server-side and switches the
        // session to it.
        openMoreRoot()
        require(app.buttons["Settings"], "Settings entry", timeout: 10).tap()
        require(app.buttons["Accounts"], "Accounts entry", timeout: 10).tap()
        require(app.buttons["New user on this device"], "add device user", timeout: 10).tap()
        let user2 = require(app.textFields["Username"], "username field", timeout: 10)
        user2.tap()
        user2.typeText("seconduser")
        require(app.buttons["Create and switch"], "create-and-switch submit", timeout: 5).tap()
        requireSessionUp(timeout: 60)
        // Subscriptions are per-user: the fresh user starts with an empty
        // library (the first user's Acquired subscription is theirs alone).
        app.tabBars.buttons["Podcasts"].tap()
        require(app.staticTexts["No podcasts yet"], "fresh library for user 2", timeout: 20)
        snap("08-second-user")
    }
}
