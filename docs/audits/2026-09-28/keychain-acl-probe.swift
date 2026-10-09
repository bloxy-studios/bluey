// Keychain ACL probe — mirrors keyring 3.6.3 apple-native (legacy SecKeychain* generic passwords).
// User interaction is DISABLED: an operation that would show a dialog returns
// errSecInteractionNotAllowed (-25308) instead. Never prints secret data.
import Foundation
import Security

let BUILD_TAG = "BUILD_TAG_PLACEHOLDER"
let args = CommandLine.arguments
guard args.count >= 3 else { print("usage: probe <add|read|attrs|attrs-secitem|modify|delete> <service>"); exit(2) }
let op = args[1], service = args[2], account = "probe:item"
SecKeychainSetUserInteractionAllowed(false)

func find(_ wantData: Bool) -> (OSStatus, SecKeychainItem?) {
    var item: SecKeychainItem?
    var len: UInt32 = 0
    var data: UnsafeMutableRawPointer?
    let st = service.withCString { s in account.withCString { a in
        wantData
          ? SecKeychainFindGenericPassword(nil, UInt32(strlen(s)), s, UInt32(strlen(a)), a, &len, &data, &item)
          : SecKeychainFindGenericPassword(nil, UInt32(strlen(s)), s, UInt32(strlen(a)), a, nil, nil, &item)
    } }
    if let d = data { SecKeychainItemFreeContent(nil, d) }
    return (st, item)
}
var status: OSStatus = 0
switch op {
case "add":
    let pw = "probe-value-\(BUILD_TAG)"
    status = service.withCString { s in account.withCString { a in pw.withCString { p in
        SecKeychainAddGenericPassword(nil, UInt32(strlen(s)), s, UInt32(strlen(a)), a, UInt32(strlen(p)), p, nil)
    } } }
case "read": status = find(true).0
case "attrs": status = find(false).0
case "attrs-secitem":
    let q: [String: Any] = [kSecClass as String: kSecClassGenericPassword, kSecAttrService as String: service,
        kSecAttrAccount as String: account, kSecReturnAttributes as String: true, kSecMatchLimit as String: kSecMatchLimitOne]
    var out: CFTypeRef?
    status = SecItemCopyMatching(q as CFDictionary, &out)
case "modify":
    let (st, item) = find(false)
    status = st
    if st == errSecSuccess, let item = item {
        let pw = "probe-value-modified-\(BUILD_TAG)"
        status = pw.withCString { p in SecKeychainItemModifyAttributesAndData(item, nil, UInt32(strlen(p)), p) }
    }
case "delete":
    let (st, item) = find(false)
    status = st
    if st == errSecSuccess, let item = item { status = SecKeychainItemDelete(item) }
default: print("unknown op"); exit(2)
}
let msg = (SecCopyErrorMessageString(status, nil) as String?) ?? ""
print("build=\(BUILD_TAG) op=\(op) service=\(service) status=\(status) (\(msg))")
