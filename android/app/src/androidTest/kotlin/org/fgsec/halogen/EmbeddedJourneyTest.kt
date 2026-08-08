package org.fgsec.halogen

import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Test
import org.junit.runner.RunWith

/// The embedded-account journey: boot the on-device server from a blank library,
/// subscribe to a fixture feed from the recipe's host server, and watch episodes
/// arrive through the embedded poll. Step-for-step port of `EmbeddedJourneyTests.swift`.
@RunWith(AndroidJUnit4::class)
class EmbeddedJourneyTest : JourneyCase() {

    @Test
    fun embeddedFlagshipJourney() {
        launch(autoconnect = "local")
        // Embedded boot provisions a database + admin on first run.
        requireSessionUp(timeoutMs = 60_000)
        snap("embedded-up")

        // Subscribe by URL to a fixture feed (data/tests/*.xml, served from
        // the host server's public dir — the only outbound dependency).
        require("Podcasts tab") { tag("tab-podcasts") }.performClick()
        require("add-podcast button") { tag("podcast-add") }.performClick()
        type(require("title field", 10_000) { field("Title") }, "Acquired")
        type(
            require("feed url field", 5_000) { field("Feed URL") },
            "$serverBase/feeds/transistor_acquired.xml",
        )
        snap("create-form")
        require("create submit") { tag("podcast-create-submit") }.performClick()

        // The new podcast row appears…
        require("created podcast row", 30_000) { text("Acquired") }
        snap("podcast-created")

        // Episodes only arrive on a feed poll (default interval far beyond the test
        // budget) — trigger one from admin Polling, unhidden via Configure dock first.
        openMoreRoot()
        require("Settings entry", 10_000) { tag("more-settings") }.performClick()
        scrollTo("Configure dock entry") { text("Configure dock") }.performClick()
        scrollTo("polling visibility toggle") { tag("nav-toggle-polling") }.performClick()
        snap("polling-unhidden")
        back() // back to Settings
        back() // back to More
        scrollTo("Polling entry") { tag("more-polling") }.performClick()
        require("poll-now button", 10_000) { desc("Poll now") }.performClick()
        snap("poll-triggered")

        // …then the fixture feed's episodes land in the podcast.
        require("Podcasts tab") { tag("tab-podcasts") }.performClick()
        require("podcast row", 10_000) { text("Acquired") }.performClick()
        // The screen's first fetch can race the poll — refresh until they land.
        refreshUntil("polled episodes", 90_000) { tag("row-play") }
        snap("episodes-polled")

        // They play locally (embedded = stream from the on-device server).
        tapUntil("mini player", reveals = { tag("mini-play") }) { tag("row-play") }
        snap("playing")

        // Multi-user: create a second device user (admin by policy, secret
        // app-managed) — the app creates it server-side and switches the
        // session to it.
        openMoreRoot()
        require("Settings entry", 10_000) { tag("more-settings") }.performClick()
        scrollTo("Accounts entry") { text("Accounts") }.performClick()
        require("add device user", 10_000) { text("New user on this device") }.performClick()
        type(require("username field", 10_000) { field("Username") }, "seconduser")
        require("create-and-switch submit", 5_000) { text("Create and switch") }.performClick()
        requireSessionUp(timeoutMs = 60_000)
        // The OLD tree's dock satisfies requireSessionUp while the switch is
        // in flight; anchor on the remounted shell (fresh user 2 lands on an
        // empty Queue) before navigating, or the tap hits the dying tree.
        require("fresh session (user 2)", 60_000) { text("No queue yet") }
        // Subscriptions are per-user: the fresh user starts with an empty
        // library (the first user's Acquired subscription is theirs alone).
        require("Podcasts tab") { tag("tab-podcasts") }.performClick()
        require("fresh library for user 2", 20_000) { text("No podcasts yet") }
        snap("second-user")
    }
}
