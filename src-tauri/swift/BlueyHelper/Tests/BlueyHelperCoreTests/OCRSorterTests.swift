import XCTest

@testable import BlueyHelperCore

final class OCRSorterTests: XCTestCase {
    private func block(_ text: String, x: Double, y: Double, w: Double = 0.1, h: Double = 0.03)
        -> OCRService.Block
    {
        OCRService.Block(
            text: text, confidence: 0.9,
            boundingBox: RectJSON(x: x, y: y, width: w, height: h))
    }

    func testTopToBottomOrdering() {
        let blocks = [
            block("bottom", x: 0.1, y: 0.8),
            block("top", x: 0.1, y: 0.1),
            block("middle", x: 0.1, y: 0.5),
        ]
        let ordered = OCRSorter.sortIntoReadingOrder(blocks)
        XCTAssertEqual(ordered.map(\.text), ["top", "middle", "bottom"])
    }

    func testLeftToRightWithinALine() {
        // Same visual line with slight y jitter (< 60 % of block height).
        let blocks = [
            block("world", x: 0.5, y: 0.102),
            block("hello", x: 0.1, y: 0.100),
            block("!", x: 0.8, y: 0.098),
        ]
        let ordered = OCRSorter.sortIntoReadingOrder(blocks)
        XCTAssertEqual(ordered.map(\.text), ["hello", "world", "!"])
    }

    func testJoinedTextUsesSpacesInLineAndNewlinesBetweenLines() {
        let blocks = [
            block("Sign", x: 0.1, y: 0.1),
            block("in", x: 0.25, y: 0.1),
            block("Forgot password?", x: 0.1, y: 0.3),
        ]
        let ordered = OCRSorter.sortIntoReadingOrder(blocks)
        XCTAssertEqual(OCRSorter.joinedText(ordered), "Sign in\nForgot password?")
    }

    func testTwoColumnsOnSameLineStayLeftToRight() {
        let blocks = [
            block("right column", x: 0.6, y: 0.2),
            block("left column", x: 0.05, y: 0.2),
        ]
        let ordered = OCRSorter.sortIntoReadingOrder(blocks)
        XCTAssertEqual(ordered.map(\.text), ["left column", "right column"])
    }

    func testDistinctLinesNotMerged() {
        // Two blocks with the same x but clearly different y (gap > tolerance).
        let a = block("line1", x: 0.1, y: 0.10, h: 0.03)
        let b = block("line2", x: 0.1, y: 0.16, h: 0.03)
        XCTAssertFalse(OCRSorter.sameLine(a, b))
        XCTAssertEqual(OCRSorter.joinedText([a, b]), "line1\nline2")
    }

    func testEmptyAndSingle() {
        XCTAssertEqual(OCRSorter.sortIntoReadingOrder([]).count, 0)
        XCTAssertEqual(OCRSorter.joinedText([]), "")
        let single = [block("only", x: 0.4, y: 0.4)]
        XCTAssertEqual(OCRSorter.joinedText(OCRSorter.sortIntoReadingOrder(single)), "only")
    }
}
