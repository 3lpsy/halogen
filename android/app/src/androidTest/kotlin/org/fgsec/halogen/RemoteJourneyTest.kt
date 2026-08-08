package org.fgsec.halogen

import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertEquals
import org.junit.Test
import org.junit.runner.RunWith

/// The remote-account flagship journey against the recipe's seeded server — port
/// of `RemoteJourneyTests.swift`. Ordering rule: every FORM flow runs before
/// anything plays — the mini player floats over form bottoms and steals submit taps.
@RunWith(AndroidJUnit4::class)
class RemoteJourneyTest : JourneyCase() {

    @Test
    fun remoteFlagshipJourney() {
        launch(autoconnect = "$serverBase|dev|dev")
        requireSessionUp()
        snap("signed-in")

        // Latest: the seeded library renders rows (anchored on each row's play
        // button). Generous first-content timeout: on a loaded emulator the
        // first fetch+render can exceed the interactive-feeling default.
        require("Latest tab") { tag("tab-latest") }.performClick()
        require("Latest episode rows", 60_000) { tag("row-play") }
        snap("latest-rows")

        // Admin surfaces exist for an admin account.
        openMoreRoot()
        require("Settings entry", 10_000) { tag("more-settings") }.performClick()
        require("admin Server section", 10_000) { text("Server") }
        snap("settings-admin")

        // Admin user management: create a user with a password (the
        // /admin/users flow) and see it land in the list.
        scrollTo("Accounts entry") { text("Accounts") }.performClick()
        require("manage users entry", 10_000) { text("Manage server users") }.performClick()
        // Icon button: the label rides contentDescription, not text (the
        // form's own title and submit are the two "Create user" texts).
        require("create user button", 10_000) { desc("Create user") }.performClick()
        type(require("username field", 10_000) { field("Username") }, "e2e-tester")
        // Reveal the passwords first: the fields are masked until toggled, and
        // a masked field's semantics carry no label to anchor on.
        require("password eye toggle", 5_000) { tag("password-eye") }.performClick()
        type(require("password field", 5_000) { field("Password") }, "password123")
        type(require("confirm field", 5_000) { field("Confirm password") }, "password123")
        snap("create-user-form")
        scrollTo("create submit") { text("Create user", index = 1) }.performClick()
        require("created user row", 15_000) { text("e2e-tester") }
        snap("user-created")

        // Build the play-context playlist while nothing is playing: create it,
        // then add the first two Latest episodes from their row menus.
        require("Playlists tab") { tag("tab-playlists") }.performClick()
        require("playlist add button", 10_000) { tag("playlist-add") }.performClick()
        type(require("playlist name field", 10_000) { field("Name") }, "E2E List")
        require("playlist create submit", 5_000) { text("Create") }.performClick()
        require("created playlist row", 15_000) { text("E2E List") }
        snap("playlist-created")

        require("Latest tab") { tag("tab-latest") }.performClick()
        for (row in 0 until 2) {
            require("row menu (row $row)", 10_000) { desc("Episode menu", index = row) }
                .performClick()
            // Membership-aware submenu (checkmarks; add ↔ remove toggles).
            require("playlists submenu", 5_000) { text("Manage Playlists") }.performClick()
            require("playlist choice", 5_000) { text("E2E List") }.performClick()
        }
        snap("episodes-added")

        // Add-to-front regression: row-menu "Add to Queue" must land at the TOP of
        // the queue. Scan only COMPOSED LazyColumn rows (swiping more into view);
        // the menu-open check keeps back() balanced — an unpaired back exits the app.
        var queuedTitle: String? = null
        outer@ for (sweep in 0 until 5) {
            for (row in 0 until tagCount("row-title")) {
                val title = labelOf(tag("row-title", index = row))
                // A row can compose with its action line below the fold —
                // settle the button into view or the tap lands offscreen.
                runCatching { desc("Episode menu", index = row).performScrollTo() }
                // tapUntil: a lingering add-toast can swallow the first tap.
                tapUntil("row menu ($title)", reveals = { text("Manage Playlists") }) {
                    desc("Episode menu", index = row)
                }
                val addable = runCatching {
                    compose.waitUntil(2_000) {
                        runCatching { text("Add to Queue").fetchSemanticsNode(); true }
                            .getOrDefault(false)
                    }
                }.isSuccess
                if (addable) {
                    text("Add to Queue").performClick()
                    queuedTitle = title
                    break@outer
                }
                back() // already queued — dismiss the menu and try the next row
            }
            swipeUp()
        }
        val queued = requireNotNull(queuedTitle) { "no Latest row offered Add to Queue" }
        require("Queue tab") { tag("tab-queue") }.performClick()
        require("queue front row", 20_000) { tag("row-title") }
        compose.waitUntil(20_000) { labelOf(tag("row-title")) == queued }
        assertEquals("added episode not at queue front", queued, labelOf(tag("row-title")))
        snap("queue-front", index = "07b")
        require("Latest tab") { tag("tab-latest") }.performClick()

        // Play the first Latest row → the mini player appears (the media is
        // the mock-download fixture; the assertion is the player shell).
        tapUntil("mini player", reveals = { tag("mini-play") }) { tag("row-play") }
        snap("mini-player")

        // Queue and Podcasts render.
        require("Queue tab") { tag("tab-queue") }.performClick()
        snap("queue")
        require("Podcasts tab") { tag("tab-podcasts") }.performClick()
        // Content anchor (iOS asserts podcasts content, not just the tab);
        // "Acquired" is deterministic in the seeded library.
        require("podcasts content", 15_000) { text("Acquired") }
        snap("podcasts")

        // Playlist play-context: play from the playlist detail, then assert Up
        // Next follows the PLAYLIST via the player sheet's next.
        require("Playlists tab") { tag("tab-playlists") }.performClick()
        require("playlist row", 10_000) { text("E2E List") }.performClick()
        require("playlist episodes", 15_000) { tag("row-play") }
        snap("playlist-detail")

        tapUntil("mini player (playlist)", reveals = { tag("mini-play") }) { tag("row-play") }
        require("mini title") { tag("mini-title") }.performClick()
        val firstTitle = labelOf(require("player sheet", 10_000) { tag("player-title") })
        require("player next transport", 5_000) { tag("player-next") }.performClick()
        // Up Next follows the playlist: the sheet's title flips to the OTHER
        // playlist episode (queue order would be a different pool).
        compose.waitUntil(20_000) { labelOf(tag("player-title")) != firstTitle }
        snap("up-next-playlist")
        back() // dismiss the player sheet

        // Offline → mutate → online: the outbox round-trip. The navbar account
        // menu carries the manual toggle.
        require("Latest tab") { tag("tab-latest") }.performClick()
        require("account menu", 10_000) { desc("Account menu") }.performClick()
        require("go-offline action", 5_000) { text("Go offline") }.performClick()
        snap("offline")
        require("row menu offline", 10_000) { desc("Episode menu") }.performClick()
        // Whichever played-state action shows, it queues an outbox op.
        val markPlayed = runCatching {
            compose.waitUntil(3_000) {
                runCatching { text("Mark played").fetchSemanticsNode(); true }
                    .getOrDefault(false)
            }
        }.isSuccess
        if (markPlayed) text("Mark played").performClick()
        else require("mark toggle", 3_000) { text("Mark unplayed") }.performClick()
        snap("offline-mutation")
        require("account menu", 10_000) { desc("Account menu") }.performClick()
        require("go-online action", 5_000) { text("Go online") }.performClick()
        snap("online-again")

        // Relaunch WITHOUT reset: the stored session resumes and cached content
        // renders (the local-first boot path).
        relaunchKeepingState()
        requireSessionUp()
        require("Latest tab") { tag("tab-latest") }.performClick()
        require("Latest rows after relaunch", 20_000) { tag("row-play") }
        snap("relaunch-persisted")
    }
}
