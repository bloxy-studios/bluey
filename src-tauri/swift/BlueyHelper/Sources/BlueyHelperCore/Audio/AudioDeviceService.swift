import CoreAudio
import Foundation

/// CoreAudio input-device enumeration + default-input change listener.
/// All property access uses the AudioObject API on kAudioObjectSystemObject:
/// https://developer.apple.com/documentation/coreaudio/audioobjectgetpropertydata(_:_:_:_:_:_:)
public final class AudioDeviceService {
    public struct Device: Encodable, Equatable {
        public let id: String // device UID (stable across reboots)
        public let name: String
        public let isDefault: Bool
        public let kind: String // always "input" here
    }

    public struct ListResult: Encodable {
        public let devices: [Device]
    }

    private let listenerQueue = DispatchQueue(label: "com.codewithabdul.bluey.helper.coreaudio")
    private var listenerBlock: ((UInt32, UnsafePointer<AudioObjectPropertyAddress>) -> Void)?
    private var listenerAddresses: [AudioObjectPropertyAddress] = []

    public init() {}

    // MARK: enumeration

    private static func address(
        _ selector: AudioObjectPropertySelector,
        scope: AudioObjectPropertyScope = kAudioObjectPropertyScopeGlobal
    ) -> AudioObjectPropertyAddress {
        AudioObjectPropertyAddress(
            mSelector: selector,
            mScope: scope,
            // kAudioObjectPropertyElementMain (macOS 12+ rename of ...ElementMaster)
            mElement: kAudioObjectPropertyElementMain)
    }

    private static func allDeviceIDs() -> [AudioDeviceID] {
        var addr = address(kAudioHardwarePropertyDevices)
        var size: UInt32 = 0
        guard
            AudioObjectGetPropertyDataSize(
                AudioObjectID(kAudioObjectSystemObject), &addr, 0, nil, &size) == noErr,
            size > 0
        else { return [] }
        let count = Int(size) / MemoryLayout<AudioDeviceID>.size
        var ids = [AudioDeviceID](repeating: 0, count: count)
        guard
            AudioObjectGetPropertyData(
                AudioObjectID(kAudioObjectSystemObject), &addr, 0, nil, &size, &ids) == noErr
        else { return [] }
        return ids
    }

    /// Input channel count via kAudioDevicePropertyStreamConfiguration (input scope).
    private static func inputChannelCount(_ device: AudioDeviceID) -> Int {
        var addr = address(kAudioDevicePropertyStreamConfiguration, scope: kAudioObjectPropertyScopeInput)
        var size: UInt32 = 0
        guard AudioObjectGetPropertyDataSize(device, &addr, 0, nil, &size) == noErr, size > 0 else {
            return 0
        }
        let raw = UnsafeMutableRawPointer.allocate(
            byteCount: Int(size), alignment: MemoryLayout<AudioBufferList>.alignment)
        defer { raw.deallocate() }
        let listPtr = raw.assumingMemoryBound(to: AudioBufferList.self)
        guard AudioObjectGetPropertyData(device, &addr, 0, nil, &size, listPtr) == noErr else {
            return 0
        }
        let buffers = UnsafeMutableAudioBufferListPointer(listPtr)
        return buffers.reduce(0) { $0 + Int($1.mNumberChannels) }
    }

    private static func stringProperty(
        _ device: AudioDeviceID, _ selector: AudioObjectPropertySelector
    ) -> String? {
        var addr = address(selector)
        var size = UInt32(MemoryLayout<CFString?>.size)
        var value: CFString?
        let status = withUnsafeMutablePointer(to: &value) { ptr in
            AudioObjectGetPropertyData(device, &addr, 0, nil, &size, ptr)
        }
        guard status == noErr, let value else { return nil }
        return value as String
    }

    public static func defaultInputDeviceID() -> AudioDeviceID? {
        var addr = address(kAudioHardwarePropertyDefaultInputDevice)
        var device = AudioDeviceID(0)
        var size = UInt32(MemoryLayout<AudioDeviceID>.size)
        guard
            AudioObjectGetPropertyData(
                AudioObjectID(kAudioObjectSystemObject), &addr, 0, nil, &size, &device) == noErr,
            device != kAudioObjectUnknown
        else { return nil }
        return device
    }

    public func listInputDevices() -> [Device] {
        let defaultID = Self.defaultInputDeviceID()
        var devices: [Device] = []
        for id in Self.allDeviceIDs() where Self.inputChannelCount(id) > 0 {
            guard let uid = Self.stringProperty(id, kAudioDevicePropertyDeviceUID) else { continue }
            let name =
                Self.stringProperty(id, kAudioObjectPropertyName)
                ?? Self.stringProperty(id, kAudioDevicePropertyDeviceNameCFString)
                ?? "Unknown input"
            devices.append(
                Device(id: uid, name: name, isDefault: id == defaultID, kind: "input"))
        }
        return devices
    }

    public func defaultInputDevice() -> Device? {
        listInputDevices().first { $0.isDefault }
    }

    public func deviceID(forUID uid: String) -> AudioDeviceID? {
        for id in Self.allDeviceIDs() where Self.inputChannelCount(id) > 0 {
            if Self.stringProperty(id, kAudioDevicePropertyDeviceUID) == uid {
                return id
            }
        }
        return nil
    }

    // MARK: change listener

    /// Fires on default-input change and on device list changes.
    /// https://developer.apple.com/documentation/coreaudio/audioobjectaddpropertylistenerblock(_:_:_:_:)
    public func startListening(onChange: @escaping () -> Void) {
        guard listenerBlock == nil else { return }
        let block: (UInt32, UnsafePointer<AudioObjectPropertyAddress>) -> Void = { _, _ in
            onChange()
        }
        listenerBlock = block
        listenerAddresses = [
            Self.address(kAudioHardwarePropertyDefaultInputDevice),
            Self.address(kAudioHardwarePropertyDevices),
        ]
        for i in listenerAddresses.indices {
            let status = AudioObjectAddPropertyListenerBlock(
                AudioObjectID(kAudioObjectSystemObject), &listenerAddresses[i], listenerQueue, block)
            if status != noErr {
                Log.shared.warn("AudioObjectAddPropertyListenerBlock failed: \(status)")
            }
        }
    }

    public func stopListening() {
        guard let block = listenerBlock else { return }
        for i in listenerAddresses.indices {
            AudioObjectRemovePropertyListenerBlock(
                AudioObjectID(kAudioObjectSystemObject), &listenerAddresses[i], listenerQueue, block)
        }
        listenerBlock = nil
        listenerAddresses = []
    }
}
