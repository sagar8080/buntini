//! The macOS accessibility API, used from Buntini's own process so the Accessibility
//! permission the user gives Buntini applies (a helper program would need its own).
//! Skills use it to press buttons and read text in other apps' windows, the way screen
//! readers do. Every call is synchronous; run it on a blocking thread.
use objc2_application_services::{kAXTrustedCheckOptionPrompt, AXError, AXIsProcessTrustedWithOptions, AXUIElement};
use objc2_core_foundation::{CFArray, CFBoolean, CFDictionary, CFRetained, CFString, CFType};
use std::ptr::NonNull;

/// Whether Buntini may use accessibility. With `ask`, macOS shows its prompt (once) that
/// leads to System Settings → Privacy & Security → Accessibility.
pub fn trusted(ask: bool) -> bool {
    let options = CFDictionary::<CFString, CFBoolean>::from_slices(&[unsafe { kAXTrustedCheckOptionPrompt }], &[CFBoolean::new(ask)]);
    unsafe { AXIsProcessTrustedWithOptions(Some(options.as_opaque())) }
}

pub struct Element(CFRetained<AXUIElement>);

impl Element {
    /// The top of an app's accessibility tree. Chromium-based apps (Spotify, Chrome, Brave,
    /// Edge, Arc) only build their tree once an assistive app asks, which this does.
    pub fn app(pid: i32) -> Element {
        let element = Element(unsafe { AXUIElement::new_application(pid) });
        unsafe {
            element.0.set_messaging_timeout(2.0);
        }
        for name in ["AXManualAccessibility", "AXEnhancedUserInterface"] {
            unsafe {
                element.0.set_attribute_value(&CFString::from_str(name), CFBoolean::new(true));
            }
        }
        element
    }

    fn value(&self, name: &str) -> Option<CFRetained<CFType>> {
        let mut out: *const CFType = std::ptr::null();
        let error = unsafe { self.0.copy_attribute_value(&CFString::from_str(name), NonNull::from(&mut out)) };
        if error != AXError::Success {
            return None;
        }
        NonNull::new(out.cast_mut()).map(|pointer| unsafe { CFRetained::from_raw(pointer) })
    }

    /// A text attribute such as AXRole, AXTitle, AXDescription or AXValue; empty if absent.
    pub fn text(&self, name: &str) -> String {
        self.value(name).and_then(|value| value.downcast::<CFString>().ok()).map(|text| text.to_string()).unwrap_or_default()
    }

    /// Child elements (AXChildren), or another list attribute such as AXWindows.
    pub fn list(&self, name: &str) -> Vec<Element> {
        let Some(array) = self.value(name).and_then(|value| value.downcast::<CFArray>().ok()) else { return Vec::new() };
        let array: CFRetained<CFArray<CFType>> = unsafe { CFRetained::cast_unchecked(array) };
        array.to_vec().into_iter().filter_map(|item| item.downcast::<AXUIElement>().ok()).map(Element).collect()
    }

    /// A single element attribute such as AXFocusedWindow.
    pub fn element(&self, name: &str) -> Option<Element> {
        self.value(name)?.downcast::<AXUIElement>().ok().map(Element)
    }

    pub fn press(&self) -> bool {
        unsafe { self.0.perform_action(&CFString::from_str("AXPress")) == AXError::Success }
    }

    /// Every element under this one, breadth first (page order), up to `limit`.
    pub fn descendants(&self, limit: usize) -> Vec<Element> {
        let mut found = self.list("AXChildren");
        let mut index = 0;
        while index < found.len() && found.len() < limit {
            let children = found[index].list("AXChildren");
            found.extend(children);
            index += 1;
        }
        found.truncate(limit);
        found
    }
}

/// The process id of a running app by bundle identifier.
pub fn pid(bundle: &str) -> Option<i32> {
    let apps = objc2_app_kit::NSRunningApplication::runningApplicationsWithBundleIdentifier(&objc2_foundation::NSString::from_str(bundle));
    apps.iter().next().map(|app| app.processIdentifier())
}

/// Buttons in an app's windows whose label starts with `prefix` ("Play "), in page order,
/// as (button, label without the prefix).
pub fn buttons(pid: i32, prefix: &str) -> Vec<(Element, String)> {
    let app = Element::app(pid);
    app.list("AXWindows")
        .into_iter()
        .flat_map(|window| window.descendants(40_000))
        .filter(|element| element.text("AXRole") == "AXButton")
        .filter_map(|button| {
            let label = [button.text("AXDescription"), button.text("AXTitle")].into_iter().find(|label| label.starts_with(prefix) && label.len() > prefix.len())?;
            Some((button, label[prefix.len()..].to_string()))
        })
        .collect()
}

/// The readable text of the web page showing in an app's front window (a browser), up
/// to `cap` characters: headings, paragraphs and links in page order.
pub fn page_text(pid: i32, cap: usize) -> Option<String> {
    let app = Element::app(pid);
    let window = app.element("AXFocusedWindow").or_else(|| app.list("AXWindows").into_iter().next())?;
    let area = window.descendants(20_000).into_iter().find(|element| element.text("AXRole") == "AXWebArea")?;
    let mut text = String::new();
    for element in area.descendants(30_000) {
        if element.text("AXRole") == "AXStaticText" {
            let value = element.text("AXValue");
            if !value.trim().is_empty() {
                text.push_str(value.trim());
                text.push(' ');
            }
            if text.len() > cap {
                break;
            }
        }
    }
    let title = area.text("AXTitle");
    (!text.trim().is_empty()).then(|| format!("{title}\n{}", text.chars().take(cap).collect::<String>()))
}
