//! Putting one product in the cart in the user's own browser, through macOS
//! Accessibility. Buntini opens the page and reads it before the approval sheet, so the
//! sheet names what it found. Once approved it presses that product's Add to Cart, opens
//! the cart and reads the item back, then stops: the tab is the user's from there.
//!
//! Rust reads the page and picks the button. No page text reaches a model, so a page
//! cannot talk it into anything. Only Add to Cart, a search result and the cart link are
//! ever pressed; checkout and payment controls are refused by name. A human check is
//! never answered: the tab is left for the user.
use crate::actions::{ActionOutcome, ActionReceipt, Field};
use serde_json::{json, Map, Value};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

pub const TOOL: &str = "add_to_cart";

/// Browsers where this has worked on real shops, with their Settings → Apps id. Safari
/// only showed part of a page this way, and Brave's page never appeared.
const BROWSERS: [(&str, &str); 1] = [("com.google.Chrome", "chrome")];

/// Where a shop's own search lives. Other shops need a link, or their exact address and
/// the common `/search?q=` path, so a name is never guessed into a lookalike site.
const SHOPS: [(&str, &str, &str); 8] = [
    ("target", "www.target.com", "/s?searchTerm="),
    ("walmart", "www.walmart.com", "/search?q="),
    ("best buy", "www.bestbuy.com", "/site/searchpage.jsp?st="),
    ("bestbuy", "www.bestbuy.com", "/site/searchpage.jsp?st="),
    ("newegg", "www.newegg.com", "/p/pl?d="),
    ("b&h", "www.bhphotovideo.com", "/c/search?Ntt="),
    ("adorama", "www.adorama.com", "/l/?searchinfo="),
    ("keychron", "www.keychron.com", "/search?q="),
];

const NOT_PRESSED: [&str; 11] = [
    "checkout", "check out", "buy now", "buy with", "place order", "pay ", "paypal", "subscribe", "sign in", "wishlist", "save for later",
];

#[derive(Debug, Clone, Default, PartialEq)]
struct Control {
    link: bool,
    label: String,
    enabled: bool,
    url: String,
}

#[derive(Debug, Clone, Default)]
struct Page {
    url: String,
    title: String,
    heading: String,
    text: String,
    controls: Vec<Control>,
}

#[derive(Debug, PartialEq)]
enum Pick {
    Press(usize),
    Unavailable,
    Missing,
}

#[derive(Debug, PartialEq)]
enum CartState {
    Holds,
    Empty,
    Unclear,
}

fn words(text: &str) -> Vec<String> {
    const SKIP: [&str; 13] = ["a", "an", "and", "at", "for", "from", "in", "of", "on", "the", "to", "with", "your"];
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty() && !SKIP.contains(word))
        .map(str::to_string)
        .collect()
}

/// How much of the item's name appears in a label.
fn overlap(label: &str, item: &str) -> f32 {
    let wanted = words(item);
    if wanted.is_empty() {
        return 0.0;
    }
    let seen = words(label);
    wanted.iter().filter(|word| seen.contains(word)).count() as f32 / wanted.len() as f32
}

/// Whether a label names this item: most of its words, and every word with a digit in it,
/// since "AirPods 4" isn't "AirPods Pro 2" however many other words they share.
fn names(label: &str, item: &str) -> bool {
    let seen = words(label);
    overlap(label, item) >= 0.5 && words(item).iter().filter(|word| word.chars().any(|c| c.is_ascii_digit())).all(|word| seen.contains(word))
}

fn refused(label: &str) -> bool {
    let lower = format!("{} ", label.to_lowercase());
    NOT_PRESSED.iter().any(|name| lower.contains(name))
}

fn cart_noun(lower: &str) -> Option<&'static str> {
    ["to cart", "to bag", "to basket", "to your cart", "to your bag", "to my cart"].into_iter().find(|noun| lower.contains(noun))
}

fn adds_to_cart(label: &str) -> bool {
    let lower = label.to_lowercase();
    let starts = lower.trim_start().starts_with("add ") || lower.contains(" add ");
    starts && cart_noun(&lower).is_some() && !refused(label)
}

/// "Add to cart" or "Add to Cart - $129", naming no product of its own.
fn plain_add(label: &str) -> bool {
    let lower = label.to_lowercase();
    let Some(noun) = cart_noun(&lower) else { return false };
    let rest = lower.replacen(noun, "", 1).replacen("add", "", 1);
    !rest.chars().any(char::is_alphabetic)
}

/// The first Add to Cart in page order that is plain or names this item. A disabled one
/// stops here: the next button down is usually a recommendation for something else.
fn pick_add(controls: &[Control], item: &str) -> Pick {
    for (at, control) in controls.iter().enumerate() {
        if adds_to_cart(&control.label) && (plain_add(&control.label) || overlap(&control.label, item) >= 0.8) {
            return if control.enabled { Pick::Press(at) } else { Pick::Unavailable };
        }
    }
    Pick::Missing
}

fn removes(control: &Control) -> bool {
    let lower = control.label.to_lowercase();
    !refused(&control.label) && ["remove", "delete", "decrease quantity"].iter().any(|verb| lower.starts_with(verb))
}

/// How many things the shop's cart link says it holds: "cart 0 items", "Cart 1 item".
fn cart_count(controls: &[Control]) -> Option<u32> {
    controls.iter().find_map(|control| {
        let lower = control.label.to_lowercase();
        let short = lower.len() < 30 && ["cart", "bag", "basket", "shopping cart"].iter().any(|name| lower.starts_with(name));
        short.then(|| lower.split(|c: char| !c.is_ascii_digit()).find(|part| !part.is_empty()).and_then(|part| part.parse().ok())).flatten()
    })
}

/// Recommendations name products too, so a cart holds the item only when a control takes
/// that item out: one that names it, or one in the row right after a link that names it.
fn cart_state(page: &Page, item: &str) -> CartState {
    let named_row = page.controls.iter().enumerate().any(|(at, control)| {
        (removes(control) && names(&control.label, item))
            || (control.link && names(&control.label, item) && page.controls[at + 1..].iter().take(8).any(removes))
    });
    if named_row {
        return CartState::Holds;
    }
    let text = page.text.to_lowercase();
    let says_empty = ["cart is empty", "bag is empty", "cart is currently empty", "nothing in your cart", "no items in your cart"].iter().any(|phrase| text.contains(phrase));
    if says_empty || cart_count(&page.controls) == Some(0) {
        CartState::Empty
    } else {
        CartState::Unclear
    }
}

/// The shop's count going up, when it shows one both times, is what proves the press took:
/// the item may have been in the cart already.
fn press_took(before: Option<u32>, after: Option<u32>) -> bool {
    !matches!((before, after), (Some(before), Some(after)) if after <= before)
}

fn human_check(page: &Page) -> bool {
    let text = page.text.to_lowercase();
    let title = page.title.to_lowercase();
    ["verify you are human", "confirm you are human", "are you a robot", "press & hold", "press and hold", "unusual activity", "checking your browser", "pardon our interruption", "complete the security check"]
        .iter()
        .any(|phrase| text.contains(phrase) || title.contains(phrase))
        || ["access denied", "just a moment", "attention required"].iter().any(|phrase| title.contains(phrase))
}

fn host_of(raw: &str) -> String {
    reqwest::Url::parse(raw).ok().and_then(|url| url.host_str().map(str::to_string)).unwrap_or_default()
}
fn shop_name(raw: &str) -> String {
    host_of(raw).trim_start_matches("www.").to_string()
}
fn same_shop(left: &str, right: &str) -> bool {
    let (left, right) = (shop_name(left), shop_name(right));
    !left.is_empty() && (left == right || left.ends_with(&format!(".{right}")) || right.ends_with(&format!(".{left}")))
}
fn same_path(left: &str, right: &str) -> bool {
    match (reqwest::Url::parse(left), reqwest::Url::parse(right)) {
        (Ok(left), Ok(right)) => same_shop(left.as_str(), right.as_str()) && left.path().trim_end_matches('/') == right.path().trim_end_matches('/'),
        _ => false,
    }
}
/// The page approved, down to its query, which is often the variant.
fn same_page(left: &str, right: &str) -> bool {
    same_path(left, right) && reqwest::Url::parse(left).ok().map(|url| url.query().map(str::to_string)) == reqwest::Url::parse(right).ok().map(|url| url.query().map(str::to_string))
}

/// The cart itself, not `/cart/change`, `/cart/clear`, `/cart/123:1` or checkout.
fn cart_path(url: &str) -> bool {
    reqwest::Url::parse(url).is_ok_and(|url| {
        url.path().to_lowercase().trim_end_matches('/').rsplit('/').next().is_some_and(|part| ["cart", "cart.html", "bag", "basket", "shoppingcart"].contains(&part))
    })
}
fn cart_link(page: &Page) -> Option<usize> {
    page.controls.iter().position(|control| {
        control.link
            && !refused(&control.label)
            && !removes(control)
            && !adds_to_cart(&control.label)
            && cart_path(&control.url)
            && same_shop(&control.url, &page.url)
            && reqwest::Url::parse(&control.url).is_ok_and(|url| url.scheme() == "https" && url.query().is_none())
    })
}

/// Where the item's words sit together in a label, if they do. "MX Master 3S" names the
/// mouse in "Logitech MX Master 3S Wireless" but only part of a combo in "MX Keys Combo:
/// Logitech Keys, MX Master 3S" or an accessory in "Case for MX Master 3S".
fn phrase_at(label: &str, item: &str) -> Option<usize> {
    let (seen, wanted) = (words(label), words(item));
    if wanted.is_empty() {
        return None;
    }
    seen.windows(wanted.len()).position(|run| run == wanted.as_slice())
}

/// The result that names the item most plainly, on this shop, and isn't its cart.
fn result_link(page: &Page, item: &str) -> Option<usize> {
    page.controls
        .iter()
        .enumerate()
        .filter(|(_, control)| control.link && control.url.starts_with("https://") && overlap(&control.label, item) >= 0.8 && same_shop(&control.url, &page.url) && !cart_path(&control.url) && !same_path(&control.url, &page.url))
        .min_by_key(|(at, control)| (phrase_at(&control.label, item).unwrap_or(usize::MAX), *at))
        .map(|(at, _)| at)
}

fn product_name(page: &Page) -> String {
    let heading = page.heading.trim();
    if !heading.is_empty() {
        return heading.chars().take(200).collect();
    }
    let title = page.title.trim();
    let name = [" | ", " : ", " – ", " - "].iter().filter_map(|mark| title.split_once(mark).map(|(name, _)| name)).min_by_key(|name| name.len()).unwrap_or(title);
    name.chars().take(200).collect()
}

fn search_url(shop: &str, item: &str) -> Result<String, String> {
    let wanted = shop.trim().to_lowercase();
    let encoded: String = reqwest::Url::parse_with_params("https://a.invalid/", [("q", item)])
        .map_err(|_| "That item name can't be searched.")?
        .query()
        .unwrap_or_default()
        .trim_start_matches("q=")
        .to_string();
    if let Some((_, host, path)) = SHOPS.iter().find(|(name, ..)| *name == wanted || wanted == name.replace(' ', "") + ".com") {
        return Ok(format!("https://{host}{path}{encoded}"));
    }
    if wanted.contains('.') && !wanted.contains(char::is_whitespace) {
        let page = crate::research::validate_public_url(&format!("https://{}/search?q={encoded}", wanted.trim_start_matches("https://").trim_end_matches('/')))?;
        return Ok(page.to_string());
    }
    Err(format!("Send me the link to it at {} and I'll add it.", shop.trim()))
}

fn text(args: &Map<String, Value>, key: &str, max: usize) -> Result<Option<String>, String> {
    let Some(value) = args.get(key) else { return Ok(None) };
    let value = value.as_str().ok_or_else(|| format!("{key} must be text."))?.trim();
    if value.is_empty() {
        return Ok(None);
    }
    if value.chars().count() > max || value.chars().any(char::is_control) {
        return Err(format!("{key} is too long."));
    }
    Ok(Some(value.to_string()))
}

/// A plain request to put something in a cart, read without a model so it can't be
/// refused or given a shop nobody named. Anything else, or anything more, is the model's.
pub fn parse_request(prompt: &str) -> Option<Map<String, Value>> {
    let tidy: String = prompt.trim().trim_end_matches(['.', '!', '?']).to_string();
    let mut said: Vec<&str> = tidy.split_whitespace().collect();
    loop {
        let lead = said.first()?.to_lowercase();
        let lead = lead.trim_end_matches(',');
        match lead {
            "please" | "hey" | "buntini" | "ok" | "okay" => { said.remove(0); }
            "can" | "could" | "would" | "will" if said.get(1).is_some_and(|word| word.eq_ignore_ascii_case("you")) => { said.drain(..2); }
            _ => break,
        }
    }
    let lower: Vec<String> = said.iter().map(|word| word.to_lowercase().trim_matches(|c: char| !c.is_alphanumeric()).to_string()).collect();
    let joined = format!(" {} ", lower.join(" "));
    if [" then ", " remind ", " reminder ", " note ", " timer ", " calendar ", " tell ", " email ", " text ", " list ", " and check ", " and tell ", " and open "].iter().any(|word| joined.contains(word)) {
        return None;
    }
    let verb = lower.first()?.as_str();
    let buying = matches!(verb, "buy" | "purchase");
    if !buying && !matches!(verb, "add" | "put" | "throw" | "stick" | "pop") {
        return None;
    }
    let mut body: Vec<&str> = said[1..].to_vec();
    let mut shop_words: Vec<&str> = Vec::new();
    if !buying {
        let plain = |word: &str| word.to_lowercase().trim_matches(|c: char| !c.is_alphanumeric()).to_string();
        let at = body.iter().rposition(|word| {
            let word = plain(word);
            word == "cart" || word == "basket" || word == "bag"
        })?;
        let mut before = at;
        while before > 0 && !["my", "the", "our", "to", "in", "into"].contains(&plain(body[before - 1]).as_str()) {
            before -= 1;
        }
        // "my bag" is a shopping bag; "the bag" could be anything.
        let owner = before.checked_sub(1).map(|at| plain(body[at]));
        if plain(body[at]) == "bag" && owner.as_deref() != Some("my") {
            return None;
        }
        if owner.is_none() {
            return None;
        }
        shop_words = body[before..at].iter().copied().filter(|word| !word.eq_ignore_ascii_case("shopping")).collect();
        let mut end = before;
        while end > 0 && ["my", "the", "our", "to", "in", "into"].contains(&plain(body[end - 1]).as_str()) {
            end -= 1;
        }
        // Only a shop may follow the cart.
        let after = &body[at + 1..];
        if let Some((first, rest)) = after.split_first() {
            if !["at", "from", "on"].contains(&plain(first).as_str()) || rest.is_empty() || !shop_words.is_empty() {
                return None;
            }
            shop_words = rest.to_vec();
        }
        body.truncate(end);
    }
    if body.first().is_some_and(|word| word.eq_ignore_ascii_case("me")) {
        body.remove(0);
    }
    let url = body.iter().find(|word| word.starts_with("https://")).map(|word| word.to_string());
    if url.is_none() && shop_words.is_empty() {
        if let Some(at) = body.iter().rposition(|word| word.eq_ignore_ascii_case("from") || word.eq_ignore_ascii_case("at")) {
            if at > 0 && at + 1 < body.len() {
                shop_words = body[at + 1..].to_vec();
                body.truncate(at);
            }
        }
    }
    while body.first().is_some_and(|word| ["a", "an", "the", "some"].contains(&word.to_lowercase().as_str())) {
        body.remove(0);
    }
    let item = body.join(" ");
    let this_page = url.is_some() || item.is_empty() || ["this", "it", "that", "this item", "this one", "this product", "that one"].contains(&item.to_lowercase().as_str());
    // Buying names a shop or the page in front; "buy milk" is a chore, not a request.
    if buying && !this_page && shop_words.is_empty() {
        return None;
    }
    let mut out = Map::new();
    if let Some(url) = url {
        out.insert("url".into(), json!(url));
    } else if !this_page {
        out.insert("item".into(), json!(item));
    }
    if !shop_words.is_empty() {
        out.insert("shop".into(), json!(shop_words.join(" ")));
    }
    Some(out)
}

pub fn validate(args: &Value) -> Result<Map<String, Value>, String> {
    let args = args.as_object().ok_or("Arguments must be an object.")?;
    if let Some(extra) = args.keys().find(|key| !["url", "shop", "item", "browser", "price"].contains(&key.as_str())) {
        return Err(format!("{TOOL} doesn’t take “{extra}”."));
    }
    let mut out = Map::new();
    if let Some(url) = text(args, "url", 2000)? {
        out.insert("url".into(), json!(crate::research::validate_public_url(&url)?.to_string()));
    }
    for (key, max) in [("shop", 100), ("item", 200), ("price", 40)] {
        if let Some(value) = text(args, key, max)? {
            out.insert(key.into(), json!(value));
        }
    }
    if let Some(browser) = text(args, "browser", 100)? {
        if !BROWSERS.iter().any(|(bundle, _)| *bundle == browser) {
            return Err("That browser isn't supported.".into());
        }
        out.insert("browser".into(), json!(browser));
    }
    if out.contains_key("item") && !out.contains_key("url") && !out.contains_key("shop") {
        return Err("Which shop should I add it from?".into());
    }
    Ok(out)
}

/// Opening a page happens before the sheet, so a model may only aim it at what the user's
/// own words named. Anything it read could otherwise send it, and whatever else it read,
/// to any site in the user's signed-in browser. Nothing named means the page in front.
pub fn named_by_user(args: &Map<String, Value>, request: &str) -> bool {
    let get = |key: &str| args.get(key).and_then(Value::as_str);
    let url_said = get("url").is_none_or(|url| {
        request.split_whitespace().any(|word| {
            let word = word.trim_end_matches(['.', ',', '!', '?', ')']);
            word == url || crate::research::validate_public_url(word).is_ok_and(|said| Some(said.as_str()) == crate::research::validate_public_url(url).ok().as_ref().map(reqwest::Url::as_str))
        })
    });
    let request_lower = request.to_lowercase();
    let said = words(request);
    url_said
        && get("shop").is_none_or(|shop| request_lower.contains(&shop.to_lowercase()))
        && get("item").is_none_or(|item| words(item).iter().all(|word| said.contains(word)))
}

/// The browser may have been turned off since the sheet was shown.
pub fn permitted(settings: &crate::types::Settings, args: &Map<String, Value>) -> bool {
    let browser = args.get("browser").and_then(Value::as_str).unwrap_or_default();
    BROWSERS.iter().any(|(bundle, id)| *bundle == browser && crate::apps::allowed(settings, id))
}

pub fn describe(args: &Map<String, Value>) -> (String, Vec<Field>, String) {
    let get = |key: &str| args.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
    let shown = |key: &str, label: &str, value: String| Field { key: key.into(), label: label.into(), value, kind: "fixed" };
    let mut fields = vec![shown("shown_item", "Item", get("item")), shown("shown_shop", "Shop", shop_name(&get("url")))];
    if !get("price").is_empty() {
        fields.push(shown("shown_price", "Price", get("price")));
    }
    ("Add to your cart?".into(), fields, String::new())
}

fn receipt(outcome: ActionOutcome, message: String, cart: Option<String>, evidence: Value) -> ActionReceipt {
    ActionReceipt {
        error: (outcome != ActionOutcome::Succeeded).then(|| message.clone()),
        outcome,
        tool: TOOL.into(),
        object_id: cart.clone(),
        destination: cart,
        message,
        evidence,
    }
}

#[cfg(target_os = "macos")]
mod live {
    use super::*;
    use crate::ax::Element;
    use std::time::{Duration, Instant};

    const NODES: usize = 25_000;
    const PAGE_TEXT: usize = 60_000;

    pub fn browser_name(bundle: &str) -> &'static str {
        match bundle {
            "com.google.Chrome" => "Chrome",
            _ => "your browser",
        }
    }

    fn pause(seconds: f32, cancel: &AtomicBool) -> Result<(), String> {
        std::thread::sleep(Duration::from_secs_f32(seconds));
        if cancel.load(Ordering::SeqCst) {
            Err("Cancelled.".to_string())
        } else {
            Ok(())
        }
    }

    pub fn open(bundle: &str, url: &str) -> Result<(), String> {
        let status = std::process::Command::new("/usr/bin/open").args(["-b", bundle, url]).status();
        match status {
            Ok(done) if done.success() => Ok(()),
            _ => Err(format!("I couldn't open that page in {}.", browser_name(bundle))),
        }
    }

    fn web_area(window: &Element) -> Option<Element> {
        window.descendants(4000).into_iter().find(|element| element.text("AXRole") == "AXWebArea")
    }

    /// Every window's showing page, the main window first.
    fn areas(app: &Element) -> Vec<Element> {
        app.element("AXMainWindow")
            .or_else(|| app.element("AXFocusedWindow"))
            .into_iter()
            .chain(app.list("AXWindows"))
            .filter_map(|window| web_area(&window))
            .collect()
    }

    fn label_of(element: &Element) -> String {
        for name in ["AXTitle", "AXDescription"] {
            let value = element.text(name);
            if !value.trim().is_empty() {
                return value.trim().to_string();
            }
        }
        let parts: Vec<String> = element
            .descendants(40)
            .iter()
            .filter(|inner| inner.text("AXRole") == "AXStaticText")
            .map(|inner| inner.text("AXValue"))
            .collect();
        parts.join(" ").trim().to_string()
    }

    /// Depth first, so controls come in the order they sit on the page.
    fn read(area: &Element) -> (Page, Vec<Element>) {
        let mut page = Page { url: area.url("AXURL").unwrap_or_default(), title: area.text("AXTitle"), ..Page::default() };
        let mut elements = Vec::new();
        let mut stack = area.list("AXChildren");
        stack.reverse();
        let mut visited = 0;
        let deadline = Instant::now() + Duration::from_secs(12);
        while let Some(element) = stack.pop() {
            visited += 1;
            if visited > NODES || Instant::now() > deadline {
                break;
            }
            match element.text("AXRole").as_str() {
                "AXStaticText" if page.text.len() < PAGE_TEXT => {
                    page.text.push_str(&element.text("AXValue"));
                    page.text.push('\n');
                }
                "AXHeading" if page.heading.is_empty() && element.text("AXValue") == "1" => page.heading = label_of(&element),
                role @ ("AXButton" | "AXLink") => {
                    page.controls.push(Control {
                        link: role == "AXLink",
                        label: label_of(&element),
                        enabled: element.flag("AXEnabled").unwrap_or(true),
                        url: element.url("AXURL").unwrap_or_default(),
                    });
                    elements.push(element.clone());
                }
                _ => {}
            }
            let mut children = element.list("AXChildren");
            children.reverse();
            stack.extend(children);
        }
        (page, elements)
    }

    fn showing(app: &Element, wanted: impl Fn(&str) -> bool) -> Option<Element> {
        areas(app).into_iter().find(|area| area.url("AXURL").is_some_and(|url| wanted(&url)))
    }

    fn wait_for(app: &Element, seconds: u64, cancel: &AtomicBool, wanted: impl Fn(&str) -> bool) -> Result<Element, String> {
        let deadline = Instant::now() + Duration::from_secs(seconds);
        loop {
            // Asking again rebuilds the page view, so only while the browser is still starting.
            if app.list("AXWindows").is_empty() {
                app.ask_for_tree();
            }
            if let Some(area) = showing(app, &wanted) {
                return Ok(area);
            }
            if Instant::now() > deadline {
                return Err("The page didn't open.".into());
            }
            pause(0.5, cancel)?;
        }
    }

    fn app_for(bundle: &str) -> Result<Element, String> {
        if !crate::ax::trusted(true) {
            return Err("Allow Buntini under Privacy & Security → Accessibility, then ask again.".into());
        }
        // A browser that was closed takes a moment to start after the page is opened.
        for _ in 0..40 {
            if let Some(pid) = crate::ax::pid(bundle) {
                let app = Element::app(pid);
                // A browser whose Accessibility has gone wrong calls every window the whole
                // app. Nothing can be read or pressed until it restarts, so say that at once.
                let windows = app.list("AXWindows");
                if !windows.is_empty() && windows.iter().all(|window| window.text("AXRole") != "AXWindow") {
                    return Err(format!("{0} has stopped answering me. Quit and reopen {0}, then ask again.", browser_name(bundle)));
                }
                return Ok(app);
            }
            std::thread::sleep(Duration::from_millis(500));
        }
        Err(format!("{} didn't open.", browser_name(bundle)))
    }

    pub const CHECKED: &str = "wants to check you're a person. That tab is yours, and you can add it there.";

    /// Opens the page, or uses the one showing, and reads what's on it.
    pub fn look(bundle: &str, url: Option<&str>, search: Option<(&str, &str)>, cancel: &AtomicBool) -> Result<(String, String, String), String> {
        if url.is_some() || search.is_some() {
            let target = url.or(search.map(|(page, _)| page)).unwrap_or_default();
            open(bundle, target)?;
            // The browser builds its tree when asked, so it needs a moment after opening.
            pause(0.5, cancel)?;
        }
        let app = app_for(bundle)?;
        let target = url.or(search.map(|(page, _)| page)).map(str::to_string);
        let area = match &target {
            Some(target) => wait_for(&app, 30, cancel, |seen| same_shop(seen, target))?,
            None => areas(&app).into_iter().next().ok_or_else(|| format!("I can't see a page in {}.", browser_name(bundle)))?,
        };
        let shop = area.url("AXURL").map(|seen| shop_name(&seen)).unwrap_or_default();
        if let Some((results, item)) = search {
            let deadline = Instant::now() + Duration::from_secs(20);
            let mut result = None;
            while Instant::now() < deadline {
                if let Some(area) = showing(&app, |seen| same_shop(seen, results)) {
                    let (page, elements) = read(&area);
                    if human_check(&page) {
                        return Err(format!("{shop} {CHECKED}"));
                    }
                    if let Some(at) = result_link(&page, item) {
                        result = Some(page.controls[at].url.clone());
                        elements[at].press();
                        break;
                    }
                }
                pause(1.0, cancel)?;
            }
            let Some(result) = result else {
                return Err(format!("I couldn't find {item} at {shop}. The search is open in {}.", browser_name(bundle)));
            };
            // A results page must never be taken for the product: its first Add to Cart
            // would be whatever the shop ranked first.
            if wait_for(&app, 8, cancel, |seen| same_shop(seen, results) && !same_path(seen, results)).is_err() {
                open(bundle, &result)?;
                wait_for(&app, 15, cancel, |seen| same_page(seen, &result)).map_err(|_| format!("I couldn't open {item} at {shop}."))?;
            }
            pause(2.0, cancel)?;
        }
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let area = match &target {
                Some(target) => wait_for(&app, 15, cancel, |seen| same_shop(seen, target) && !search.is_some_and(|(results, _)| same_path(seen, results)))?,
                None => area.clone(),
            };
            let (page, _) = read(&area);
            if human_check(&page) {
                return Err(format!("{shop} {CHECKED}"));
            }
            let name = product_name(&page);
            let pick = pick_add(&page.controls, &name);
            if matches!(pick, Pick::Press(_)) {
                return Ok((page.url, name, page.text));
            }
            if Instant::now() > deadline {
                return Err(match pick {
                    Pick::Unavailable => format!("{name} needs an option chosen at {shop}, or it's out of stock. The page is open."),
                    _ => format!("I can't find an Add to Cart button on that {shop} page. It's open if you want a look."),
                });
            }
            pause(1.0, cancel)?;
        }
    }

    pub fn add(bundle: &str, url: &str, item: &str, cancel: &AtomicBool) -> ActionReceipt {
        let shop = shop_name(url);
        let failed = |message: String| receipt(ActionOutcome::Failed, message, None, json!({}));
        let app = match app_for(bundle) {
            Ok(app) => app,
            Err(error) => return failed(error),
        };
        if showing(&app, |seen| same_page(seen, url)).is_none() {
            if let Err(error) = open(bundle, url) {
                return failed(error);
            }
        }
        let area = match wait_for(&app, 30, cancel, |seen| same_page(seen, url)) {
            Ok(area) => area,
            Err(error) => return failed(error),
        };
        let deadline = Instant::now() + Duration::from_secs(20);
        let (press, count_before) = loop {
            let (page, elements) = read(&area);
            if human_check(&page) {
                return failed(format!("{shop} {CHECKED}"));
            }
            // A page can change after the sheet named it.
            if !names(&product_name(&page), item) {
                return failed(format!("That {shop} page no longer shows {item}, so I didn't add anything. The page is open."));
            }
            match pick_add(&page.controls, item) {
                Pick::Press(at) => break (elements[at].clone(), cart_count(&page.controls)),
                _ if Instant::now() < deadline => {
                    if pause(1.0, cancel).is_err() {
                        return failed("Cancelled before anything was added.".into());
                    }
                }
                Pick::Unavailable => return failed(format!("{item} needs an option chosen at {shop}, or it's out of stock. The page is open.")),
                Pick::Missing => return failed(format!("The Add to Cart button on that {shop} page has gone. The page is open.")),
            }
        };
        if !press.press() {
            return failed(format!("The Add to Cart button at {shop} didn't respond. The page is open."));
        }
        // From here the item may be in the cart, so nothing below is ever retried.
        let unsure = |detail: String, cart: Option<String>| receipt(ActionOutcome::Unknown, detail, cart, json!({"verification":"not_read_back"}));
        std::thread::sleep(Duration::from_secs(3));
        let cart = (|| {
            let area = showing(&app, |seen| same_shop(seen, url))?;
            let (page, elements) = read(&area);
            if human_check(&page) {
                return None;
            }
            let at = cart_link(&page)?;
            elements[at].press().then(|| page.controls[at].url.clone())
        })();
        // A pop-up can swallow the press, so the cart gets opened directly if it doesn't come.
        let arrived = cart.is_some() && wait_for(&app, 8, cancel, |seen| same_shop(seen, url) && cart_path(seen)).is_ok();
        if !arrived {
            let page = cart.unwrap_or_else(|| format!("https://{}/cart", host_of(url)));
            if open(bundle, &page).is_err() {
                return unsure(format!("I pressed Add to Cart at {shop} but couldn't open the cart to check."), None);
            }
        }
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut last = CartState::Unclear;
        let mut cart_url = None;
        while Instant::now() < deadline {
            if let Some(area) = showing(&app, |seen| same_shop(seen, url) && cart_path(seen)) {
                let (page, _) = read(&area);
                cart_url = Some(page.url.clone());
                if human_check(&page) {
                    return unsure(format!("I pressed Add to Cart, but {shop} {CHECKED}"), cart_url);
                }
                last = cart_state(&page, item);
                if last == CartState::Holds && !press_took(count_before, cart_count(&page.controls)) {
                    last = CartState::Unclear;
                }
                if last == CartState::Holds {
                    return receipt(
                        ActionOutcome::Succeeded,
                        format!("{item} is in your {shop} cart, open in {}. Checkout is yours.", browser_name(bundle)),
                        cart_url,
                        json!({"verification":"cart_read_back", "url":url, "item":item}),
                    );
                }
            }
            std::thread::sleep(Duration::from_secs(1));
        }
        let detail = match last {
            CartState::Empty => format!("I pressed Add to Cart but your {shop} cart looks empty. It may need a size or colour chosen first."),
            _ => format!("I pressed Add to Cart at {shop} but couldn't see {item} in the cart."),
        };
        unsure(format!("{detail} Check the cart before trying again."), cart_url)
    }

    /// Test only: takes the item back out of the cart showing in this browser.
    #[cfg(test)]
    pub fn remove(bundle: &str, url: &str, item: &str) -> bool {
        let Ok(app) = app_for(bundle) else { return false };
        let Some(area) = showing(&app, |seen| same_shop(seen, url) && cart_path(seen)) else { return false };
        let (page, elements) = read(&area);
        let found = page.controls.iter().position(|control| {
            let lower = control.label.to_lowercase();
            lower.starts_with("remove") && (overlap(&control.label, item) >= 0.5 || lower == "remove")
        });
        let Some(at) = found else { return false };
        let pressed = elements[at].press();
        std::thread::sleep(Duration::from_secs(6));
        pressed
    }
}

/// Opens or reads the product before the sheet, so the sheet says what was found.
#[cfg(target_os = "macos")]
pub async fn look(settings: &crate::types::Settings, args: &Map<String, Value>, cancel: Arc<AtomicBool>) -> Result<Map<String, Value>, String> {
    let browser = crate::browser_session::selected_browser()?;
    let bundle = browser.bundle_id.clone();
    let id = BROWSERS.iter().find(|(known, _)| *known == bundle).map(|(_, id)| *id).ok_or_else(|| format!("I can't add to a cart in {} yet, only in Chrome.", browser.name))?;
    if !crate::apps::allowed(settings, id) {
        return Err(format!("Allow {} in Settings → Apps, then ask again.", browser.name));
    }
    let url = args.get("url").and_then(Value::as_str).map(str::to_string);
    if url.is_none() && !args.contains_key("shop") {
        // "This" is whatever is in front, and a private window isn't Buntini's to read.
        let app = crate::apps::app(id).ok_or("That browser isn't available.")?;
        crate::apps::ensure_public_window(app).await?;
    }
    let wanted = args.get("item").and_then(Value::as_str).map(str::to_string);
    let results = match (&url, args.get("shop").and_then(Value::as_str), &wanted) {
        (None, Some(shop), Some(item)) => Some(search_url(shop, item)?),
        (None, Some(shop), None) => return Err(format!("What should I add from {shop}?")),
        _ => None,
    };
    let (found_url, name, page_text) = {
        let (bundle, url, results, wanted, cancel) = (bundle.clone(), url.clone(), results.clone(), wanted.clone(), cancel.clone());
        tokio::task::spawn_blocking(move || {
            let search = results.as_deref().zip(wanted.as_deref());
            live::look(&bundle, url.as_deref(), search, &cancel)
        })
        .await
        .map_err(|_| "Reading the page ended unexpectedly.".to_string())??
    };
    let mut out = Map::new();
    out.insert("url".into(), json!(crate::research::validate_public_url(&found_url)?.to_string()));
    out.insert("item".into(), json!(if name.is_empty() { wanted.unwrap_or_else(|| shop_name(&found_url)) } else { name }));
    out.insert("browser".into(), json!(bundle));
    if let Some(price) = published_price(&found_url).await.filter(|price| price_on_page(price, &page_text)) {
        out.insert("price".into(), json!(price));
    }
    Ok(out)
}

#[cfg(not(target_os = "macos"))]
pub async fn look(_: &crate::types::Settings, _: &Map<String, Value>, _: Arc<AtomicBool>) -> Result<Map<String, Value>, String> {
    Err("Adding to a cart needs macOS.".into())
}

/// The published price is fetched apart from the browser, so it only counts if the page
/// the user will see shows it too.
fn price_on_page(price: &str, text: &str) -> bool {
    let number: String = price.chars().filter(|c| c.is_ascii_digit() || *c == '.' || *c == ',').collect();
    !number.is_empty() && (text.contains(&number) || number.strip_suffix(".00").is_some_and(|whole| text.contains(whole)))
}

/// The one price the shop publishes for this page, if it publishes exactly one.
async fn published_price(url: &str) -> Option<String> {
    let page = tokio::time::timeout(std::time::Duration::from_secs(8), crate::research::fetch_watched_html(url)).await.ok()?.ok()?;
    let offers = crate::pricing::extract_offers(&page.html, &page.url).ok()?;
    match offers.as_slice() {
        [offer] if !offer.is_aggregate => Some(crate::watches::money(&offer.price, &offer.currency)),
        _ => None,
    }
}

pub async fn execute(args: &Map<String, Value>, cancel: Arc<AtomicBool>) -> ActionReceipt {
    let get = |key: &str| args.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
    let (url, item, bundle) = (get("url"), get("item"), get("browser"));
    if url.is_empty() || item.is_empty() || bundle.is_empty() {
        return receipt(ActionOutcome::Failed, "That cart request is missing its page. Ask again.".into(), None, json!({}));
    }
    #[cfg(target_os = "macos")]
    {
        tokio::task::spawn_blocking(move || live::add(&bundle, &url, &item, &cancel))
            .await
            .unwrap_or_else(|_| receipt(ActionOutcome::Unknown, "Adding to the cart ended unexpectedly. Check the cart before trying again.".into(), None, json!({})))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (url, item, bundle, cancel);
        receipt(ActionOutcome::Failed, "Adding to a cart needs macOS.".into(), None, json!({}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn button(label: &str, enabled: bool) -> Control {
        Control { link: false, label: label.into(), enabled, url: String::new() }
    }
    fn link(label: &str, url: &str) -> Control {
        Control { link: true, label: label.into(), enabled: true, url: url.into() }
    }

    #[test]
    fn a_disabled_button_stops_rather_than_pressing_a_recommendation() {
        // What Target's AirPods Pro 2 page showed.
        let controls = [
            button("Add to cart Apple AirPods Pro 2 Wireless Earbuds with Active Noise Cancellation", false),
            button("Add to cart for Apple AirPods 4 Wireless Earbuds", true),
        ];
        assert_eq!(pick_add(&controls, "Apple AirPods Pro 2 Wireless Earbuds with Active Noise Cancellation"), Pick::Unavailable);
    }

    #[test]
    fn only_this_items_button_is_pressed() {
        let controls = [button("Add to cart for Bose QuietComfort Ultra", true), button("ADD TO CART", true), button("Add to cart", true)];
        assert_eq!(pick_add(&controls, "MX Master 3S"), Pick::Press(1));
        assert_eq!(pick_add(&controls[..1], "MX Master 3S"), Pick::Missing);
        assert_eq!(pick_add(&[button("Add to Cart - $129.00", true)], "Keyboard"), Pick::Press(0));
    }

    #[test]
    fn checkout_and_payment_are_never_an_add_button() {
        for label in ["Buy now", "Checkout", "Add to cart and check out", "Buy with Shop Pay", "Add to wishlist", "Pay with PayPal", "Save for later", "Subscribe & add to cart"] {
            assert_eq!(pick_add(&[button(label, true)], "Thing"), Pick::Missing, "{label}");
        }
    }

    #[test]
    fn recommendations_do_not_prove_the_item_is_in_the_cart() {
        // Target's empty cart recommends the same AirPods with an Add button.
        let empty = Page {
            controls: vec![link("cart 0 items", "https://www.target.com/cart"), link("Apple AirPods 4 Wireless Earbuds", "https://www.target.com/p/a"), button("Add to cart for Apple AirPods 4 Wireless Earbuds", true)],
            ..Page::default()
        };
        assert_eq!(cart_state(&empty, "Apple AirPods 4 Wireless Earbuds"), CartState::Empty);
        let holds = Page { controls: vec![button("Remove Apple AirPods 4 Wireless Earbuds from cart", true)], ..Page::default() };
        assert_eq!(cart_state(&holds, "Apple AirPods 4 Wireless Earbuds"), CartState::Holds);
        let logitech = Page { controls: vec![button("remove MX Master 3S Bluetooth Edition", true)], ..Page::default() };
        assert_eq!(cart_state(&logitech, "MX Master 3S"), CartState::Holds);
        // Keychron: plain Remove buttons beside the item's link.
        let keychron = Page { controls: vec![link("Keychron Q1 Max QMK/VIA Wireless Custom Mechanical Keyboard", "https://www.keychron.com/products/q1"), link("Remove", "https://www.keychron.com/cart/change")], ..Page::default() };
        assert_eq!(cart_state(&keychron, "Keychron Q1 Max QMK/VIA Wireless Custom Mechanical Keyboard"), CartState::Holds);
        assert_eq!(cart_state(&Page::default(), "Thing"), CartState::Unclear);
    }

    #[test]
    fn something_else_in_the_cart_is_not_this_item() {
        // A cart holding another thing, with this item recommended further down.
        let page = Page {
            controls: vec![
                link("USB-C Cable", "https://shop.example/p/cable"),
                button("Remove", true),
                link("Apple AirPods Pro 2 Wireless Earbuds", "https://shop.example/p/pro"),
            ],
            ..Page::default()
        };
        assert_eq!(cart_state(&page, "Apple AirPods Pro 2 Wireless Earbuds"), CartState::Unclear);
        // The same family with a different number is a different product.
        let other = Page { controls: vec![button("Remove Apple AirPods 4 Wireless Earbuds from cart", true)], ..Page::default() };
        assert_eq!(cart_state(&other, "Apple AirPods Pro 2 Wireless Earbuds"), CartState::Unclear);
    }

    #[test]
    fn the_shops_own_count_can_overrule_a_row_that_was_already_there() {
        let cart = |label: &str| vec![link(label, "https://shop.example/cart")];
        assert_eq!(cart_count(&cart("cart 0 items")), Some(0));
        assert_eq!(cart_count(&cart("Cart 1 item")), Some(1));
        assert_eq!(cart_count(&cart("cart, 12 items")), Some(12));
        assert_eq!(cart_count(&cart("Cart")), None);
        assert!(press_took(Some(0), Some(1)));
        assert!(press_took(None, Some(1)));
        assert!(press_took(Some(1), None));
        assert!(!press_took(Some(1), Some(1)));
    }

    #[test]
    fn only_what_the_user_named_can_open_before_the_sheet() {
        let args = |value: Value| value.as_object().unwrap().clone();
        let asked = "add AirPods 4 to my Target cart";
        assert!(named_by_user(&args(json!({"shop":"Target","item":"AirPods 4"})), asked));
        assert!(named_by_user(&args(json!({})), "add this to my cart"));
        assert!(!named_by_user(&args(json!({"shop":"evil.example","item":"AirPods 4"})), asked));
        assert!(!named_by_user(&args(json!({"shop":"Target","item":"AirPods door code 4471"})), asked));
        assert!(!named_by_user(&args(json!({"url":"https://evil.example/p?d=secret"})), "Check Ana's email and add the keyboard to my cart"));
        assert!(named_by_user(&args(json!({"url":"https://www.keychron.com/products/q1"})), "add https://www.keychron.com/products/q1, thanks"));
    }

    #[test]
    fn the_approved_page_includes_its_variant() {
        assert!(same_page("https://shop.example/p?id=1", "https://shop.example/p?id=1"));
        assert!(!same_page("https://shop.example/p?id=2", "https://shop.example/p?id=1"));
        assert!(same_path("https://shop.example/p?id=2", "https://shop.example/p?id=1"));
    }

    #[test]
    fn a_published_price_must_also_be_on_the_page() {
        assert!(price_on_page("$129.00", "Now $129"));
        assert!(price_on_page("$1,299.50", "Price $1,299.50 today"));
        assert!(!price_on_page("$9.99", "Now $999.00"));
    }

    #[test]
    fn numbers_tell_products_apart() {
        assert!(names("Sony - WH-1000XM5 Wireless Noise Cancelling Over-the-Ear Headphones - Black", "Sony WH 1000XM5 Wireless Noise Cancelling Over the Ear Headphones Black"));
        assert!(names("remove MX Master 3S Bluetooth Edition", "MX Master 3s Wireless Mouse"));
        assert!(!names("Apple AirPods 4 Wireless Earbuds", "Apple AirPods Pro 2 Wireless Earbuds"));
    }

    #[test]
    fn a_human_check_is_noticed_but_ordinary_pages_are_not_one() {
        let page = |title: &str, text: &str| Page { title: title.into(), text: text.into(), ..Page::default() };
        assert!(human_check(&page("Just a moment...", "")));
        assert!(human_check(&page("Walmart", "Press & Hold to confirm you are a human")));
        assert!(!human_check(&page("Robot vacuum", "This site is protected by reCAPTCHA. Just a moment of your time.")));
    }

    #[test]
    fn the_cart_link_stays_on_this_shop_and_short_of_checkout() {
        let page = |controls| Page { url: "https://www.keychron.com/products/q1".into(), controls, ..Page::default() };
        assert_eq!(cart_link(&page(vec![link("Checkout", "https://www.keychron.com/cart/checkout"), link("Cart", "https://www.keychron.com/cart")])), Some(1));
        assert_eq!(cart_link(&page(vec![link("Cart", "https://evil.example/cart")])), None);
        // Links that change the cart or skip to buying, whatever they call themselves.
        for url in ["https://www.keychron.com/cart/change?line=1&quantity=0", "https://www.keychron.com/cart/clear", "https://www.keychron.com/cart/12345:1", "https://www.keychron.com/cart?add=1"] {
            assert_eq!(cart_link(&page(vec![link("Cart", url)])), None, "{url}");
        }
        assert_eq!(cart_link(&page(vec![link("Remove", "https://www.keychron.com/cart")])), None);
        assert_eq!(cart_link(&page(vec![link("Cart", "http://www.keychron.com/cart")])), None);
    }

    #[test]
    fn a_search_result_must_name_the_item_on_this_shop() {
        let page = Page {
            url: "https://www.target.com/s?searchTerm=airpods".into(),
            controls: vec![
                link("Cart", "https://www.target.com/cart"),
                link("Apple AirPods Pro 3", "https://www.target.com/p/pro-3"),
                link("Apple AirPods 4 Wireless Earbuds", "https://elsewhere.example/p/4"),
                link("Apple AirPods 4 Wireless Earbuds", "https://www.target.com/p/4"),
            ],
            ..Page::default()
        };
        assert_eq!(result_link(&page, "AirPods 4"), Some(3));
        assert_eq!(result_link(&page, "AirPods Max"), None);
    }

    #[test]
    fn the_plainest_result_wins_over_a_combo_or_an_accessory() {
        let page = Page {
            url: "https://www.newegg.com/p/pl?d=mx".into(),
            controls: vec![
                link("Logitech MX Keys S Combo: MX Master 3S, MX Keys S & MX Palm Rest", "https://www.newegg.com/combo/p/1"),
                link("Case for Logitech MX Master 3S", "https://www.newegg.com/case/p/2"),
                link("Logitech MX Master 3S Wireless Mouse", "https://www.newegg.com/mouse/p/3"),
                link("Logitech MX Master 3S", "https://www.newegg.com/p/pl"),
            ],
            ..Page::default()
        };
        assert_eq!(result_link(&page, "Logitech MX Master 3S"), Some(2));
    }

    #[test]
    fn shops_are_never_guessed_from_a_name() {
        assert_eq!(search_url("Target", "AirPods 4").unwrap(), "https://www.target.com/s?searchTerm=AirPods+4");
        assert!(search_url("Keychron", "Q1 & Max").unwrap().starts_with("https://www.keychron.com/search?q=Q1+%26+Max"));
        assert!(search_url("shop.example.com", "Thing").unwrap().starts_with("https://shop.example.com/search?q=Thing"));
        assert!(search_url("Some Little Shop", "Thing").unwrap_err().contains("link"));
        assert!(search_url("localhost.", "Thing").is_err());
    }

    #[test]
    fn the_sheet_names_what_was_found_and_cannot_aim_it_elsewhere() {
        let args = validate(&json!({"url":"https://www.target.com/p/a","item":"AirPods 4","browser":"com.google.Chrome","price":"$129.00"})).unwrap();
        let (_, fields, note) = describe(&args);
        assert!(fields.iter().all(|field| field.kind == "fixed"));
        assert_eq!(fields.iter().map(|field| field.value.as_str()).collect::<Vec<_>>(), ["AirPods 4", "target.com", "$129.00"]);
        assert!(note.is_empty());
        let edited = crate::actions::keep_fixed(TOOL, &json!(args), &json!({"url":"https://evil.example/x","item":"Other"}));
        assert_eq!(edited["url"], "https://www.target.com/p/a");
        assert_eq!(edited["item"], "AirPods 4");
    }

    #[test]
    fn plain_requests_are_read_without_a_model() {
        let read = |prompt: &str| parse_request(prompt).map(|args| {
            let get = |key: &str| args.get(key).and_then(Value::as_str).unwrap_or("").to_string();
            (get("item"), get("shop"), get("url"))
        });
        let named = |item: &str, shop: &str| Some((item.to_string(), shop.to_string(), String::new()));
        assert_eq!(read("add AirPods 4 to my Target cart"), named("AirPods 4", "Target"));
        assert_eq!(read("Please add the Sony WH-1000XM5 to my cart at Best Buy."), named("Sony WH-1000XM5", "Best Buy"));
        assert_eq!(read("can you put AirPods 4 from Target in my cart?"), named("AirPods 4", "Target"));
        assert_eq!(read("add a mechanical keyboard to my basket at Best Buy"), named("mechanical keyboard", "Best Buy"));
        assert_eq!(read("add a keyboard to my cart"), named("keyboard", ""));
        assert_eq!(read("buy me AirPods 4 from Target"), named("AirPods 4", "Target"));
        assert_eq!(read("add this to my cart"), named("", ""));
        assert_eq!(read("add it to cart"), named("", ""));
        assert_eq!(read("buy this"), named("", ""));
        assert_eq!(read("put https://www.keychron.com/products/q1 in my cart"), Some((String::new(), String::new(), "https://www.keychron.com/products/q1".to_string())));
        assert_eq!(read("add https://shop.example.com/p/1 to my bag"), Some((String::new(), String::new(), "https://shop.example.com/p/1".to_string())));
    }

    #[test]
    fn other_requests_stay_with_the_model() {
        for prompt in [
            "add milk to my shopping list",
            "how do I add something to my cart",
            "don't add this to my cart",
            "what's in my cart",
            "add AirPods to my cart and remind me tomorrow",
            "add a reminder to check my cart",
            "buy milk",
            "remind me to buy milk from Target",
            "put my bag in the car",
            "search for golf cart deals",
        ] {
            assert_eq!(parse_request(prompt), None, "{prompt}");
        }
    }

    #[test]
    fn requests_are_checked_before_anything_opens() {
        assert!(validate(&json!({"item":"AirPods"})).unwrap_err().contains("Which shop"));
        assert!(validate(&json!({"url":"http://example.com"})).is_err());
        assert!(validate(&json!({"url":"https://example.com","quantity":3})).is_err());
        assert!(validate(&json!({"url":"https://example.com","browser":"org.mozilla.firefox"})).is_err());
        assert!(validate(&json!({})).unwrap().is_empty());
    }

    /// `BUNTINI_CART_ITEMS='url|url'` in the browser in front or the default one. Presses
    /// Add to Cart for real, then takes the item back out. Leaves the cart tab open.
    #[cfg(target_os = "macos")]
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "uses the real browser and real shops"]
    async fn live_add_to_cart() {
        let urls = std::env::var("BUNTINI_CART_ITEMS").expect("BUNTINI_CART_ITEMS");
        let bundle = std::env::var("BUNTINI_CART_BROWSER").unwrap_or_else(|_| "com.google.Chrome".into());
        let look_only = std::env::var("BUNTINI_CART_LOOK_ONLY").is_ok();
        for url in urls.split('|') {
            let cancel = Arc::new(AtomicBool::new(false));
            let found = {
                // "Shop=item" finds it through the shop's search.
                let search = url.split_once('=').filter(|_| !url.starts_with("https://")).map(|(shop, item)| (search_url(shop, item).unwrap(), item.to_string()));
                let (bundle, url, cancel) = (bundle.clone(), url.to_string(), cancel.clone());
                tokio::task::spawn_blocking(move || match &search {
                    Some((page, item)) => live::look(&bundle, None, Some((page, item)), &cancel),
                    None => live::look(&bundle, Some(&url), None, &cancel),
                }).await.unwrap()
            };
            let Ok((page, item, _)) = found else {
                println!("{url}\n  look: {}", found.unwrap_err());
                continue;
            };
            println!("{url}\n  found: {item} at {page}; price {:?}", published_price(&page).await);
            if look_only {
                continue;
            }
            let done = execute(&Map::from_iter([("url".into(), json!(page.clone())), ("item".into(), json!(item.clone())), ("browser".into(), json!(bundle.clone()))]), cancel).await;
            println!("  {:?}: {}\n  evidence {}", done.outcome, done.message, done.evidence);
            let (bundle, cleanup, name) = (bundle.clone(), page.clone(), item.clone());
            let removed = tokio::task::spawn_blocking(move || live::remove(&bundle, &cleanup, &name)).await.unwrap();
            println!("  removed: {removed}");
        }
    }
}
