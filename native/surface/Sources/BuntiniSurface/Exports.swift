// The C entry points Rust calls (src-tauri/src/surface/ffi.rs). Every call may come from any
// thread: the bytes are copied and checked here, and the work then runs on the main thread.
import Foundation
import SurfaceKit

public typealias SurfaceActionCallback = @convention(c) (UnsafePointer<UInt8>?, Int) -> Void

/// The contract this library speaks; Rust refuses to start on a mismatch.
@_cdecl("buntini_surface_contract")
public func buntiniSurfaceContract() -> UInt32 { surfaceContract }

/// Creates the windows. Actions come back through `callback`, on the main thread.
@_cdecl("buntini_surface_start")
public func buntiniSurfaceStart(_ callback: SurfaceActionCallback?) -> Bool {
    guard let callback else { return false }
    onMain {
        SurfaceRuntime.shared.start { data in
            data.withUnsafeBytes { raw in
                callback(raw.bindMemory(to: UInt8.self).baseAddress, raw.count)
            }
        }
    }
    return true
}

/// A whole scene as JSON. False when it doesn't decode or speaks another contract.
@_cdecl("buntini_surface_update")
public func buntiniSurfaceUpdate(_ bytes: UnsafePointer<UInt8>?, _ length: Int) -> Bool {
    guard let bytes, length > 0, length <= SurfaceRuntime.sceneLimit else { return false }
    guard let scene = SurfaceRuntime.decode(Data(bytes: bytes, count: length)) else { return false }
    onMain { SurfaceRuntime.shared.show(scene) }
    return true
}

/// Nine voice bands, 0…1, read by the pill at its own frame rate.
@_cdecl("buntini_surface_levels")
public func buntiniSurfaceLevels(_ values: UnsafePointer<Float>?, _ count: Int) {
    guard let values, count == Levels.count else { return }
    SurfaceRuntime.shared.levels.set(Array(UnsafeBufferPointer(start: values, count: count)))
}

/// Closes the windows. Safe to call more than once.
@_cdecl("buntini_surface_stop")
public func buntiniSurfaceStop() {
    onMain { SurfaceRuntime.shared.stop() }
}

func onMain(_ work: @escaping () -> Void) {
    if Thread.isMainThread { work() } else { DispatchQueue.main.async(execute: work) }
}
