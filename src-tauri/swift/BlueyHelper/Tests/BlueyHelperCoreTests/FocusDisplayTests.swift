import CoreGraphics
import XCTest

@testable import BlueyHelperCore

/// CTX-013: "Display with focus" must follow the user's window, not the
/// menu-bar display.
final class FocusDisplayTests: XCTestCase {
    // Main display at the origin, a secondary display to its right.
    private let displays: [(id: UInt32, frame: CGRect)] = [
        (id: 1, frame: CGRect(x: 0, y: 0, width: 1440, height: 900)),
        (id: 2, frame: CGRect(x: 1440, y: 0, width: 1920, height: 1080)),
    ]

    func testPicksTheDisplayContainingTheFocusedWindow() {
        let window = CGRect(x: 1600, y: 100, width: 800, height: 600)
        let picked = FocusDisplay.resolve(
            displays: displays, focusedWindow: window,
            mouse: CGPoint(x: 10, y: 10), mainDisplayID: 1)
        XCTAssertEqual(picked, 2)
    }

    func testUsesTheWindowMidpointWhenItStraddlesDisplays() {
        // Mostly on display 1: the midpoint (x = 1340) is on display 1.
        let window = CGRect(x: 940, y: 100, width: 800, height: 600)
        let picked = FocusDisplay.resolve(
            displays: displays, focusedWindow: window, mouse: nil, mainDisplayID: 2)
        XCTAssertEqual(picked, 1)
    }

    func testFallsBackToTheDisplayUnderTheMouse() {
        let picked = FocusDisplay.resolve(
            displays: displays, focusedWindow: nil,
            mouse: CGPoint(x: 2000, y: 500), mainDisplayID: 1)
        XCTAssertEqual(picked, 2)
    }

    func testFallsBackToTheMainDisplayThenTheFirst() {
        let offscreen = CGPoint(x: -5000, y: -5000)
        XCTAssertEqual(
            FocusDisplay.resolve(
                displays: displays, focusedWindow: nil, mouse: offscreen, mainDisplayID: 2),
            2)
        XCTAssertEqual(
            FocusDisplay.resolve(
                displays: displays, focusedWindow: nil, mouse: nil, mainDisplayID: 99),
            1)
        XCTAssertNil(
            FocusDisplay.resolve(displays: [], focusedWindow: nil, mouse: nil, mainDisplayID: 1))
    }
}
