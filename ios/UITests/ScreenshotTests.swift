import XCTest

/// Captures the maintained podcast journey using the same assertions as the functional lane.
final class ScreenshotTests: JourneyCase {
    func testScreenWalkthrough() { runRemoteJourney() }
}
