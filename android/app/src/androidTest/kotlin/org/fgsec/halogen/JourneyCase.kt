package org.fgsec.halogen

import android.Manifest
import android.content.Intent
import android.os.SystemClock
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.SemanticsNodeInteraction
import androidx.compose.ui.test.hasSetTextAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createEmptyComposeRule
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performTextInput
import androidx.test.core.app.ActivityScenario
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.rule.GrantPermissionRule
import androidx.test.uiautomator.UiDevice
import org.fgsec.halogen.core.DeviceLog
import org.junit.After
import org.junit.Rule
import java.io.File

/// Base for the e2e journeys (`just android-e2e`): one app instance driven
/// against the seeded server the recipe spawns. Serial by design — one
/// emulator, one journey class at a time. The Swift `JourneyCase` is the spec.
abstract class JourneyCase {
    // Empty rule: the journeys launch (and relaunch) the activity themselves,
    // so no rule may own its lifecycle. Compose sync still applies.
    @get:Rule
    val compose = createEmptyComposeRule()

    /// `pm clear` between runs revokes POST_NOTIFICATIONS and the re-prompt dialog
    /// blocks RESUMED, killing every journey at `requireSessionUp` — pre-grant to skip it.
    @get:Rule
    val notifications: GrantPermissionRule =
        GrantPermissionRule.grant(Manifest.permission.POST_NOTIFICATIONS)

    private val instrumentation get() = InstrumentationRegistry.getInstrumentation()
    private val device: UiDevice by lazy { UiDevice.getInstance(instrumentation) }
    private var scenario: ActivityScenario<MainActivity>? = null
    private var stepIndex = 0

    /// The device log rides the journey artifacts: the instrumentation shares
    /// the app process, so the ring is readable directly. Pass or fail — the
    /// on-device view of a failure is otherwise unpullable.
    @After
    fun dumpDeviceLog() {
        runCatching {
            val dir = File(
                instrumentation.targetContext.getExternalFilesDir(null), "journey"
            ).apply { mkdirs() }
            File(dir, "99-device-log.txt").writeText(DeviceLog.shared.exportText())
        }
    }

    /// Where the recipe's seeded server listens, passed as an instrumentation
    /// arg (`-e HALOGEN_E2E_BASE …`). The default matches the recipe's port.
    protected val serverBase: String
        get() = InstrumentationRegistry.getArguments().getString("HALOGEN_E2E_BASE")
            ?: "http://10.0.2.2:8099"

    /// Launch on wiped state. `autoconnect` feeds the DEBUG landing hook:
    /// "url|user|pass" or "local". Hooks ride intent extras (DebugHooks).
    protected fun launch(autoconnect: String) {
        scenario?.close()
        scenario = ActivityScenario.launch<MainActivity>(
            Intent(instrumentation.targetContext, MainActivity::class.java)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                .putExtra("HALOGEN_RESET", "1")
                .putExtra("HALOGEN_AUTOCONNECT", autoconnect)
        )
        compose.waitForIdle()
    }

    /// Relaunch with state KEPT (no reset, no autoconnect): exercises the
    /// stored-session resume + cache-first boot path.
    protected fun relaunchKeepingState() {
        scenario?.close()
        scenario = ActivityScenario.launch<MainActivity>(
            Intent(instrumentation.targetContext, MainActivity::class.java)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        )
        compose.waitForIdle()
    }

    /// Screenshot every step (pass or fail) into the app's external files dir,
    /// where the runner pulls journey artifacts from. `index` overrides the
    /// auto-number for the odd interposed step (iOS's "07b-queue-front") so
    /// the two journeys' screenshot filenames stay 1:1 comparable.
    protected fun snap(name: String, index: String? = null) {
        // Settle first: a snap right after a click otherwise captures the
        // PREVIOUS frame (every shot came out one step behind).
        compose.waitForIdle()
        val dir = File(
            instrumentation.targetContext.getExternalFilesDir(null), "journey"
        ).apply { mkdirs() }
        // Clear on the first snap: a previous journey's shots outnumber and
        // outrank this one's in the pull directory, and "05-MISSING-…" from
        // the wrong run is worse than no artifact at all.
        if (stepIndex == 0) dir.listFiles()?.forEach { it.delete() }
        val prefix = index ?: "%02d".format(++stepIndex)
        device.takeScreenshot(File(dir, "$prefix-$name.png"))
    }

    /// Wait for `matcher` to match, failing with a screenshot if it never does.
    protected fun require(
        what: String,
        timeoutMs: Long = 20_000,
        finder: () -> SemanticsNodeInteraction,
    ): SemanticsNodeInteraction {
        try {
            compose.waitUntil(timeoutMs) { exists(finder) }
        } catch (e: Throwable) {
            snap("MISSING-$what")
            throw AssertionError("$what never appeared (waited ${timeoutMs / 1000}s)", e)
        }
        return finder()
    }

    private fun exists(finder: () -> SemanticsNodeInteraction): Boolean =
        runCatching { finder().fetchSemanticsNode(); true }.getOrDefault(false)

    // ── Finders (first match, mirroring XCUITest's firstMatch semantics) ──

    /// Unmerged tree: a tagged node that is NOT itself clickable gets merged
    /// into a clickable ancestor, and its testTag disappears from the merged
    /// tree (`mini-title` inside the mini player's clickable bar). The tag set
    /// is otherwise identical, and a click still lands on the ancestor.
    protected fun tag(tag: String, index: Int = 0) =
        compose.onAllNodesWithTag(tag, useUnmergedTree = true)[index]

    protected fun text(value: String, substring: Boolean = false, index: Int = 0) =
        compose.onAllNodesWithText(value, substring = substring)[index]

    protected fun desc(value: String, index: Int = 0) =
        compose.onAllNodesWithContentDescription(value)[index]

    /// A text field by its label — labels merge into the field's semantics,
    /// which is how the Swift journeys address fields too (no identifiers).
    protected fun field(label: String, index: Int = 0) =
        compose.onAllNodes(hasSetTextAction() and hasText(label, substring = true))[index]

    /// The signed-in shell is up when the dock renders its tabs.
    protected fun requireSessionUp(timeoutMs: Long = 30_000) {
        require("dock (session shell)", timeoutMs) { tag("tab-latest") }
    }

    /// Land on the More menu ROOT: tab stacks persist across tab switches, so
    /// a second tap on the selected tab pops the stack (RootView parity).
    protected fun openMoreRoot() {
        require("More tab") { tag("tab-more") }.performClick()
        if (!exists { tag("more-settings") }) tag("tab-more").performClick()
        require("More menu root", 5_000) { tag("more-settings") }
    }

    /// Scroll a lazy list until the node materializes — LazyColumn only
    /// composes on-screen rows, so waiting can't find rows below the fold and
    /// `performScrollTo` can't either (it needs the node to exist first).
    /// Swipe between attempts, like the Swift `scrollTo` does.
    protected fun scrollTo(
        what: String,
        timeoutMs: Long = 20_000,
        finder: () -> SemanticsNodeInteraction,
    ): SemanticsNodeInteraction {
        val deadline = SystemClock.uptimeMillis() + timeoutMs
        while (!exists(finder)) {
            if (SystemClock.uptimeMillis() >= deadline) {
                snap("MISSING-$what")
                throw AssertionError("$what never appeared (waited ${timeoutMs / 1000}s, scrolling)")
            }
            swipeUp()
        }
        val node = finder()
        // Composed but clipped (a non-lazy scrollable): settle it into view.
        runCatching { node.performScrollTo() }
        return node
    }

    /// Count of nodes currently matching a tag (composed rows only).
    protected fun tagCount(tag: String): Int =
        compose.onAllNodesWithTag(tag, useUnmergedTree = true).fetchSemanticsNodes().size

    /// One list-sized swipe up, then let Compose settle.
    protected fun swipeUp() {
        val x = device.displayWidth / 2
        device.swipe(x, device.displayHeight * 7 / 10, x, device.displayHeight * 3 / 10, 12)
        compose.waitForIdle()
    }

    /// Pull-to-refresh until `finder` matches: server-side state that lands
    /// AFTER the screen's first fetch (the embedded poll) needs a refetch the
    /// app only does on user refresh — exactly what a person would do.
    protected fun refreshUntil(
        what: String,
        timeoutMs: Long = 90_000,
        finder: () -> SemanticsNodeInteraction,
    ): SemanticsNodeInteraction {
        val deadline = SystemClock.uptimeMillis() + timeoutMs
        val x = device.displayWidth / 2
        while (!exists(finder)) {
            if (SystemClock.uptimeMillis() >= deadline) {
                snap("MISSING-$what")
                throw AssertionError("$what never appeared (waited ${timeoutMs / 1000}s, refreshing)")
            }
            device.swipe(x, device.displayHeight * 4 / 10, x, device.displayHeight * 8 / 10, 24)
            compose.waitForIdle()
            SystemClock.sleep(2_000)
        }
        return finder()
    }

    /// Tap `control` until `reveals` shows: a transient toast can swallow the
    /// first tap (it dismisses on tap) without failing anything.
    protected fun tapUntil(
        what: String,
        reveals: () -> SemanticsNodeInteraction,
        control: () -> SemanticsNodeInteraction,
    ) {
        repeat(3) {
            runCatching { control().performClick() }
            if (runCatching {
                    compose.waitUntil(7_000) { exists(reveals) }
                }.isSuccess
            ) return
        }
        snap("MISSING-$what")
        throw AssertionError("$what never appeared after tapping")
    }

    /// Read a node's visible text (row titles drive the queue-order assertion).
    protected fun labelOf(node: SemanticsNodeInteraction): String =
        node.fetchSemanticsNode().config.getOrNull(SemanticsProperties.Text)
            ?.joinToString(" ") { it.text }.orEmpty()

    /// Type into a field after clearing whatever a prior attempt left.
    protected fun type(node: SemanticsNodeInteraction, value: String) {
        node.performClick()
        node.performTextInput(value)
    }

    /// System back — Android's journeys pop with the hardware gesture where
    /// the Swift ones tap the navigation bar's back button.
    protected fun back() {
        device.pressBack()
        compose.waitForIdle()
    }
}
