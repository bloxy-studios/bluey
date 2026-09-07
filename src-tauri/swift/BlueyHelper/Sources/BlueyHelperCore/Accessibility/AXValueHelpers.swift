import ApplicationServices
import Foundation

/// Small typed wrappers over the C AXUIElement API.
/// Attribute constants (kAX…Attribute) are imported into Swift as `String`;
/// AXUIElementCopyAttributeValue takes CFString, hence the `as CFString` casts.
/// https://developer.apple.com/documentation/applicationservices/axuielement_h
public enum AX {
    public static func copyValue(_ element: AXUIElement, _ attribute: String) -> CFTypeRef? {
        var value: CFTypeRef?
        let err = AXUIElementCopyAttributeValue(element, attribute as CFString, &value)
        guard err == .success else { return nil }
        return value
    }

    public static func string(_ element: AXUIElement, _ attribute: String) -> String? {
        guard let value = copyValue(element, attribute) else { return nil }
        if let s = value as? String { return s }
        if CFGetTypeID(value) == CFAttributedStringGetTypeID() {
            return (value as! NSAttributedString).string
        }
        if let url = value as? URL { return url.absoluteString }
        if let n = value as? NSNumber { return n.stringValue }
        return nil
    }

    public static func element(_ element: AXUIElement, _ attribute: String) -> AXUIElement? {
        guard let value = copyValue(element, attribute) else { return nil }
        guard CFGetTypeID(value) == AXUIElementGetTypeID() else { return nil }
        return (value as! AXUIElement)
    }

    public static func elementArray(_ element: AXUIElement, _ attribute: String) -> [AXUIElement] {
        guard let value = copyValue(element, attribute) else { return [] }
        guard let array = value as? [AnyObject] else { return [] }
        return array.compactMap { item in
            guard CFGetTypeID(item) == AXUIElementGetTypeID() else { return nil }
            return (item as! AXUIElement)
        }
    }

    /// Unpack AXValue structs (CGPoint / CGSize) via AXValueGetValue:
    /// https://developer.apple.com/documentation/applicationservices/1462011-axvaluegetvalue
    public static func point(_ element: AXUIElement, _ attribute: String) -> CGPoint? {
        guard let value = copyValue(element, attribute),
            CFGetTypeID(value) == AXValueGetTypeID()
        else { return nil }
        let axValue = value as! AXValue
        var pt = CGPoint.zero
        guard AXValueGetType(axValue) == .cgPoint, AXValueGetValue(axValue, .cgPoint, &pt) else {
            return nil
        }
        return pt
    }

    public static func size(_ element: AXUIElement, _ attribute: String) -> CGSize? {
        guard let value = copyValue(element, attribute),
            CFGetTypeID(value) == AXValueGetTypeID()
        else { return nil }
        let axValue = value as! AXValue
        var sz = CGSize.zero
        guard AXValueGetType(axValue) == .cgSize, AXValueGetValue(axValue, .cgSize, &sz) else {
            return nil
        }
        return sz
    }

    /// https://developer.apple.com/documentation/applicationservices/1459617-axuielementcopyactionnames
    public static func actions(_ element: AXUIElement) -> [String] {
        var names: CFArray?
        guard AXUIElementCopyActionNames(element, &names) == .success, let names else { return [] }
        return (names as? [String]) ?? []
    }

    public static func role(_ element: AXUIElement) -> String? {
        string(element, kAXRoleAttribute)
    }

    /// Best-effort human label: title → description → placeholder.
    public static func label(_ element: AXUIElement) -> String? {
        if let t = string(element, kAXTitleAttribute), !t.isEmpty { return t }
        if let d = string(element, kAXDescriptionAttribute), !d.isEmpty { return d }
        if let p = string(element, kAXPlaceholderValueAttribute), !p.isEmpty { return p }
        return nil
    }

    /// Stringified kAXValueAttribute (numbers/bools included), untruncated.
    public static func valueString(_ element: AXUIElement) -> String? {
        guard let value = copyValue(element, kAXValueAttribute) else { return nil }
        if let s = value as? String { return s }
        if CFGetTypeID(value) == CFAttributedStringGetTypeID() {
            return (value as! NSAttributedString).string
        }
        if let n = value as? NSNumber { return n.stringValue }
        if let url = value as? URL { return url.absoluteString }
        return nil
    }

    public static func children(_ element: AXUIElement) -> [AXUIElement] {
        elementArray(element, kAXChildrenAttribute)
    }
}
