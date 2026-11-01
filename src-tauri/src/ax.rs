//! The macOS accessibility API, used from Buntini's own process so the Accessibility
//! permission the user gives Buntini applies (a helper program would need its own).
//! Skills use it to press buttons and read text in other apps' windows, the way screen
//! readers do. Every call is synchronous; run it on a blocking thread.
use objc2_application_services::{
    kAXTrustedCheckOptionPrompt, AXError, AXIsProcessTrustedWithOptions, AXUIElement, AXValue,
    AXValueType,
};
use objc2_core_foundation::{
    CFArray, CFBoolean, CFDictionary, CFRetained, CFString, CFType, CFURL,
};
use serde::Serialize;
use std::{
    collections::{HashSet, VecDeque},
    ptr::NonNull,
    time::{Duration, Instant},
};

/// Whether Buntini may use accessibility. With `ask`, macOS shows its prompt (once) that
/// leads to System Settings → Privacy & Security → Accessibility.
pub fn trusted(ask: bool) -> bool {
    let options = CFDictionary::<CFString, CFBoolean>::from_slices(
        &[unsafe { kAXTrustedCheckOptionPrompt }],
        &[CFBoolean::new(ask)],
    );
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
                element
                    .0
                    .set_attribute_value(&CFString::from_str(name), CFBoolean::new(true));
            }
        }
        element
    }

    fn value(&self, name: &str) -> Option<CFRetained<CFType>> {
        let mut out: *const CFType = std::ptr::null();
        let error = unsafe {
            self.0
                .copy_attribute_value(&CFString::from_str(name), NonNull::from(&mut out))
        };
        if error != AXError::Success {
            return None;
        }
        NonNull::new(out.cast_mut()).map(|pointer| unsafe { CFRetained::from_raw(pointer) })
    }

    /// A text attribute such as AXRole, AXTitle, AXDescription or AXValue; empty if absent.
    pub fn text(&self, name: &str) -> String {
        self.value(name)
            .and_then(|value| value.downcast::<CFString>().ok())
            .map(|text| text.to_string())
            .unwrap_or_default()
    }

    /// Child elements (AXChildren), or another list attribute such as AXWindows.
    pub fn list(&self, name: &str) -> Vec<Element> {
        let Some(array) = self
            .value(name)
            .and_then(|value| value.downcast::<CFArray>().ok())
        else {
            return Vec::new();
        };
        let array: CFRetained<CFArray<CFType>> = unsafe { CFRetained::cast_unchecked(array) };
        array
            .to_vec()
            .into_iter()
            .filter_map(|item| item.downcast::<AXUIElement>().ok())
            .map(Element)
            .collect()
    }

    /// A single element attribute such as AXFocusedWindow.
    pub fn element(&self, name: &str) -> Option<Element> {
        self.value(name)?
            .downcast::<AXUIElement>()
            .ok()
            .map(Element)
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

    /// Bounded AX reads do not first materialize an arbitrarily large child list.
    fn children_bounded(&self, limit: usize) -> (Vec<Element>, bool) {
        let name = CFString::from_str("AXChildren");
        let mut count = 0;
        let status = unsafe {
            self.0
                .attribute_value_count(&name, NonNull::from(&mut count))
        };
        if status != AXError::Success {
            return (
                vec![],
                status != AXError::AttributeUnsupported && status != AXError::NoValue,
            );
        }
        if count <= 0 {
            return (vec![], count < 0);
        }
        let take = (count as usize).min(limit);
        if take == 0 {
            return (vec![], true);
        }
        let mut out: *const CFArray = std::ptr::null();
        if unsafe {
            self.0
                .copy_attribute_values(&name, 0, take as _, NonNull::from(&mut out))
        } != AXError::Success
        {
            return (vec![], true);
        }
        let Some(pointer) = NonNull::new(out.cast_mut()) else {
            return (vec![], true);
        };
        let array = unsafe { CFRetained::from_raw(pointer) };
        let array: CFRetained<CFArray<CFType>> = unsafe { CFRetained::cast_unchecked(array) };
        let elements = array
            .to_vec()
            .into_iter()
            .filter_map(|item| item.downcast::<AXUIElement>().ok())
            .map(|element| {
                // AX timeouts apply to this exact object; the application's
                // timeout is not inherited by child elements.
                unsafe {
                    element.set_messaging_timeout(0.25);
                }
                Element(element)
            })
            .collect();
        (elements, count as usize > take)
    }

    fn url(&self, attribute: &str) -> Option<String> {
        let value = self.value(attribute)?;
        if let Ok(text) = value.clone().downcast::<CFString>() {
            return Some(text.to_string());
        }
        value
            .downcast::<CFURL>()
            .ok()
            .map(|url| url.string().to_string())
    }

    fn process_id(&self) -> Option<i32> {
        let mut pid = 0;
        (unsafe { self.0.pid(NonNull::from(&mut pid)) } == AXError::Success).then_some(pid)
    }

    fn hidden(&self) -> bool {
        self.value("AXHidden")
            .and_then(|value| value.downcast::<CFBoolean>().ok())
            .is_some_and(|value| value.value())
    }
}

/// A CGPoint or CGSize, as AXValue wraps them.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Pair(f64, f64);

impl Element {
    fn pair(&self, name: &str, kind: AXValueType) -> Option<(f64, f64)> {
        let value = self.value(name)?.downcast::<AXValue>().ok()?;
        let mut pair = Pair::default();
        unsafe { value.value(kind, NonNull::from(&mut pair).cast()) }.then_some((pair.0, pair.1))
    }

    fn set_position(&self, x: f64, y: f64) -> bool {
        let mut pair = Pair(x, y);
        let Some(value) = (unsafe { AXValue::new(AXValueType::CGPoint, NonNull::from(&mut pair).cast()) }) else {
            return false;
        };
        unsafe { self.0.set_attribute_value(&CFString::from_str("AXPosition"), &value) == AXError::Success }
    }
}

/// The cat batting at the window it sits on, once the user said yes: the window is pushed
/// a few points each way and put back where it was, which is then read back. The window is
/// found by its owner and frame; nothing else about it is read.
pub fn knock_window(pid: i32, frame: (f64, f64, f64, f64)) -> Result<(), String> {
    if !trusted(true) {
        return Err("access".into());
    }
    let app = Element(unsafe { AXUIElement::new_application(pid) });
    unsafe { app.0.set_messaging_timeout(1.0) };
    let close = |a: (f64, f64), b: (f64, f64)| (a.0 - b.0).abs() < 2.0 && (a.1 - b.1).abs() < 2.0;
    let window = app
        .list("AXWindows")
        .into_iter()
        .find(|window| {
            window.pair("AXPosition", AXValueType::CGPoint).is_some_and(|at| close(at, (frame.0, frame.1)))
                && window.pair("AXSize", AXValueType::CGSize).is_some_and(|size| close(size, (frame.2, frame.3)))
        })
        .ok_or("gone")?;
    let (x, y) = window.pair("AXPosition", AXValueType::CGPoint).ok_or("gone")?;
    for dx in [9.0, -6.0, 4.0, 0.0] {
        window.set_position(x + dx, y);
        std::thread::sleep(Duration::from_millis(70));
    }
    for _ in 0..3 {
        if window.pair("AXPosition", AXValueType::CGPoint).is_some_and(|at| close(at, (x, y)) && (at.0 - x).abs() < 1.0) {
            return Ok(());
        }
        window.set_position(x, y);
        std::thread::sleep(Duration::from_millis(80));
    }
    Err("moved".into())
}

/// The process id of a running app by bundle identifier.
pub fn pid(bundle: &str) -> Option<i32> {
    let apps = objc2_app_kit::NSRunningApplication::runningApplicationsWithBundleIdentifier(
        &objc2_foundation::NSString::from_str(bundle),
    );
    apps.iter().next().map(|app| app.processIdentifier())
}

/// What an app shows in front: its focused window's title and the text selected in its
/// focused element, up to `cap` characters. Password fields are never read (`secure`).
/// Unlike `Element::app`, this only asks Chromium-style apps to build their tree and
/// leaves every other app's drawing alone, because it runs whenever Buntini opens.
pub struct Selection {
    pub window: String,
    pub text: String,
    pub secure: bool,
}

pub fn selection(pid: i32, cap: usize) -> Selection {
    let app = Element(unsafe { AXUIElement::new_application(pid) });
    unsafe {
        app.0.set_messaging_timeout(1.0);
        app.0.set_attribute_value(
            &CFString::from_str("AXManualAccessibility"),
            CFBoolean::new(true),
        );
    }
    let window = app
        .element("AXFocusedWindow")
        .map(|window| window.text("AXTitle"))
        .unwrap_or_default();
    let Some(focused) = app.element("AXFocusedUIElement") else {
        return Selection { window, text: String::new(), secure: false };
    };
    if [focused.text("AXRole"), focused.text("AXSubrole")]
        .iter()
        .any(|role| role == "AXSecureTextField")
    {
        return Selection { window, text: String::new(), secure: true };
    }
    let text = focused.text("AXSelectedText").trim().chars().take(cap).collect();
    Selection { window, text, secure: false }
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
            let label = [button.text("AXDescription"), button.text("AXTitle")]
                .into_iter()
                .find(|label| label.starts_with(prefix) && label.len() > prefix.len())?;
            Some((button, label[prefix.len()..].to_string()))
        })
        .collect()
}

/// The readable text of the web page showing in an app's front window (a browser), up
/// to `cap` characters: headings, paragraphs and links in page order.
pub fn page_text(pid: i32, cap: usize) -> Option<String> {
    let app = Element::app(pid);
    let window = app
        .element("AXFocusedWindow")
        .or_else(|| app.list("AXWindows").into_iter().next())?;
    let area = window
        .descendants(20_000)
        .into_iter()
        .find(|element| element.text("AXRole") == "AXWebArea")?;
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
    (!text.trim().is_empty())
        .then(|| format!("{title}\n{}", text.chars().take(cap).collect::<String>()))
}

const RESEARCH_NODE_CAP: usize = 12_000;
const RESEARCH_TEXT_CAP: usize = 24_000;
const RESEARCH_LINK_CAP: usize = 128;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AxResearchLink {
    pub url: String,
    pub label: String,
    pub heading: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AxResearchPage {
    pub url: String,
    pub title: String,
    pub text: String,
    pub links: Vec<AxResearchLink>,
    pub truncated: bool,
}

#[derive(Clone, Copy)]
struct ResearchLinkContext {
    index: usize,
    fill_label: bool,
}

fn record_link_descendant(
    links: &mut [AxResearchLink],
    context: Option<ResearchLinkContext>,
    role: &str,
    value: &str,
) {
    let Some(context) = context else {
        return;
    };
    let Some(link) = links.get_mut(context.index) else {
        return;
    };
    // Chromium can expose either heading → link or link → heading. Carry
    // the link through the existing bounded walk instead of rescanning it.
    link.heading |= role == "AXHeading";
    if context.fill_label && role == "AXStaticText" {
        let mut remaining = 500usize.saturating_sub(link.label.chars().count());
        append_bounded(&mut link.label, &mut remaining, value);
    }
}

fn research_request(
    pid: i32,
    expected_url: &str,
    text_cap: usize,
    link_cap: usize,
) -> Result<reqwest::Url, String> {
    if pid <= 0 {
        return Err("A specific running browser process is required.".into());
    }
    if !(1..=RESEARCH_TEXT_CAP).contains(&text_cap) || !(1..=RESEARCH_LINK_CAP).contains(&link_cap)
    {
        return Err("The requested browser snapshot exceeds its text or link bounds.".into());
    }
    crate::research::validate_public_url(expected_url)
}

fn same_document(expected: &reqwest::Url, observed: &str) -> bool {
    crate::research::validate_public_url(observed).is_ok_and(|observed| observed == *expected)
}

fn frontmost_pid() -> Option<i32> {
    objc2_app_kit::NSWorkspace::sharedWorkspace()
        .frontmostApplication()
        .map(|app| app.processIdentifier())
}

fn deadline(deadline: Instant) -> Result<(), String> {
    if Instant::now() >= deadline {
        Err("The browser accessibility snapshot exceeded its time limit.".into())
    } else {
        Ok(())
    }
}

fn append_bounded(text: &mut String, remaining: &mut usize, value: &str) -> bool {
    let value = value.trim();
    if value.is_empty() {
        return false;
    }
    let separator = usize::from(!text.is_empty());
    if *remaining <= separator {
        return true;
    }
    if separator > 0 {
        text.push('\n');
        *remaining -= 1;
    }
    let mut chars = value.chars();
    let piece: String = chars.by_ref().take(*remaining).collect();
    *remaining -= piece.chars().count();
    text.push_str(&piece);
    chars.next().is_some()
}

/// Read only the currently focused document in an expected browser process.
/// The session owner must verify its exact created tab is active before and
/// after calling this on a blocking thread. Missing document identity fails
/// closed; no other window, tab, address-bar value, or form field is read.
pub fn research_page(
    pid: i32,
    expected_url: &str,
    text_cap: usize,
    link_cap: usize,
) -> Result<AxResearchPage, String> {
    let expected = research_request(pid, expected_url, text_cap, link_cap)?;
    if !trusted(false) {
        return Err(
            "Browser page reading needs Buntini's existing Accessibility permission.".into(),
        );
    }
    if frontmost_pid() != Some(pid) {
        return Err("The research browser is no longer the frontmost app.".into());
    }
    let deadline_at = Instant::now() + Duration::from_secs(8);
    let app = Element::app(pid);
    unsafe {
        app.0.set_messaging_timeout(0.25);
    }
    let window = app
        .element("AXFocusedWindow")
        .ok_or("The research browser has no focused window.")?;
    unsafe {
        window.0.set_messaging_timeout(0.25);
    }
    if window.process_id() != Some(pid) {
        return Err("The focused window belongs to a different process.".into());
    }
    let (children, mut truncated) = window.children_bounded(RESEARCH_NODE_CAP);
    let mut queue: VecDeque<Element> = children.into();
    let mut visited = 0;
    let area = loop {
        deadline(deadline_at)?;
        let element = queue
            .pop_front()
            .ok_or("The focused research tab exposes no accessible web document.")?;
        visited += 1;
        if element.hidden() {
            continue;
        }
        if element.text("AXRole") == "AXWebArea" {
            let observed = element
                .url("AXURL")
                .or_else(|| element.url("AXDocument"))
                .ok_or("The browser did not expose a verifiable document URL.")?;
            if !same_document(&expected, &observed) {
                return Err("The focused web document does not match the research tab URL.".into());
            }
            break element;
        }
        let (children, cut) =
            element.children_bounded(RESEARCH_NODE_CAP.saturating_sub(visited + queue.len()));
        truncated |= cut;
        queue.extend(children);
    };
    if window
        .url("AXDocument")
        .is_some_and(|url| !same_document(&expected, &url))
    {
        return Err("The research window's document changed before reading.".into());
    }
    let title = area.text("AXTitle").chars().take(512).collect();
    let (children, cut) = area.children_bounded(RESEARCH_NODE_CAP);
    truncated |= cut;
    let mut queue: VecDeque<(Element, bool, Option<ResearchLinkContext>)> = children
        .into_iter()
        .map(|element| (element, false, None))
        .collect();
    let mut text = String::new();
    let mut remaining = text_cap;
    let mut links = Vec::new();
    let mut seen = HashSet::new();
    let mut visited = 0;
    let mut main_scoped = false;
    while let Some((element, within_heading, mut link_context)) = queue.pop_front() {
        deadline(deadline_at)?;
        visited += 1;
        if element.hidden() {
            continue;
        }
        let role = element.text("AXRole");
        // An embedded document can have a different origin/account. Only this
        // explicitly selected top-level page is in the snapshot scope.
        if role == "AXWebArea" {
            truncated = true;
            continue;
        }
        if !main_scoped
            && (role == "AXLandmarkMain" || element.text("AXSubrole") == "AXLandmarkMain")
        {
            // Prefer the page's main landmark over account/navigation chrome.
            // Discard anything collected before discovering the landmark.
            main_scoped = true;
            text.clear();
            remaining = text_cap;
            links.clear();
            seen.clear();
            queue.clear();
            let (children, cut) =
                element.children_bounded(RESEARCH_NODE_CAP.saturating_sub(visited));
            truncated |= cut;
            queue.extend(children.into_iter().map(|child| (child, false, None)));
            continue;
        }
        let heading = within_heading || role == "AXHeading";
        let value = if role == "AXStaticText" {
            element.text("AXValue")
        } else {
            String::new()
        };
        record_link_descendant(&mut links, link_context, &role, &value);
        if role == "AXStaticText" {
            truncated |= append_bounded(&mut text, &mut remaining, &value);
        }
        let (children, cut) =
            element.children_bounded(RESEARCH_NODE_CAP.saturating_sub(visited + queue.len()));
        truncated |= cut;
        if role == "AXLink" {
            link_context = None;
            if let Some(raw) = element.url("AXURL") {
                if let Some(url) = expected
                    .join(&raw)
                    .ok()
                    .and_then(|url| crate::research::validate_public_url(url.as_str()).ok())
                {
                    if seen.insert(url.to_string()) {
                        if links.len() < link_cap {
                            let mut label = [
                                element.text("AXTitle"),
                                element.text("AXDescription"),
                                element.text("AXValue"),
                            ]
                            .into_iter()
                            .find(|value| !value.trim().is_empty())
                            .unwrap_or_default();
                            if label.is_empty() {
                                label = children
                                    .iter()
                                    .take(8)
                                    .filter(|child| child.text("AXRole") == "AXStaticText")
                                    .map(|child| child.text("AXValue"))
                                    .collect::<Vec<_>>()
                                    .join(" ");
                            }
                            link_context = Some(ResearchLinkContext {
                                index: links.len(),
                                fill_label: label.is_empty(),
                            });
                            links.push(AxResearchLink {
                                url: url.to_string(),
                                label: label.chars().take(500).collect(),
                                heading,
                            });
                        } else {
                            truncated = true;
                        }
                    }
                }
            }
        }
        queue.extend(
            children
                .into_iter()
                .map(|child| (child, heading, link_context)),
        );
        if visited >= RESEARCH_NODE_CAP {
            truncated |= !queue.is_empty();
            break;
        }
    }
    deadline(deadline_at)?;
    let focused = app
        .element("AXFocusedWindow")
        .ok_or("The research browser lost its focused window.")?;
    if frontmost_pid() != Some(pid) || focused.0 != window.0 {
        return Err("The focused browser window changed during the snapshot.".into());
    }
    let observed = area
        .url("AXURL")
        .or_else(|| area.url("AXDocument"))
        .ok_or("The research document URL is no longer available.")?;
    if !same_document(&expected, &observed)
        || window
            .url("AXDocument")
            .is_some_and(|url| !same_document(&expected, &url))
    {
        return Err("The research document changed during the snapshot.".into());
    }
    if text.trim().is_empty() && links.is_empty() {
        return Err("The research page exposed no readable text or links.".into());
    }
    Ok(AxResearchPage {
        url: expected.to_string(),
        title,
        text,
        links,
        truncated,
    })
}

#[cfg(test)]
mod research_tests {
    use super::*;

    #[test]
    fn research_bounds_and_public_url_are_validated_before_any_ax_access() {
        for (pid, url, text, links) in [
            (0, "https://example.com", 100, 8),
            (1, "http://example.com", 100, 8),
            (1, "https://127.0.0.1", 100, 8),
            (1, "https://user:secret@example.com", 100, 8),
            (1, "https://example.com", 0, 8),
            (1, "https://example.com", 24_001, 8),
            (1, "https://example.com", 100, 129),
        ] {
            assert!(research_request(pid, url, text, links).is_err());
        }
        assert!(
            research_request(123, "https://www.google.com/search?q=C%2B%2B", 24_000, 64).is_ok()
        );
    }

    #[test]
    fn research_document_binding_preserves_query_path_and_origin() {
        let expected =
            research_request(123, "https://www.google.com/search?q=C%2B%2B", 100, 8).unwrap();
        assert!(same_document(
            &expected,
            "https://www.google.com/search?q=C%2B%2B#results"
        ));
        for observed in [
            "https://www.google.com/search?q=private",
            "https://accounts.google.com/search?q=C%2B%2B",
            "https://www.google.com/private?q=C%2B%2B",
            "file:///private/data",
            "https://google.com/search?q=C%2B%2B",
        ] {
            assert!(!same_document(&expected, observed));
        }
    }

    #[test]
    fn text_caps_count_unicode_characters_and_report_cut_content() {
        let mut text = String::new();
        let mut remaining = 5;
        assert!(!append_bounded(&mut text, &mut remaining, "café"));
        assert!(append_bounded(&mut text, &mut remaining, "more"));
        assert_eq!(text, "café");
        assert!(text.chars().count() <= 5);
        let mut text = String::new();
        let mut remaining = 3;
        assert!(append_bounded(&mut text, &mut remaining, "東京駅前"));
        assert_eq!(text, "東京駅");
        assert_eq!(remaining, 0);
    }

    #[test]
    fn link_heading_children_and_nested_text_are_recorded_without_an_extra_walk() {
        let mut links = vec![AxResearchLink {
            url: "https://example.com/".into(),
            label: String::new(),
            heading: false,
        }];
        let context = Some(ResearchLinkContext {
            index: 0,
            fill_label: true,
        });
        record_link_descendant(&mut links, context, "AXHeading", "");
        record_link_descendant(&mut links, context, "AXStaticText", "Museum opening hours");
        assert!(links[0].heading);
        assert_eq!(links[0].label, "Museum opening hours");
        record_link_descendant(&mut links, context, "AXStaticText", &"x".repeat(1000));
        assert_eq!(links[0].label.chars().count(), 500);

        let mut links = vec![AxResearchLink {
            url: "https://example.com/".into(),
            label: "Existing accessible title".into(),
            heading: true,
        }];
        record_link_descendant(
            &mut links,
            Some(ResearchLinkContext {
                index: 0,
                fill_label: false,
            }),
            "AXStaticText",
            "Do not duplicate this",
        );
        assert!(links[0].heading);
        assert_eq!(links[0].label, "Existing accessible title");
    }
}
