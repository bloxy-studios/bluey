import XCTest

@testable import BlueyHelperCore

final class AXRoleFilterTests: XCTestCase {
    func testCollectsExactlyTheProtocolRoles() {
        // docs/HELPER_PROTOCOL.md "accessibility.snapshot" element roles.
        let expected: Set<String> = [
            "AXStaticText", "AXTextField", "AXTextArea", "AXButton", "AXLink",
            "AXMenuItem", "AXCheckBox", "AXRadioButton", "AXPopUpButton",
            "AXHeading", "AXCell", "AXWebArea",
        ]
        XCTAssertEqual(AXRoleFilter.collectibleRoles, expected)
        for role in expected {
            XCTAssertTrue(AXRoleFilter.isCollectible(role), role)
        }
    }

    func testRejectsStructuralAndUnknownRoles() {
        for role in ["AXWindow", "AXGroup", "AXScrollArea", "AXSplitGroup", "AXToolbar", "AXImage", "Bogus"] {
            XCTAssertFalse(AXRoleFilter.isCollectible(role), role)
        }
        XCTAssertFalse(AXRoleFilter.isCollectible(nil))
    }

    func testDescendPolicy() {
        XCTAssertTrue(AXRoleFilter.shouldDescend(into: "AXGroup"))
        XCTAssertTrue(AXRoleFilter.shouldDescend(into: "AXWebArea"))
        XCTAssertTrue(AXRoleFilter.shouldDescend(into: nil))
        XCTAssertFalse(AXRoleFilter.shouldDescend(into: "AXMenuBar"))
        XCTAssertFalse(AXRoleFilter.shouldDescend(into: "AXScrollBar"))
    }
}
