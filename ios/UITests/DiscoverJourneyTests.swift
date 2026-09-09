import XCTest

final class DiscoverJourneyTests: JourneyCase {
    func testCrossPodcastSearchAndSubscribe() {
        launch(autoconnect: "\(Self.serverBase)|dev|dev")
        requireSessionUp()
        openMoreRoot()
        require(app.buttons["Discover"], "Discover entry").tap()
        let mode = require(anyElement("discover-mode"), "search mode")
        XCTAssertTrue(mode.label.contains("By Podcast"))
        mode.tap()
        require(app.buttons["By Episode"], "episode mode").tap()
        let search = require(app.textFields["Search Discover"], "Discover search")
        focusAndType(search, "Meditation", "episode query")
        search.typeText("\n")
        let first = anyElement("discover-episode-Meditation episode 0")
        require(first, "first episode")
        XCTAssertTrue(first.label.contains("Show 0"))
        XCTAssertTrue(
            require(anyElement("discover-episode-Meditation episode 1"), "second podcast").label.contains("Show 1"))
        snap("01-cross-podcast-results")

        let later = anyElement("discover-episode-Meditation episode 29")
        scrollTo(later, "second page episode", maxSwipes: 18)
        snap("02-second-page")
        later.tap()
        require(app.navigationBars["Discover episode"], "remote episode detail")
        require(app.buttons["Subscribe to podcast"], "parent subscription action")
        require(app.buttons["Show 2"], "parent podcast link").tap()
        require(app.navigationBars["Discover podcast"], "remote podcast detail")
        let expand = require(app.buttons["Expand podcast description"], "description expand")
        expand.tap()
        require(app.buttons["Collapse podcast description"], "description collapse").tap()
        require(app.buttons["Expand podcast description"], "collapsed description")
        require(app.buttons["Subscribe"], "subscribe parent").tap()
        require(app.buttons["Subscribed"], "subscribed parent", timeout: 30)
        snap("03-subscribed-parent")
        app.tabBars.buttons["Podcasts"].tap()
        require(app.staticTexts["Show 2"], "subscribed library podcast", timeout: 30).tap()
        require(app.buttons["Expand podcast description"], "saved podcast description")
        snap("04-library-description")
    }
}
