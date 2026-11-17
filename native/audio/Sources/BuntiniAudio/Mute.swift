// Mutes the Mac's sound while you dictate: every output that can be muted and isn't already,
// except aggregate devices (made of other devices, which are muted on their own). Only what
// this muted is unmuted again, and Rust keeps the list so a crash is undone at next launch.
import CoreAudio
import Foundation

enum OutputMute {
    private static func address(_ selector: AudioObjectPropertySelector, _ scope: AudioObjectPropertyScope = kAudioObjectPropertyScopeGlobal) -> AudioObjectPropertyAddress {
        AudioObjectPropertyAddress(mSelector: selector, mScope: scope, mElement: kAudioObjectPropertyElementMain)
    }

    static func devices() -> [AudioObjectID] {
        var where_ = address(kAudioHardwarePropertyDevices)
        var size: UInt32 = 0
        guard AudioObjectGetPropertyDataSize(AudioObjectID(kAudioObjectSystemObject), &where_, 0, nil, &size) == noErr else { return [] }
        var ids = [AudioObjectID](repeating: 0, count: Int(size) / MemoryLayout<AudioObjectID>.size)
        guard AudioObjectGetPropertyData(AudioObjectID(kAudioObjectSystemObject), &where_, 0, nil, &size, &ids) == noErr else { return [] }
        return ids
    }

    static func hasOutput(_ device: AudioObjectID) -> Bool {
        var where_ = address(kAudioDevicePropertyStreams, kAudioObjectPropertyScopeOutput)
        var size: UInt32 = 0
        return AudioObjectGetPropertyDataSize(device, &where_, 0, nil, &size) == noErr && size > 0
    }

    static func aggregate(_ device: AudioObjectID) -> Bool {
        var where_ = address(kAudioDevicePropertyTransportType)
        var transport: UInt32 = 0
        var size = UInt32(MemoryLayout<UInt32>.size)
        guard AudioObjectGetPropertyData(device, &where_, 0, nil, &size, &transport) == noErr else { return false }
        return transport == kAudioDeviceTransportTypeAggregate
    }

    static func uid(_ device: AudioObjectID) -> String? {
        var where_ = address(kAudioDevicePropertyDeviceUID)
        var uid: Unmanaged<CFString>?
        var size = UInt32(MemoryLayout<Unmanaged<CFString>?>.size)
        guard AudioObjectGetPropertyData(device, &where_, 0, nil, &size, &uid) == noErr, let uid else { return nil }
        return uid.takeRetainedValue() as String
    }

    static func device(uid: String) -> AudioObjectID? {
        devices().first { self.uid($0) == uid }
    }

    static func muted(_ device: AudioObjectID) -> Bool? {
        var where_ = address(kAudioDevicePropertyMute, kAudioObjectPropertyScopeOutput)
        guard AudioObjectHasProperty(device, &where_) else { return nil }
        var value: UInt32 = 0
        var size = UInt32(MemoryLayout<UInt32>.size)
        guard AudioObjectGetPropertyData(device, &where_, 0, nil, &size, &value) == noErr else { return nil }
        return value != 0
    }

    @discardableResult
    static func set(_ device: AudioObjectID, muted: Bool) -> Bool {
        var where_ = address(kAudioDevicePropertyMute, kAudioObjectPropertyScopeOutput)
        var settable: DarwinBoolean = false
        guard AudioObjectHasProperty(device, &where_), AudioObjectIsPropertySettable(device, &where_, &settable) == noErr, settable.boolValue else { return false }
        var value: UInt32 = muted ? 1 : 0
        return AudioObjectSetPropertyData(device, &where_, 0, nil, UInt32(MemoryLayout<UInt32>.size), &value) == noErr
    }
}

/// What this helper muted, so it unmutes exactly that.
final class MuteState {
    private(set) var ours: [String] = []

    func mute() -> [String] {
        for device in OutputMute.devices() where OutputMute.hasOutput(device) && !OutputMute.aggregate(device) {
            guard OutputMute.muted(device) == false, let uid = OutputMute.uid(device), !ours.contains(uid) else { continue }
            if OutputMute.set(device, muted: true) { ours.append(uid) }
        }
        return ours
    }

    /// Unmutes what this muted, and the devices Rust remembered (an earlier helper's, from
    /// before a crash). Returns every device it was asked to unmute, for Rust to forget.
    @discardableResult
    func unmute(_ devices: [String]? = nil) -> [String] {
        var list = ours
        for uid in devices ?? [] where !list.contains(uid) { list.append(uid) }
        for uid in list {
            if let device = OutputMute.device(uid: uid) { OutputMute.set(device, muted: false) }
        }
        ours.removeAll()
        return list
    }
}
