//! Extract prices from published Schema.org JSON-LD and, for a page that publishes none,
//! from what its rendered page plainly shows. Never inferred discounts or model output.

use crate::research::{label, validate_public_url, MAX_HTML_BYTES};
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};

const MAX_SCRIPTS: usize = 32;
const MAX_SCRIPT_BYTES: usize = 262_144;
const MAX_JSON_NODES: usize = 4096;
const MAX_OFFERS: usize = 40;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PriceOffer {
    pub product: String,
    /// The evidence page, not a checkout URL or an automatically followed link.
    pub url: String,
    /// Canonical positive decimal strings avoid float rounding in watches.
    pub price: String,
    pub currency: String,
    pub availability: Option<String>,
    pub seller: Option<String>,
    /// An AggregateOffer is explicitly a published range, not a merchant quote.
    pub is_aggregate: bool,
    pub high_price: Option<String>,
}

#[derive(Clone, Copy)]
struct Node<'a> {
    value: &'a Value,
    schema: bool,
}

fn field<'a>(object: &'a Map<String, Value>, name: &str) -> Option<&'a Value> {
    object
        .get(name)
        .or_else(|| object.get(&format!("https://schema.org/{name}")))
        .or_else(|| object.get(&format!("http://schema.org/{name}")))
}

fn schema_url(value: &str) -> bool {
    matches!(
        value.trim_end_matches('/'),
        "http://schema.org" | "https://schema.org"
    )
}

fn schema_context(context: &Value) -> bool {
    match context {
        Value::String(value) => schema_url(value),
        Value::Array(values) => values.iter().any(schema_context),
        Value::Object(value) => value
            .get("@vocab")
            .and_then(Value::as_str)
            .is_some_and(schema_url),
        _ => false,
    }
}

fn typed(node: Node<'_>, expected: &str) -> bool {
    let Some(kind) = node.value.get("@type") else {
        return false;
    };
    let matches = |value: &str| {
        (node.schema && value == expected)
            || value == format!("https://schema.org/{expected}")
            || value == format!("http://schema.org/{expected}")
    };
    match kind {
        Value::String(value) => matches(value),
        Value::Array(values) => values.iter().filter_map(Value::as_str).any(matches),
        _ => false,
    }
}

fn collect<'a>(
    value: &'a Value,
    inherited: bool,
    depth: usize,
    budget: &mut usize,
    nodes: &mut Vec<Node<'a>>,
) -> Result<(), String> {
    if depth > 32 || *budget == 0 {
        return Err("The page's structured product data exceeds the safe parsing limit.".into());
    }
    *budget -= 1;
    match value {
        Value::Object(object) => {
            let schema = object
                .get("@context")
                .map(schema_context)
                .unwrap_or(inherited);
            nodes.push(Node { value, schema });
            for (key, child) in object {
                if key != "@context" {
                    collect(child, schema, depth + 1, budget, nodes)?;
                }
            }
        }
        Value::Array(values) => {
            for child in values {
                collect(child, inherited, depth + 1, budget, nodes)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Strict ISO 4217 alphabetic currency validation. No inferred symbol/currency
/// conversion is performed. Unknown and non-currency codes fail closed.
fn currency(value: Option<&Value>) -> Option<String> {
    const CODES: &str = "AED AFN ALL AMD ANG AOA ARS AUD AWG AZN BAM BBD BDT BGN BHD BIF BMD BND BOB BOV BRL BSD BTN BWP BYN BZD CAD CDF CHE CHF CHW CLF CLP CNY COP COU CRC CUP CVE CZK DJF DKK DOP DZD EGP ERN ETB EUR FJD FKP GBP GEL GHS GIP GMD GNF GTQ GYD HKD HNL HTG HUF IDR ILS INR IQD IRR ISK JMD JOD JPY KES KGS KHR KMF KPW KRW KWD KYD KZT LAK LBP LKR LRD LSL LYD MAD MDL MGA MKD MMK MNT MOP MRU MUR MVR MWK MXN MXV MYR MZN NAD NGN NIO NOK NPR NZD OMR PAB PEN PGK PHP PKR PLN PYG QAR RON RSD RUB RWF SAR SBD SCR SDG SEK SGD SHP SLE SOS SRD SSP STN SVC SYP SZL THB TJS TMT TND TOP TRY TTD TWD TZS UAH UGX USD USN UYI UYU UYW UZS VED VES VND VUV WST XAF XCD XCG XDR XOF XPF XSU XUA YER ZAR ZMW ZWG";
    let raw = value?.as_str()?;
    if raw.len() != 3
        || !raw.bytes().all(|c| c.is_ascii_uppercase())
        || !CODES.split_ascii_whitespace().any(|code| code == raw)
    {
        return None;
    }
    Some(raw.into())
}

fn decimal(value: Option<&Value>) -> Option<String> {
    let raw = match value? {
        Value::String(value) => value.trim().to_string(),
        Value::Number(value) => value.to_string(),
        _ => return None,
    };
    if raw.is_empty() || raw.len() > 26 {
        return None;
    }
    let mut parts = raw.split('.');
    let whole = parts.next()?;
    let fraction = parts.next();
    if parts.next().is_some()
        || whole.is_empty()
        || whole.len() > 18
        || !whole.bytes().all(|c| c.is_ascii_digit())
    {
        return None;
    }
    if fraction.is_some_and(|part| {
        part.is_empty() || part.len() > 6 || !part.bytes().all(|c| c.is_ascii_digit())
    }) {
        return None;
    }
    let whole = whole.trim_start_matches('0');
    let whole = if whole.is_empty() { "0" } else { whole };
    let fraction = fraction.unwrap_or("").trim_end_matches('0');
    if whole == "0" && fraction.is_empty() {
        return None;
    }
    Some(if fraction.is_empty() {
        whole.into()
    } else {
        format!("{whole}.{fraction}")
    })
}

pub fn canonical_decimal(value: &str) -> Option<String> {
    decimal(Some(&Value::String(value.into())))
}

/// Validate and compare canonical monetary decimals without binary floats.
/// Invalid operands fail closed, even for direct controller callers.
pub fn decimal_lte(a: &str, b: &str) -> bool {
    let (Some(a), Some(b)) = (canonical_decimal(a), canonical_decimal(b)) else {
        return false;
    };
    let (a, b) = (a.as_str(), b.as_str());
    let (ai, af) = a.split_once('.').unwrap_or((a, ""));
    let (bi, bf) = b.split_once('.').unwrap_or((b, ""));
    match ai.len().cmp(&bi.len()).then_with(|| ai.cmp(bi)) {
        std::cmp::Ordering::Less => true,
        std::cmp::Ordering::Greater => false,
        std::cmp::Ordering::Equal => format!("{af:0<6}") <= format!("{bf:0<6}"),
    }
}

fn text_property(object: &Map<String, Value>, key: &str, max: usize) -> Option<String> {
    field(object, key)
        .and_then(Value::as_str)
        .map(|text| label(text, max))
        .filter(|text| !text.is_empty())
}

fn resolve<'a>(node: Node<'a>, ids: &HashMap<&'a str, Node<'a>>) -> Node<'a> {
    if let Some(id) = node.value.get("@id").and_then(Value::as_str) {
        if let Some(found) = ids.get(id) {
            // Prefer the object carrying actual offer properties over an @id
            // stub. References never cause a network request or remote context.
            if node
                .value
                .as_object()
                .is_some_and(|object| object.len() <= 2)
            {
                return *found;
            }
        }
    }
    node
}

struct Extractor<'a> {
    ids: HashMap<&'a str, Node<'a>>,
    visited: HashSet<usize>,
    offers: Vec<PriceOffer>,
    source_url: String,
}

impl<'a> Extractor<'a> {
    fn skip_nested_offers(&mut self, node: Node<'a>, depth: usize) {
        if depth > 16 {
            return;
        }
        if let Some(values) = node.value.as_array() {
            for value in values {
                self.skip_nested_offers(
                    Node {
                        value,
                        schema: node.schema,
                    },
                    depth + 1,
                );
            }
            return;
        }
        let node = resolve(node, &self.ids);
        if !self.visited.insert(node.value as *const Value as usize) {
            return;
        }
        if let Some(children) = node
            .value
            .as_object()
            .and_then(|object| field(object, "offers"))
        {
            self.skip_nested_offers(
                Node {
                    value: children,
                    schema: node.schema,
                },
                depth + 1,
            );
        }
    }

    fn offers(&mut self, node: Node<'a>, product: &str, depth: usize) {
        if depth > 16 || self.offers.len() >= MAX_OFFERS {
            return;
        }
        if let Some(values) = node.value.as_array() {
            for value in values {
                self.offers(
                    Node {
                        value,
                        schema: node.schema,
                    },
                    product,
                    depth + 1,
                );
            }
            return;
        }
        let mut node = resolve(node, &self.ids);
        if let Some(context) = node.value.get("@context") {
            node.schema = schema_context(context);
        }
        if !self.visited.insert(node.value as *const Value as usize) {
            return;
        }
        let Some(object) = node.value.as_object() else {
            return;
        };
        let aggregate = typed(node, "AggregateOffer");
        if !aggregate && !typed(node, "Offer") {
            return;
        }
        if let Some(function) = field(object, "businessFunction").and_then(Value::as_str) {
            if !matches!(
                function,
                "Sell"
                    | "http://purl.org/goodrelations/v1#Sell"
                    | "https://purl.org/goodrelations/v1#Sell"
            ) {
                return;
            }
        }
        let specification = field(object, "priceSpecification").and_then(Value::as_object);
        let direct_price = field(object, if aggregate { "lowPrice" } else { "price" });
        let specified_price = specification.and_then(|spec| field(spec, "price"));
        let Some(price) = decimal(direct_price.or(specified_price)) else {
            if aggregate {
                if let Some(children) = field(object, "offers") {
                    self.offers(
                        Node {
                            value: children,
                            schema: node.schema,
                        },
                        product,
                        depth + 1,
                    );
                }
            }
            return;
        };
        let direct_currency = field(object, "priceCurrency");
        let specified_currency = specification.and_then(|spec| field(spec, "priceCurrency"));
        let Some(currency) = currency(direct_currency.or(specified_currency)) else {
            return;
        };
        // Conflicting explicit price specifications are ambiguous; never choose
        // an arbitrary regular/list/sale price on behalf of a price watch.
        if direct_price.is_some()
            && specified_price.is_some()
            && decimal(specified_price).as_deref() != Some(&price)
        {
            return;
        }
        if direct_currency.is_some()
            && specified_currency.is_some()
            && self::currency(specified_currency).as_deref() != Some(&currency)
        {
            return;
        }
        let high_price = if aggregate {
            match field(object, "highPrice") {
                Some(value) => match decimal(Some(value)) {
                    Some(high) if decimal_lte(&price, &high) => Some(high),
                    _ => return,
                },
                None => None,
            }
        } else {
            None
        };
        let availability = text_property(object, "availability", 180).and_then(|raw| {
            let value = raw
                .strip_prefix("https://schema.org/")
                .or_else(|| raw.strip_prefix("http://schema.org/"))
                .unwrap_or(&raw);
            [
                "InStock",
                "OutOfStock",
                "PreOrder",
                "BackOrder",
                "LimitedAvailability",
                "SoldOut",
                "OnlineOnly",
                "InStoreOnly",
                "Discontinued",
                "PreSale",
                "MadeToOrder",
                "Reserved",
            ]
            .contains(&value)
            .then(|| value.to_string())
        });
        let seller = field(object, "seller")
            .and_then(|value| {
                value.as_str().map(|text| label(text, 200)).or_else(|| {
                    value
                        .as_object()
                        .and_then(|seller| text_property(seller, "name", 200))
                })
            })
            .filter(|seller| !seller.is_empty());
        let product = if product.is_empty() {
            field(object, "itemOffered")
                .and_then(Value::as_object)
                .and_then(|item| text_property(item, "name", 300))
                .or_else(|| text_property(object, "name", 300))
                .unwrap_or_else(|| "Published product offer".into())
        } else {
            label(product, 300)
        };
        let offer = PriceOffer {
            product,
            url: self.source_url.clone(),
            price,
            currency,
            availability,
            seller,
            is_aggregate: aggregate,
            high_price,
        };
        if !self.offers.contains(&offer) {
            self.offers.push(offer);
        }
        if aggregate {
            // The range already represents these underlying offers. Avoid
            // duplicating them as unrelated, unnamed product quotes.
            if let Some(children) = field(object, "offers") {
                self.skip_nested_offers(
                    Node {
                        value: children,
                        schema: node.schema,
                    },
                    depth + 1,
                );
            }
        }
    }
}

pub fn extract_offers(html: &str, page_url: &str) -> Result<Vec<PriceOffer>, String> {
    if html.len() > MAX_HTML_BYTES {
        return Err("The product page exceeds the structured-data parsing limit.".into());
    }
    let source_url = validate_public_url(page_url)?.to_string();
    let document = Html::parse_document(html);
    let scripts = Selector::parse("script[type]").unwrap();
    let mut documents = Vec::new();
    let mut count = 0;
    for script in document.select(&scripts) {
        let mime = script
            .value()
            .attr("type")
            .unwrap_or("")
            .split(';')
            .next()
            .unwrap_or("")
            .trim();
        if !mime.eq_ignore_ascii_case("application/ld+json") {
            continue;
        }
        count += 1;
        if count > MAX_SCRIPTS {
            return Err("The product page contains too many structured-data blocks.".into());
        }
        let raw = script.text().collect::<String>();
        if raw.len() > MAX_SCRIPT_BYTES {
            return Err("A product structured-data block exceeds the safe size limit.".into());
        }
        if let Ok(value) = serde_json::from_str::<Value>(raw.trim().trim_start_matches('\u{feff}'))
        {
            documents.push(value);
        }
    }
    let mut nodes = Vec::new();
    let mut budget = MAX_JSON_NODES;
    for document in &documents {
        collect(document, false, 0, &mut budget, &mut nodes)?;
    }
    let mut ids = HashMap::new();
    for node in &nodes {
        if let Some(id) = node.value.get("@id").and_then(Value::as_str) {
            // A real definition must win over a reference-only object.
            if node
                .value
                .as_object()
                .is_some_and(|object| object.len() > 2)
            {
                ids.entry(id).or_insert(*node);
            }
        }
    }
    let mut extractor = Extractor {
        ids,
        visited: HashSet::new(),
        offers: Vec::new(),
        source_url,
    };
    // Resolve Product associations first, so an offer referenced through @graph
    // retains its actual product name rather than a generic label.
    for node in &nodes {
        if typed(*node, "Product") {
            let object = node.value.as_object().unwrap();
            let name = text_property(object, "name", 300).unwrap_or_default();
            if let Some(value) = field(object, "offers") {
                extractor.offers(
                    Node {
                        value,
                        schema: node.schema,
                    },
                    &name,
                    0,
                );
            }
        }
    }
    for node in nodes {
        if typed(node, "Offer") || typed(node, "AggregateOffer") {
            extractor.offers(node, "", 0);
        }
    }
    if extractor.offers.is_empty() {
        return Err("This shop doesn’t publish its price in a form I can read reliably, so I won’t guess it.".into());
    }
    Ok(extractor.offers)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn html(data: Value) -> String {
        format!("<script type='application/ld+json'>{data}</script>")
    }

    #[test]
    fn product_offer_arrays_preserve_decimal_currency_and_provenance() {
        let page = html(
            serde_json::json!({"@context":"https://schema.org","@type":"Product","name":"Camera","offers":[
                {"@type":"Offer","price":"00199.9900","priceCurrency":"USD","availability":"https://schema.org/InStock","seller":{"name":"Store"},"url":"https://evil.example/checkout"},
                {"@type":"Offer","price":219,"priceCurrency":"EUR"}
            ]}),
        );
        let offers = extract_offers(&page, "https://shop.example.com/camera").unwrap();
        assert_eq!(offers.len(), 2);
        assert_eq!(offers[0].product, "Camera");
        assert_eq!(offers[0].price, "199.99");
        assert_eq!(offers[0].currency, "USD");
        assert_eq!(offers[0].seller.as_deref(), Some("Store"));
        assert_eq!(offers[0].availability.as_deref(), Some("InStock"));
        assert_eq!(offers[0].url, "https://shop.example.com/camera");
        assert!(!offers[0].is_aggregate);
    }

    #[test]
    fn aggregate_ranges_and_graph_references_are_explicit() {
        let page = html(
            serde_json::json!({"@context":"https://schema.org","@graph":[
                {"@type":"Product","name":"Shoes","offers":{"@id":"#offers"}},
                {"@id":"#offers","@type":"AggregateOffer","lowPrice":"79.50","highPrice":"120.00","priceCurrency":"GBP"}
            ]}),
        );
        let offers = extract_offers(&page, "https://shop.example.com/shoes").unwrap();
        assert_eq!(offers.len(), 1);
        assert_eq!(offers[0].product, "Shoes");
        assert!(offers[0].is_aggregate);
        assert_eq!(offers[0].price, "79.5");
        assert_eq!(offers[0].high_price.as_deref(), Some("120"));
        let invalid = html(
            serde_json::json!({"@context":"https://schema.org","@type":"AggregateOffer","lowPrice":"100","highPrice":"99.99","priceCurrency":"USD"}),
        );
        assert!(extract_offers(&invalid, "https://shop.example.com").is_err());
        let with_children = html(
            serde_json::json!({"@context":"https://schema.org","@type":"Product","name":"Shoes","offers":{"@type":"AggregateOffer","lowPrice":"20","highPrice":"25","priceCurrency":"USD","offers":[{"@type":"Offer","price":"20","priceCurrency":"USD"},{"@type":"Offer","price":"25","priceCurrency":"USD"}]}}),
        );
        let offers = extract_offers(&with_children, "https://shop.example.com/shoes").unwrap();
        assert_eq!(offers.len(), 1);
        assert!(offers[0].is_aggregate);
    }

    #[test]
    fn prices_are_never_guessed_from_random_numbers_or_ambiguous_values() {
        assert!(extract_offers(
            "<p>Now $19.99! Model 2026. 5 stars.</p>",
            "https://shop.example.com"
        )
        .is_err());
        for value in [
            serde_json::json!("$19.99"),
            serde_json::json!("1,999.00"),
            serde_json::json!("NaN"),
            serde_json::json!(-1),
            serde_json::json!(0),
            serde_json::json!(true),
            serde_json::json!("1.2345678"),
        ] {
            let page = html(
                serde_json::json!({"@context":"https://schema.org","@type":"Offer","price":value,"priceCurrency":"USD"}),
            );
            assert!(extract_offers(&page, "https://shop.example.com").is_err());
        }
        for code in ["usd", "$", "US", "XYZ", "XXX"] {
            let page = html(
                serde_json::json!({"@context":"https://schema.org","@type":"Offer","price":"10","priceCurrency":code}),
            );
            assert!(extract_offers(&page, "https://shop.example.com").is_err());
        }
        let page = html(
            serde_json::json!({"@context":"https://schema.org","@type":"Offer","price":"10","priceCurrency":"USD","priceSpecification":{"price":"12","priceCurrency":"USD"}}),
        );
        assert!(extract_offers(&page, "https://shop.example.com").is_err());
        let wrong_context = html(
            serde_json::json!({"@context":"https://unrelated.example/vocab","@type":"Offer","price":"10","priceCurrency":"USD"}),
        );
        assert!(extract_offers(&wrong_context, "https://shop.example.com").is_err());
        let nested_context = html(
            serde_json::json!({"@context":"https://schema.org","@type":"Product","name":"Item","offers":{"@context":"https://unrelated.example/vocab","@type":"Offer","price":"10","priceCurrency":"USD"}}),
        );
        assert!(extract_offers(&nested_context, "https://shop.example.com").is_err());
    }

    #[test]
    fn watch_thresholds_use_exact_decimal_comparisons() {
        assert_eq!(canonical_decimal("00199.990000").as_deref(), Some("199.99"));
        assert!(decimal_lte("9.999999", "10"));
        assert!(decimal_lte("9007199254740992", "9007199254740993"));
        assert!(!decimal_lte("9007199254740993", "9007199254740992"));
        assert!(decimal_lte("1.2000", "1.2"));
        assert!(!decimal_lte("1.000001", "1"));
        assert!(!decimal_lte("NaN", "10"));
        assert!(!decimal_lte("-1", "10"));
    }

    #[test]
    fn bounded_malformed_and_full_schema_types_are_handled() {
        let page = "<script type='application/ld+json'>{broken}</script>".to_string()
            + &html(
                serde_json::json!({"@type":"https://schema.org/Offer","price":"1.00","priceCurrency":"JPY"}),
            );
        assert_eq!(
            extract_offers(&page, "https://shop.example.com").unwrap()[0].price,
            "1"
        );
        assert!(
            extract_offers(&"a".repeat(MAX_HTML_BYTES + 1), "https://shop.example.com").is_err()
        );
        let page = html(serde_json::json!({"@context":"https://schema.org","@type":"Offer","price":"10","priceCurrency":"USD"})).repeat(MAX_SCRIPTS + 1);
        assert!(extract_offers(&page, "https://shop.example.com").is_err());
    }
}

/// What a shop page showed once its scripts had run, read by a fixed script.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RenderedPage {
    pub ready: bool,
    pub url: String,
    pub title: String,
    pub heading: String,
    pub heading_top: Option<f64>,
    pub json_ld: Vec<String>,
    pub meta_amount: Option<String>,
    pub meta_currency: Option<String>,
    pub candidates: Vec<PriceOnPage>,
    pub buy: bool,
    pub sold_out: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct PriceOnPage {
    pub text: String,
    pub size: f64,
    pub top: f64,
    /// Crossed out, as a former price is.
    pub struck: bool,
    /// In navigation, recommendations, financing or the like.
    pub noise: bool,
    /// Its own markup calls it a price.
    pub hint: bool,
    /// The short text right around it.
    pub near: String,
}

impl RenderedPage {
    /// Enough to tell whether the page has stopped changing.
    pub fn signature(&self) -> String {
        let prices: Vec<&str> = self.candidates.iter().map(|price| price.text.as_str()).collect();
        format!("{}|{}|{:?}|{}", self.json_ld.len(), self.meta_amount.as_deref().unwrap_or_default(), prices, self.heading)
    }
}

/// "$3,199.95", "1.299,00 €", "CA$40", "USD 19" as an amount and whether `currency` fits it.
fn visible_price(text: &str, currency: &str) -> Option<String> {
    let digits: String = text.chars().filter(|c| c.is_ascii_digit() || matches!(c, '.' | ',')).collect();
    let mark: String = text.chars().filter(|c| !c.is_ascii_digit() && !matches!(c, '.' | ',') && !c.is_whitespace()).collect();
    let fits = match mark.as_str() {
        "$" => ["USD", "CAD", "AUD", "NZD", "SGD", "HKD", "MXN"].contains(&currency),
        "US$" => currency == "USD",
        "CA$" | "C$" => currency == "CAD",
        "A$" | "AU$" => currency == "AUD",
        "€" => currency == "EUR",
        "£" => currency == "GBP",
        "¥" => ["JPY", "CNY"].contains(&currency),
        "₹" => currency == "INR",
        code => code == currency,
    };
    if !fits {
        return None;
    }
    // The last separator is a decimal point only with one or two digits after it.
    let plain = match digits.rfind(['.', ',']) {
        Some(at) if digits.len() - at - 1 <= 2 => format!("{}.{}", digits[..at].replace(['.', ','], ""), &digits[at + 1..]),
        _ => digits.replace(['.', ','], ""),
    };
    canonical_decimal(&plain).filter(|amount| amount != "0")
}

/// The price a rendered page is selling at, or why it can't be told. A published record comes
/// first; failing that the most prominent price on the page, if that's one clear amount. This
/// can never say a price the page doesn't show, and when it can't tell it doesn't choose.
pub fn offer_from_rendered(page: &RenderedPage, currency: &str) -> Result<PriceOffer, String> {
    let product = label(if page.heading.is_empty() { &page.title } else { &page.heading }, 180);
    // A "sold out" and an "add to cart" together may be about other products on the page.
    let availability = match (page.buy, page.sold_out) {
        (true, false) => Some("InStock".to_string()),
        (false, true) => Some("OutOfStock".to_string()),
        _ => None,
    };
    let offer = |price: String, availability: Option<String>| PriceOffer { product: product.clone(), url: page.url.clone(), price, currency: currency.to_string(), availability, seller: None, is_aggregate: false, high_price: None };
    if !page.json_ld.is_empty() {
        let html: String = page.json_ld.iter().map(|text| format!("<script type=\"application/ld+json\">{}</script>", text.replace("</", "<\\/"))).collect();
        if let Ok(found) = extract_offers(&html, &page.url).and_then(|offers| crate::watches::exact_offer(&offers, currency)) {
            return Ok(found);
        }
    }
    if let Some(amount) = page.meta_amount.as_deref().and_then(|amount| canonical_decimal(&amount.replace(',', ""))) {
        if page.meta_currency.as_deref().is_some_and(|meta| meta.eq_ignore_ascii_case(currency)) && amount != "0" {
            return Ok(offer(amount, availability));
        }
    }
    const NOT_THE_PRICE: [&str; 17] = ["save", "off", "was", "list", "reg", "msrp", "mo", "month", "monthly", "shipping", "delivery", "rebate", "coupon", "credit", "trade", "deposit", "per"];
    let eligible: Vec<(&PriceOnPage, String)> = page
        .candidates
        .iter()
        .filter(|price| !price.struck && !price.noise && price.top < 2200.0)
        .filter(|price| {
            let near = price.near.to_lowercase();
            !near.split(|c: char| !c.is_alphabetic()).any(|word| NOT_THE_PRICE.contains(&word))
        })
        .filter_map(|price| visible_price(&price.text, currency).map(|amount| (price, amount)))
        .collect();
    let Some(largest) = eligible.iter().map(|(price, _)| price.size).reduce(f64::max) else {
        return Err("This shop didn’t show a price I could read. It may need a sign-in or be keeping automated readers out.".into());
    };
    let top: Vec<&(&PriceOnPage, String)> = eligible.iter().filter(|(price, _)| price.size >= largest * 0.92).collect();
    let marked: Vec<&&(&PriceOnPage, String)> = top.iter().filter(|(price, _)| price.hint).collect();
    let mut amounts: Vec<&String> = if marked.is_empty() { top.iter().map(|(_, amount)| amount).collect() } else { marked.iter().map(|(_, amount)| amount).collect() };
    amounts.sort();
    amounts.dedup();
    match amounts[..] {
        [one] => Ok(offer(one.clone(), availability)),
        _ => Err("This page shows several prices and I can’t tell which is yours. Choose a page for just the one you want.".into()),
    }
}

#[cfg(test)]
mod rendered_tests {
    use super::*;

    fn price(text: &str, size: f64) -> PriceOnPage {
        PriceOnPage { text: text.into(), size, top: 400.0, ..Default::default() }
    }
    fn page(candidates: Vec<PriceOnPage>) -> RenderedPage {
        RenderedPage { ready: true, url: "https://shop.example.com/p/1".into(), heading: "Billy bookcase".into(), buy: true, candidates, ..Default::default() }
    }

    #[test]
    fn a_page_that_says_both_doesnt_say_whether_its_in_stock() {
        let mut both = page(vec![price("$129.00", 32.0)]);
        both.sold_out = true;
        assert_eq!(offer_from_rendered(&both, "USD").unwrap().availability, None);
        let mut foreign = page(vec![]);
        foreign.meta_amount = Some("219".into());
        assert!(offer_from_rendered(&foreign, "USD").is_err(), "a price with no currency isn't dollars");
    }

    #[test]
    fn the_most_prominent_price_is_the_price() {
        let shown = offer_from_rendered(&page(vec![price("$19.99", 14.0), price("$129.00", 32.0), price("$5.00", 12.0)]), "USD").unwrap();
        assert_eq!((shown.price.as_str(), shown.product.as_str(), shown.availability.as_deref()), ("129", "Billy bookcase", Some("InStock")));
    }

    #[test]
    fn former_prices_and_other_peoples_prices_are_not_the_price() {
        let mut was = price("$199.00", 40.0);
        was.struck = true;
        let mut other = price("$49.00", 40.0);
        other.noise = true;
        let mut monthly = price("$22.00", 40.0);
        monthly.near = "$22.00 per month".into();
        let mut saving = price("$30.00", 40.0);
        saving.near = "Save $30.00".into();
        let mut far = price("$9.00", 40.0);
        far.top = 5000.0;
        let shown = offer_from_rendered(&page(vec![was, other, monthly, saving, far, price("$169.00", 20.0)]), "USD").unwrap();
        assert_eq!(shown.price, "169");
    }

    #[test]
    fn it_never_chooses_between_equally_prominent_prices() {
        assert!(offer_from_rendered(&page(vec![price("$10.00", 20.0), price("$20.00", 20.0)]), "USD").unwrap_err().contains("several prices"));
        let mut marked = price("$20.00", 20.0);
        marked.hint = true;
        assert_eq!(offer_from_rendered(&page(vec![price("$10.00", 20.0), marked]), "USD").unwrap().price, "20");
        assert_eq!(offer_from_rendered(&page(vec![price("$10.00", 20.0), price("$10.00", 21.0)]), "USD").unwrap().price, "10");
        assert!(offer_from_rendered(&page(vec![]), "USD").unwrap_err().contains("didn’t show a price"));
    }

    #[test]
    fn a_price_has_to_be_in_the_watched_currency() {
        assert!(offer_from_rendered(&page(vec![price("£10.00", 20.0)]), "USD").is_err());
        assert_eq!(visible_price("$3,199.95", "USD").as_deref(), Some("3199.95"));
        assert_eq!(visible_price("1.299,00 €", "EUR").as_deref(), Some("1299"));
        assert_eq!(visible_price("1,299", "USD"), None);
        assert_eq!(visible_price("USD 1,299", "USD").as_deref(), Some("1299"));
        assert_eq!(visible_price("CA$40", "CAD").as_deref(), Some("40"));
        assert_eq!(visible_price("CA$40", "USD"), None);
        assert_eq!(visible_price("$0.00", "USD"), None);
    }

    #[test]
    fn a_published_record_comes_before_what_is_shown() {
        let mut record = page(vec![price("$1.00", 50.0)]);
        record.json_ld = vec![r#"{"@context":"https://schema.org","@type":"Product","name":"Z 8","offers":{"@type":"Offer","price":"3199.95","priceCurrency":"USD","availability":"https://schema.org/InStock"}}"#.into()];
        assert_eq!(offer_from_rendered(&record, "USD").unwrap().price, "3199.95");
        let mut meta = page(vec![price("$1.00", 50.0)]);
        meta.meta_amount = Some("24.50".into());
        meta.meta_currency = Some("USD".into());
        assert_eq!(offer_from_rendered(&meta, "USD").unwrap().price, "24.5");
        meta.meta_currency = Some("GBP".into());
        assert_eq!(offer_from_rendered(&meta, "USD").unwrap().price, "1");
    }

    #[test]
    fn sold_out_shows_as_sold_out() {
        let mut gone = page(vec![price("$10.00", 20.0)]);
        gone.buy = false;
        gone.sold_out = true;
        assert_eq!(offer_from_rendered(&gone, "USD").unwrap().availability.as_deref(), Some("OutOfStock"));
    }
}

#[cfg(test)]
mod live_pages {
    use super::*;
    /// Real shop pages through the fetch and reader a price watch uses. Reads public pages.
    #[tokio::test]
    #[ignore = "fetches real shop pages; run with --ignored live_price_pages"]
    async fn live_price_pages() {
        let urls = std::env::var("BUNTINI_PRICE_URLS").unwrap();
        for url in urls.split_whitespace() {
            let started = std::time::Instant::now();
            match crate::research::fetch_watched_html(url).await {
                Err(error) => eprintln!("PAGE {url} | fetch failed | {error}"),
                Ok(page) => {
                    let found = extract_offers(&page.html, url);
                    let parsed = started.elapsed();
                    match found {
                        Ok(offers) => eprintln!("PAGE {url} | watch sees: {:?} | {} bytes | {:?} | {} offers | {:?}", crate::watches::exact_offer(&offers, "USD").map(|offer| (offer.price, offer.availability)), page.html.len(), parsed, offers.len(), offers.iter().take(2).map(|o| (o.product.clone(), o.price.clone(), o.currency.clone(), o.availability.clone())).collect::<Vec<_>>()),
                        Err(error) => eprintln!("PAGE {url} | {} bytes | read failed | {error}", page.html.len()),
                    }
                }
            }
        }
    }
}
