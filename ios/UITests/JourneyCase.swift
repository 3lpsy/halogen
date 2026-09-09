import XCTest

/// Base class for the e2e journeys (`just ios-e2e`): one app instance driven
/// against the hermetic seeded server the recipe spawns on the host. Serial
/// by design — one simulator, one journey class at a time (compute budget).
class JourneyCase: XCTestCase {
    var app: XCUIApplication!

    /// Where the recipe's server listens. Passed by `just ios-e2e` via
    /// `TEST_RUNNER_HALOGEN_E2E_BASE`; the default matches the recipe's port
    /// so tests also run straight from Xcode with the server up.
    static var serverBase: String {
        ProcessInfo.processInfo.environment["HALOGEN_E2E_BASE"]
            ?? "http://127.0.0.1:8099"
    }

    override func setUpWithError() throws {
        continueAfterFailure = false
    }

    /// Launch the app on a wiped device state. `autoconnect` feeds the
    /// DEBUG landing-page hook: "url|user|pass" or "local".
    func launch(autoconnect: String) {
        app = XCUIApplication()
        app.launchEnvironment["HALOGEN_RESET"] = "1"
        app.launchEnvironment["HALOGEN_AUTOCONNECT"] = autoconnect
        if autoconnect == "local" { app.launchEnvironment["HALOGEN_LOCAL_TEST_MODE"] = "1" }
        app.launch()
    }

    /// Relaunch with state KEPT (no reset, no autoconnect): exercises the
    /// stored-session resume + cache-first boot path.
    func relaunchKeepingState() {
        app.terminate()
        app = XCUIApplication()
        app.launch()
    }

    /// Land on the More menu ROOT. Tab stacks persist across tab switches,
    /// so a plain More tap can resurface a previously pushed screen —
    /// re-tapping the selected tab pops its stack to the root.
    func openMoreRoot() {
        require(app.tabBars.buttons["More"], "More tab").tap()
        if !app.buttons["Settings"].waitForExistence(timeout: 3) {
            app.tabBars.buttons["More"].tap()
        }
        require(app.buttons["Settings"], "More menu root", timeout: 5)
    }

    /// Attach a full-screen screenshot named after the journey step —
    /// "capture everything": every step lands in the xcresult bundle even
    /// when the test passes.
    func snap(_ name: String) {
        let shot = XCTAttachment(screenshot: app.screenshot())
        shot.name = name
        shot.lifetime = .keepAlways
        add(shot)
    }

    /// Wait for an element, failing the test (with a screenshot) if it never
    /// appears.
    @discardableResult
    func require(
        _ element: XCUIElement, _ what: String, timeout: TimeInterval = 20
    ) -> XCUIElement {
        if !element.waitForExistence(timeout: timeout) {
            snap("MISSING-\(what)")
            XCTFail("\(what) never appeared (waited \(Int(timeout))s)")
        }
        return element
    }

    /// Physical SwiftUI menus can accept touches while AX reports them unhittable.
    /// Restrict the fallback to a visible row-menu; callers assert its resulting actions.
    func tapMenuTrigger(
        _ menu: XCUIElement, _ what: String, timeout: TimeInterval = 20
    ) {
        let ready = XCTNSPredicateExpectation(
            predicate: NSPredicate { [self] _, _ in
                guard menu.exists, menu.identifier == "row-menu" else { return false }
                let frame = menu.frame
                return frame.origin.x.isFinite && frame.origin.y.isFinite
                    && frame.width.isFinite && frame.height.isFinite
                    && frame.width > 0 && frame.height > 0 && app.frame.contains(frame)
            }, object: menu)
        guard XCTWaiter().wait(for: [ready], timeout: timeout) == .completed else {
            snap("NOT-VISIBLE-\(what)")
            XCTFail("\(what) did not become visible (waited \(Int(timeout))s)")
            return
        }
        if menu.isHittable {
            menu.tap()
        } else if let trigger = menu.buttons.allElementsBoundByIndex.first(where: { $0.isHittable }) {
            trigger.tap()
        } else {
            menu.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
        }
    }

    /// Tap a text field and type, re-tapping until it actually holds keyboard
    /// focus — under load a re-render can steal first responder mid-tap, and a
    /// bare typeText then dies with "no keyboard focus".
    func focusAndType(_ field: XCUIElement, _ text: String, _ what: String) {
        for attempt in 0..<3 {
            field.tap()
            let focused = XCTNSPredicateExpectation(
                predicate: NSPredicate(format: "hasKeyboardFocus == true"), object: field)
            guard XCTWaiter().wait(for: [focused], timeout: 3) == .completed else {
                // Let whatever re-render stole focus settle, then re-tap.
                if attempt < 2 { Thread.sleep(forTimeInterval: 1) }
                continue
            }
            // Clear a prior attempt's partial text (value == placeholder when
            // the field is genuinely empty).
            if let existing = field.value as? String, !existing.isEmpty,
                existing != field.placeholderValue
            {
                field.typeText(
                    String(repeating: XCUIKeyboardKey.delete.rawValue, count: existing.count))
            }
            field.typeText(text)
            // A mid-type re-render can EAT trailing characters (a run created
            // "E2E" from "E2E List") — verify the full text landed.
            if (field.value as? String) == text { return }
        }
        XCTFail("\(what): field never accepted the full text")
    }

    /// Identifier lookup across EVERY element type — SwiftUI surfaces container
    /// identifiers unpredictably; buttons/fields are the reliable anchors.
    func anyElement(_ id: String) -> XCUIElement {
        app.descendants(matching: .any).matching(identifier: id).firstMatch
    }

    /// The signed-in shell is up when the custom dock renders its tabs.
    func requireSessionUp(timeout: TimeInterval = 30) {
        require(app.tabBars.buttons["Latest"], "dock (session shell)", timeout: timeout)
    }

    /// Swipe a lazy list until the element materializes — SwiftUI Lists only
    /// realize on-screen rows, so `waitForExistence` alone can't find rows
    /// below the fold.
    @discardableResult
    func scrollTo(_ element: XCUIElement, _ what: String, maxSwipes: Int = 6) -> XCUIElement {
        for _ in 0..<maxSwipes where !element.exists {
            app.swipeUp()
        }
        return require(element, what, timeout: 5)
    }

    /// Tap `control` until `reveals` appears — a transient overlay (a 4s toast
    /// dismisses on tap) can swallow the first tap. Waits for hittability
    /// first: XCUITest HARD-FAILS a tap on a covered element.
    func tap(_ control: XCUIElement, until reveals: String, _ what: String) {
        for _ in 0..<3 {
            for _ in 0..<20 where !control.isHittable {
                Thread.sleep(forTimeInterval: 0.5)
            }
            guard control.isHittable else { continue }
            control.tap()
            if anyElement(reveals).waitForExistence(timeout: 7) { return }
        }
        snap("MISSING-\(what)")
        XCTFail("\(what) never appeared after tapping")
    }
}
