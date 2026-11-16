//! The C entry points of native/surface/Sources/BuntiniSurface/Exports.swift.
type ActionCallback = extern "C" fn(*const u8, usize);

extern "C" {
    fn buntini_surface_contract() -> u32;
    fn buntini_surface_start(callback: ActionCallback) -> bool;
    fn buntini_surface_update(bytes: *const u8, length: usize) -> bool;
    fn buntini_surface_levels(values: *const f32, count: usize);
    fn buntini_surface_stop();
}

pub fn contract() -> u32 {
    // SAFETY: takes nothing and returns a constant.
    unsafe { buntini_surface_contract() }
}

pub fn start(callback: ActionCallback) -> bool {
    // SAFETY: the callback is a plain function that lives for the whole program.
    unsafe { buntini_surface_start(callback) }
}

/// Swift copies the bytes before returning.
pub fn update(json: &[u8]) -> bool {
    // SAFETY: the pointer and length describe a live slice for the duration of the call.
    unsafe { buntini_surface_update(json.as_ptr(), json.len()) }
}

pub fn levels(values: &[f32; 9]) {
    // SAFETY: as for `update`.
    unsafe { buntini_surface_levels(values.as_ptr(), values.len()) }
}

pub fn stop() {
    // SAFETY: Swift moves the work to the main thread itself.
    unsafe { buntini_surface_stop() }
}

/// Rebuilds a byte slice handed over by Swift, copied before the callback returns.
///
/// # Safety
/// `bytes` must point to `length` readable bytes, or be null.
pub unsafe fn copy(bytes: *const u8, length: usize) -> Option<Vec<u8>> {
    if bytes.is_null() || length == 0 || length > 1 << 20 {
        return None;
    }
    Some(std::slice::from_raw_parts(bytes, length).to_vec())
}
