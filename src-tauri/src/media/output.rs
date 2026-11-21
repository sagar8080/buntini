//! Where the Mac's sound goes, for the music card: the outputs Core Audio lists (the Mac's
//! speakers, AirPods, a display, an AirPlay speaker) and which one plays now. Choosing one
//! makes it the Mac's output, as Control Center does. Read and set from Buntini's own
//! process, with no permission. Hidden, virtual and combined devices aren't offered: they're
//! other apps' plumbing, not a place to listen.
use crate::surface::cards::media::Output;

/// A device's kind, from how it's connected and, for Bluetooth, its name.
pub fn kind(transport: &[u8; 4], name: &str) -> &'static str {
    match transport {
        b"bltn" => "builtIn",
        b"blue" | b"blea" if name.to_lowercase().contains("airpods") => "airpods",
        b"blue" | b"blea" => "headphones",
        b"hdmi" | b"dprt" | b"thun" => "display",
        b"airp" => "airplay",
        b"usb " => "usb",
        _ => "other",
    }
}

/// Whether a device with this connection is offered.
pub fn offered(transport: &[u8; 4], hidden: bool) -> bool {
    !hidden && !matches!(transport, b"virt" | b"grup")
}

/// The outputs to offer, the one in use marked; the Mac's own first, then by name.
pub fn list() -> Vec<Output> {
    let mut outputs = audio::outputs();
    outputs.sort_by(|a, b| (a.kind != "builtIn", a.name.to_lowercase()).cmp(&(b.kind != "builtIn", b.name.to_lowercase())));
    outputs
}

/// Makes the output with `uid` the Mac's, if it's still there.
pub fn choose(uid: &str) -> Result<(), String> {
    let device = audio::device(uid).ok_or("That output isn't there any more.")?;
    if !audio::set_default(device) {
        return Err("The Mac didn't take that output.".into());
    }
    Ok(())
}

#[cfg(target_os = "macos")]
mod audio {
    use std::ffi::c_void;
    use std::ptr::null;

    use objc2::rc::Retained;
    use objc2_foundation::NSString;

    use super::Output;

    #[repr(C)]
    struct Address {
        selector: u32,
        scope: u32,
        element: u32,
    }

    const fn code(text: &[u8; 4]) -> u32 {
        u32::from_be_bytes(*text)
    }

    const SYSTEM: u32 = 1;
    const GLOBAL: u32 = code(b"glob");
    const OUTPUT: u32 = code(b"outp");
    const MAIN: u32 = 0;
    const DEVICES: u32 = code(b"dev#");
    const STREAMS: u32 = code(b"stm#");
    const NAME: u32 = code(b"lnam");
    const UID: u32 = code(b"uid ");
    const FROM_UID: u32 = code(b"uidd");
    const TRANSPORT: u32 = code(b"tran");
    const HIDDEN: u32 = code(b"hidn");
    const CAN_BE_DEFAULT: u32 = code(b"dflt");
    const DEFAULT_OUTPUT: u32 = code(b"dOut");
    /// More devices than this isn't a list Core Audio gave.
    const MOST: usize = 256;

    #[link(name = "CoreAudio", kind = "framework")]
    extern "C" {
        fn AudioObjectGetPropertyDataSize(object: u32, address: *const Address, qualifier_size: u32, qualifier: *const c_void, size: *mut u32) -> i32;
        fn AudioObjectGetPropertyData(object: u32, address: *const Address, qualifier_size: u32, qualifier: *const c_void, size: *mut u32, data: *mut c_void) -> i32;
        fn AudioObjectSetPropertyData(object: u32, address: *const Address, qualifier_size: u32, qualifier: *const c_void, size: u32, data: *const c_void) -> i32;
    }

    fn get(object: u32, selector: u32, scope: u32) -> Option<u32> {
        let address = Address { selector, scope, element: MAIN };
        let mut value = 0u32;
        let mut size = 4u32;
        // SAFETY: `value` has room for the four bytes Core Audio writes.
        let status = unsafe { AudioObjectGetPropertyData(object, &address, 0, null(), &mut size, &mut value as *mut u32 as *mut c_void) };
        (status == 0 && size == 4).then_some(value)
    }

    fn size(object: u32, selector: u32, scope: u32) -> Option<u32> {
        let address = Address { selector, scope, element: MAIN };
        let mut size = 0u32;
        // SAFETY: only the size is written.
        (unsafe { AudioObjectGetPropertyDataSize(object, &address, 0, null(), &mut size) } == 0).then_some(size)
    }

    fn string(object: u32, selector: u32) -> Option<String> {
        let address = Address { selector, scope: GLOBAL, element: MAIN };
        let mut value: *mut NSString = std::ptr::null_mut();
        let mut size = std::mem::size_of::<*mut NSString>() as u32;
        // SAFETY: Core Audio writes one CFString reference, which the caller then owns.
        let status = unsafe { AudioObjectGetPropertyData(object, &address, 0, null(), &mut size, &mut value as *mut *mut NSString as *mut c_void) };
        if status != 0 || value.is_null() {
            return None;
        }
        // SAFETY: a CFString is an NSString; taking it over releases it once dropped.
        let owned = unsafe { Retained::from_raw(value) }?;
        Some(owned.to_string())
    }

    pub fn outputs() -> Vec<Output> {
        let Some(bytes) = size(SYSTEM, DEVICES, GLOBAL) else { return Vec::new() };
        let count = bytes as usize / 4;
        if count == 0 || count > MOST {
            return Vec::new();
        }
        let address = Address { selector: DEVICES, scope: GLOBAL, element: MAIN };
        let mut ids = vec![0u32; count];
        let mut bytes = bytes;
        // SAFETY: `ids` has room for `bytes` bytes.
        if unsafe { AudioObjectGetPropertyData(SYSTEM, &address, 0, null(), &mut bytes, ids.as_mut_ptr() as *mut c_void) } != 0 {
            return Vec::new();
        }
        ids.truncate(bytes as usize / 4);
        let current = get(SYSTEM, DEFAULT_OUTPUT, GLOBAL);
        ids.into_iter()
            .filter(|id| size(*id, STREAMS, OUTPUT).is_some_and(|bytes| bytes > 0) && get(*id, CAN_BE_DEFAULT, OUTPUT).is_some_and(|can| can != 0))
            .filter_map(|id| {
                let transport = get(id, TRANSPORT, GLOBAL).unwrap_or(0).to_be_bytes();
                if !super::offered(&transport, get(id, HIDDEN, GLOBAL).unwrap_or(0) != 0) {
                    return None;
                }
                let name = string(id, NAME)?;
                Some(Output { uid: string(id, UID)?, kind: super::kind(&transport, &name).into(), name, current: Some(id) == current })
            })
            .collect()
    }

    pub fn device(uid: &str) -> Option<u32> {
        let address = Address { selector: FROM_UID, scope: GLOBAL, element: MAIN };
        let text = NSString::from_str(uid);
        let reference: *const NSString = Retained::as_ptr(&text);
        let mut device = 0u32;
        let mut size = 4u32;
        // SAFETY: the qualifier is one CFString reference, kept alive by `text`.
        let status = unsafe {
            AudioObjectGetPropertyData(SYSTEM, &address, std::mem::size_of::<*const NSString>() as u32, &reference as *const *const NSString as *const c_void, &mut size, &mut device as *mut u32 as *mut c_void)
        };
        (status == 0 && device != 0).then_some(device)
    }

    pub fn set_default(device: u32) -> bool {
        let address = Address { selector: DEFAULT_OUTPUT, scope: GLOBAL, element: MAIN };
        // SAFETY: Core Audio reads the four bytes of `device`.
        unsafe { AudioObjectSetPropertyData(SYSTEM, &address, 0, null(), 4, &device as *const u32 as *const c_void) == 0 }
    }

    /// The output in use, by its UID.
    #[cfg(test)]
    pub fn current() -> Option<String> {
        get(SYSTEM, DEFAULT_OUTPUT, GLOBAL).and_then(|device| string(device, UID))
    }
}

#[cfg(not(target_os = "macos"))]
mod audio {
    use super::Output;
    pub fn outputs() -> Vec<Output> {
        Vec::new()
    }
    pub fn device(_: &str) -> Option<u32> {
        None
    }
    pub fn set_default(_: u32) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_output_is_known_by_how_it_connects() {
        assert_eq!(kind(b"bltn", "MacBook Pro Speakers"), "builtIn");
        assert_eq!(kind(b"blue", "Ana’s AirPods Pro"), "airpods");
        assert_eq!(kind(b"blea", "WH-1000XM5"), "headphones");
        assert_eq!(kind(b"hdmi", "LG UltraFine"), "display");
        assert_eq!(kind(b"airp", "Living Room"), "airplay");
        assert_eq!(kind(b"usb ", "Scarlett 2i2"), "usb");
        assert_eq!(kind(b"????", "Something"), "other");
        assert!(offered(b"bltn", false) && offered(b"airp", false));
        assert!(!offered(b"virt", false) && !offered(b"grup", false) && !offered(b"bltn", true));
    }

    /// The outputs on this Mac, read as the card reads them: `cargo test --lib
    /// media::output::tests::live -- --ignored --nocapture`. With `BUNTINI_LIVE_SWITCH=1`, and
    /// when there's more than one, it chooses another and puts the first back.
    #[test]
    #[ignore]
    fn live() {
        let outputs = list();
        for output in &outputs {
            println!("{output:?}");
        }
        assert_eq!(outputs.iter().filter(|output| output.current).count(), 1, "one output is in use");
        if std::env::var("BUNTINI_LIVE_SWITCH").is_ok_and(|value| value == "1") && outputs.len() > 1 {
            let before = audio::current().unwrap();
            let other = outputs.iter().find(|output| !output.current).unwrap();
            choose(&other.uid).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(300));
            assert_eq!(audio::current().as_deref(), Some(other.uid.as_str()));
            choose(&before).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(300));
            assert_eq!(audio::current(), Some(before.clone()));
            println!("switched to {} and back to {before}", other.name);
        }
    }
}
