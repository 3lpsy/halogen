import XCTest

/// The remote-account flagship journey against the recipe's seeded server:
/// sign-in, browsing, admin, playlists, download chain, Up Next, offline→online
/// outbox round-trip, relaunch persistence. Ordering rule: FORM flows run before
/// anything plays — the mini player floats over forms and steals submit taps.
final class RemoteJourneyTests: JourneyCase {
    func testRemoteFlagshipJourney() {
        launch(autoconnect: "\(Self.serverBase)|dev|dev")
        requireSessionUp()
        snap("01-signed-in")

        // Latest: the seeded library renders rows (anchored on each row's
        // play button — container identifiers don't surface from SwiftUI).
        app.tabBars.buttons["Latest"].tap()
        // Generous first-content timeout: on a loaded ephemeral build VM the
        // first fetch+render can exceed the interactive-feeling 20s default.
        require(
            app.buttons.matching(identifier: "row-play").firstMatch, "Latest episode rows",
            timeout: 60)
        snap("02-latest-rows")

        // Admin surfaces exist for an admin account.
        openMoreRoot()
        require(app.buttons["Settings"], "Settings entry", timeout: 10).tap()
        require(app.staticTexts["Server"], "admin Server section", timeout: 10)
        snap("03-settings-admin")

        // Admin user management: create a user with a password (the
        // /admin/users flow) and see it land in the list.
        scrollTo(app.buttons["Accounts"], "Accounts entry").tap()
        require(app.buttons["Manage server users"], "manage users entry", timeout: 10).tap()
        require(app.buttons["Create user"], "create user button", timeout: 10).tap()
        let username = require(app.textFields["Username"], "username field", timeout: 10)
        username.tap()
        username.typeText("e2e-tester")
        // Reveal the passwords first: SecureField typing is unreliable on
        // simulators (strong-password overlay); plain TextFields aren't.
        require(anyElement("password-eye"), "password eye toggle", timeout: 5).tap()
        let password = require(app.textFields["Password"], "password field", timeout: 5)
        password.tap()
        password.typeText("password123")
        let confirm = app.textFields["Confirm password"]
        require(confirm, "confirm field", timeout: 5).tap()
        confirm.typeText("password123")
        snap("04-create-user-form")
        scrollTo(app.buttons["Create user"], "create submit").tap()
        require(app.staticTexts["e2e-tester"], "created user row", timeout: 15)
        snap("05-user-created")

        // Build the play-context playlist while nothing is playing: create
        // it, then add the first two Latest episodes from their row menus.
        app.tabBars.buttons["Playlists"].tap()
        require(anyElement("playlist-add"), "playlist add button", timeout: 10).tap()
        let name = require(app.textFields["Name"], "playlist name field", timeout: 10)
        focusAndType(name, "E2E List", "playlist name")
        require(app.buttons["Create"], "playlist create submit", timeout: 5).tap()
        require(app.staticTexts["E2E List"], "created playlist row", timeout: 15)
        snap("06-playlist-created")

        app.tabBars.buttons["Latest"].tap()
        for cellIndex in 0..<2 {
            let cell = app.cells.element(boundBy: cellIndex)
            require(cell.buttons["More"], "row menu (cell \(cellIndex))", timeout: 10).tap()
            // Membership-aware submenu (checkmarks; add ↔ remove toggles).
            let addToPlaylist = require(
                app.buttons["Manage Playlists"], "playlists submenu", timeout: 5)
            addToPlaylist.tap()
            // At larger UI sizes the menu overflows and becomes scrollable,
            // and scroll-gesture arbitration can swallow a synthesized tap
            // without any effect — re-tap while the parent menu is still up.
            if !app.buttons["E2E List"].waitForExistence(timeout: 4), addToPlaylist.exists {
                addToPlaylist.tap()
            }
            require(app.buttons["E2E List"], "playlist choice", timeout: 5).tap()
        }
        snap("07-episodes-added")

        // Queue add-to-front regression: row-menu "Add to Queue" must land at
        // the TOP (the queue fetch must be position-ordered; it once came back
        // id-ordered). The seed pre-queues, so scan for a row still offering Add.
        var queuedTitle: String?
        for cellIndex in 0..<4 {
            let cell = app.cells.element(boundBy: cellIndex)
            // The title is a NavigationLink (buttons expose their label;
            // their inner Text stops surfacing as a staticText).
            let title = require(
                cell.buttons.matching(identifier: "row-title").firstMatch,
                "row title (cell \(cellIndex))", timeout: 10
            ).label
            require(cell.buttons["More"], "row menu (queue \(cellIndex))", timeout: 10).tap()
            if app.buttons["Add to Queue"].waitForExistence(timeout: 3) {
                let add = app.buttons["Add to Queue"]
                add.tap()
                // Swallowed-tap guard: a registered tap closes the menu — still
                // open means re-tap. Standalone expectation: `expectation(for:)`
                // would ALSO be waited by a later `waitForExpectations`.
                let closed = XCTNSPredicateExpectation(
                    predicate: NSPredicate(format: "exists == false"), object: add)
                if XCTWaiter().wait(for: [closed], timeout: 4) != .completed, add.exists {
                    add.tap()
                }
                queuedTitle = title
                break
            }
            // Already queued — dismiss the menu (background tap, clear of the
            // top-anchored menu and of the tab bar) and try the next row.
            app.coordinate(withNormalizedOffset: CGVector(dx: 0.08, dy: 0.8)).tap()
        }
        guard let queuedTitle else {
            XCTFail("no Latest row offered Add to Queue")
            return
        }
        app.tabBars.buttons["Queue"].tap()
        let frontTitle = app.cells.element(boundBy: 0)
            .buttons.matching(identifier: "row-title").firstMatch
        let atFront = XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "label == %@", queuedTitle),
            object: frontTitle)
        if XCTWaiter().wait(for: [atFront], timeout: 20) != .completed {
            snap("MISSING-queue front")
            XCTFail("added episode not at queue front (top row: '\(frontTitle.label)')")
        }
        snap("07b-queue-front")
        app.tabBars.buttons["Latest"].tap()

        // Play the first Latest row → the mini player appears (the media is
        // the mock-download fixture; the assertion is the player shell).
        tap(
            app.buttons.matching(identifier: "row-play").firstMatch,
            until: "mini-play", "mini player")
        snap("08-mini-player")

        // Queue and Podcasts render.
        app.tabBars.buttons["Queue"].tap()
        snap("09-queue")
        app.tabBars.buttons["Podcasts"].tap()
        require(app.staticTexts.firstMatch, "podcasts content")
        snap("10-podcasts")

        // Playlist play-context: play from the playlist detail, then assert
        // Up Next follows the PLAYLIST via the player sheet's next.
        app.tabBars.buttons["Playlists"].tap()
        require(app.staticTexts["E2E List"], "playlist row", timeout: 10).tap()
        require(
            app.buttons.matching(identifier: "row-play").firstMatch, "playlist episodes",
            timeout: 15)
        snap("11-playlist-detail")

        tap(
            app.buttons.matching(identifier: "row-play").firstMatch,
            until: "mini-play", "mini player (playlist)")
        require(anyElement("mini-title"), "mini title").tap()
        let sheetTitle = require(anyElement("player-title"), "player sheet", timeout: 10)
        let firstTitle = sheetTitle.label
        require(anyElement("player-next"), "player next transport", timeout: 5).tap()
        // Up Next follows the playlist: the sheet's title flips to the OTHER
        // playlist episode (queue order would be a different pool).
        let switched = NSPredicate(format: "label != %@", firstTitle)
        expectation(for: switched, evaluatedWith: sheetTitle)
        waitForExpectations(timeout: 20)
        snap("12-up-next-playlist")
        // Dismiss the sheet by dragging its grabber down — a mid-screen
        // swipe scrolls the sheet's content instead of moving the sheet.
        let grabber = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.08))
        grabber.press(
            forDuration: 0.05,
            thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.95)))

        // Offline → mutate → online: the outbox round-trip. The navbar user
        // menu carries the manual toggle.
        app.tabBars.buttons["Latest"].tap()
        require(app.buttons["person.circle"], "user menu", timeout: 10).tap()
        require(app.buttons["Go offline"], "go-offline action", timeout: 5).tap()
        snap("13-offline")
        let firstCell = app.cells.element(boundBy: 0)
        require(firstCell.buttons["More"], "row menu offline", timeout: 10).tap()
        // Whichever played-state action shows, it queues an outbox op.
        if app.buttons["Mark played"].waitForExistence(timeout: 3) {
            app.buttons["Mark played"].tap()
        } else {
            require(app.buttons["Mark unplayed"], "mark toggle", timeout: 3).tap()
        }
        snap("14-offline-mutation")
        require(app.buttons["person.circle"], "user menu", timeout: 10).tap()
        require(app.buttons["Go online"], "go-online action", timeout: 5).tap()
        snap("15-online-again")

        // Relaunch WITHOUT reset: the stored session resumes and cached
        // content renders (the local-first boot path).
        relaunchKeepingState()
        requireSessionUp()
        app.tabBars.buttons["Latest"].tap()
        require(
            app.buttons.matching(identifier: "row-play").firstMatch,
            "Latest rows after relaunch", timeout: 20)
        snap("16-relaunch-persisted")
    }
}
