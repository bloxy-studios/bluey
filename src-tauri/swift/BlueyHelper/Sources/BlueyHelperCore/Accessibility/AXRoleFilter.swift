import Foundation

/// The traversal only collects text-bearing / interactive roles listed in
/// docs/HELPER_PROTOCOL.md ("accessibility.snapshot"). Pure Swift → unit-tested.
public enum AXRoleFilter {
    public static let collectibleRoles: Set<String> = [
        "AXStaticText",
        "AXTextField",
        "AXTextArea",
        "AXButton",
        "AXLink",
        "AXMenuItem",
        "AXCheckBox",
        "AXRadioButton",
        "AXPopUpButton",
        "AXHeading",
        "AXCell",
        "AXWebArea",
    ]

    /// Containers we never descend into (huge, useless subtrees).
    public static let skippedSubtrees: Set<String> = [
        "AXMenuBar",
        "AXScrollBar",
    ]

    public static func isCollectible(_ role: String?) -> Bool {
        guard let role else { return false }
        return collectibleRoles.contains(role)
    }

    public static func shouldDescend(into role: String?) -> Bool {
        guard let role else { return true }
        return !skippedSubtrees.contains(role)
    }
}
