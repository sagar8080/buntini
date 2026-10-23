//! eBay Browse API listing search with the user's own developer app keys.
use super::*;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const ATTRIBUTION: &str = "Live eBay listing data · prices and stock can change";
/// (marketplace id, currency used for the price filter)
pub const MARKETPLACES: [(&str, &str); 8] = [
    ("EBAY_US", "USD"),
    ("EBAY_GB", "GBP"),
    ("EBAY_DE", "EUR"),
    ("EBAY_FR", "EUR"),
    ("EBAY_IT", "EUR"),
    ("EBAY_ES", "EUR"),
    ("EBAY_AU", "AUD"),
    ("EBAY_CA", "CAD"),
];

/// Application tokens are cached per client id until shortly before they expire.
static TOKEN: Mutex<Option<(String, String, Instant)>> = Mutex::new(None);

pub fn validate(args: &Value) -> Result<Value, String> {
    let map = object(args, &["query", "max_price", "condition", "sort"])?;
    Ok(json!({
        "query": required(map, "query", 200)?,
        "max_price": number(map, "max_price", 0.01, 10_000_000.0)?,
        "condition": choice(map, "condition", &["any", "new", "used"])?.unwrap_or_else(|| "any".into()),
        "sort": choice(map, "sort", &["best_match", "price", "newest"])?.unwrap_or_else(|| "best_match".into()),
    }))
}

fn currency(marketplace: &str) -> &'static str {
    MARKETPLACES
        .iter()
        .find(|(id, _)| *id == marketplace)
        .map(|(_, currency)| *currency)
        .unwrap_or("USD")
}

pub fn preview(args: &Value, marketplace: &str) -> Preview {
    let mut sends = vec![
        CardField::new("Search", text_of(args, "query")),
        CardField::new("Marketplace", marketplace),
    ];
    if let Some(max) = args.get("max_price").and_then(Value::as_f64) {
        sends.push(CardField::new(
            "Max price",
            format!("{max} {}", currency(marketplace)),
        ));
    }
    sends.push(CardField::new("Condition", text_of(args, "condition")));
    sends.push(CardField::new(
        "Sort",
        text_of(args, "sort").replace('_', " "),
    ));
    Preview {
        provider: "eBay".into(),
        host: "api.ebay.com".into(),
        summary: format!("Search eBay listings for “{}”", text_of(args, "query")),
        sends,
        uses_key: true,
    }
}

/// The Keychain entry holds `{"clientId": …, "clientSecret": …}` from the eBay developer portal.
pub fn credentials(saved: &str) -> Result<(String, String), String> {
    let value: Value =
        serde_json::from_str(saved).map_err(|_| "Re-enter your eBay keys in Settings.")?;
    let field = |key: &str| {
        value
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty() && v.len() <= 256)
            .map(str::to_string)
    };
    match (field("clientId"), field("clientSecret")) {
        (Some(id), Some(secret)) => Ok((id, secret)),
        _ => Err("Re-enter your eBay client ID and secret in Settings.".into()),
    }
}

fn host(client_id: &str) -> &'static str {
    // Sandbox app ids carry an "-SBX-" segment; their tokens only work on the sandbox host.
    if client_id.contains("-SBX-") {
        "https://api.sandbox.ebay.com"
    } else {
        "https://api.ebay.com"
    }
}

async fn token(client: &reqwest::Client, id: &str, secret: &str) -> Result<String, String> {
    if let Some((cached_id, token, until)) = TOKEN.lock().unwrap().clone() {
        if cached_id == id && Instant::now() < until {
            return Ok(token);
        }
    }
    let data = json_response(
        client
            .post(format!("{}/identity/v1/oauth2/token", host(id)))
            .basic_auth(id, Some(secret))
            .form(&[
                ("grant_type", "client_credentials"),
                ("scope", "https://api.ebay.com/oauth/api_scope"),
            ])
            .send()
            .await,
        "eBay",
        65_536,
    )
    .await?;
    let token = data
        .get("access_token")
        .and_then(Value::as_str)
        .ok_or("eBay did not issue an access token.")?
        .to_string();
    let lifetime = data
        .get("expires_in")
        .and_then(Value::as_u64)
        .unwrap_or(3600)
        .saturating_sub(120);
    *TOKEN.lock().unwrap() = Some((
        id.into(),
        token.clone(),
        Instant::now() + Duration::from_secs(lifetime),
    ));
    Ok(token)
}

pub async fn search(
    args: &Value,
    saved: &str,
    marketplace: &str,
) -> Result<Vec<ResultCard>, String> {
    let (id, secret) = credentials(saved)?;
    let client = client()?;
    let token = token(&client, &id, &secret).await?;
    let mut query = vec![("q", text_of(args, "query")), ("limit", "10".to_string())];
    match text_of(args, "sort").as_str() {
        "price" => query.push(("sort", "price".into())),
        "newest" => query.push(("sort", "newlyListed".into())),
        _ => {}
    }
    let mut filters = vec![];
    if let Some(max) = args.get("max_price").and_then(Value::as_f64) {
        filters.push(format!(
            "price:[..{max}],priceCurrency:{}",
            currency(marketplace)
        ));
    }
    match text_of(args, "condition").as_str() {
        "new" => filters.push("conditions:{NEW}".into()),
        "used" => filters.push("conditions:{USED}".into()),
        _ => {}
    }
    if !filters.is_empty() {
        query.push(("filter", filters.join(",")));
    }
    let data = json_response(
        client
            .get(format!("{}/buy/browse/v1/item_summary/search", host(&id)))
            .bearer_auth(token)
            .header("X-EBAY-C-MARKETPLACE-ID", marketplace)
            .query(&query)
            .send()
            .await,
        "eBay",
        2_097_152,
    )
    .await?;
    let cards = parse_listings(&data);
    if cards.is_empty() {
        return Err(format!(
            "eBay has no listings matching “{}” with these filters.",
            text_of(args, "query")
        ));
    }
    Ok(cards)
}

fn money(value: Option<&Value>) -> Option<String> {
    let value = value?;
    Some(format!(
        "{} {}",
        value.get("value")?.as_str()?,
        value.get("currency")?.as_str()?
    ))
}

pub(crate) fn parse_listings(data: &Value) -> Vec<ResultCard> {
    let Some(items) = data.get("itemSummaries").and_then(Value::as_array) else {
        return vec![];
    };
    items
        .iter()
        .filter_map(|item| {
            let title = item.get("title")?.as_str()?;
            let price = money(item.get("price"))?;
            let shipping = item
                .pointer("/shippingOptions/0/shippingCost")
                .and_then(|cost| {
                    (cost.get("value")?.as_str()?.parse::<f64>().ok()? == 0.0)
                        .then(|| "Free".to_string())
                        .or_else(|| money(Some(cost)))
                });
            let seller = item.get("seller").map(|seller| {
                let name = seller
                    .get("username")
                    .and_then(Value::as_str)
                    .unwrap_or("Seller");
                match (
                    seller.get("feedbackPercentage").and_then(Value::as_str),
                    seller.get("feedbackScore").and_then(Value::as_u64),
                ) {
                    (Some(percent), Some(score)) => {
                        format!("{name} · {percent}% positive ({score} ratings)")
                    }
                    _ => name.to_string(),
                }
            });
            let options = item
                .get("buyingOptions")
                .and_then(Value::as_array)
                .map(|options| {
                    options
                        .iter()
                        .filter_map(Value::as_str)
                        .map(|option| match option {
                            "FIXED_PRICE" => "Buy it now",
                            "AUCTION" => "Auction",
                            "BEST_OFFER" => "Best offer",
                            other => other,
                        })
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            let link = item
                .get("itemWebUrl")
                .and_then(Value::as_str)
                .filter(|url| {
                    reqwest::Url::parse(url)
                        .ok()
                        .and_then(|url| url.host_str().map(|host| host.contains("ebay.")))
                        .unwrap_or(false)
                });
            Some(card(
                "listing",
                title,
                [
                    item.get("condition").and_then(Value::as_str).unwrap_or(""),
                    options.as_str(),
                ]
                .into_iter()
                .filter(|p| !p.is_empty())
                .collect::<Vec<_>>()
                .join(" · "),
                vec![
                    CardField::new("Price", price),
                    CardField::new("Shipping", shipping.unwrap_or_else(|| "See listing".into())),
                    CardField::new("Seller", seller.unwrap_or_default()),
                    CardField::new(
                        "Ships from",
                        item.pointer("/itemLocation/country")
                            .and_then(Value::as_str)
                            .unwrap_or(""),
                    ),
                ],
                link.map(str::to_string),
                ATTRIBUTION,
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_and_saved_credentials_are_checked() {
        assert_eq!(
            validate(&json!({"query":"Kindle Paperwhite","max_price":"120","condition":"Used"}))
                .unwrap(),
            json!({"query":"Kindle Paperwhite","max_price":120.0,"condition":"used","sort":"best_match"})
        );
        assert!(validate(&json!({"query":"x","condition":"refurbished"})).is_err());
        assert!(validate(&json!({"query":"x","max_price":0})).is_err());
        assert_eq!(
            credentials(r#"{"clientId":"Me-App-PRD-1","clientSecret":"PRD-s"}"#)
                .unwrap()
                .0,
            "Me-App-PRD-1"
        );
        assert!(credentials("not json").is_err());
        assert!(credentials(r#"{"clientId":"x"}"#).is_err());
        assert_eq!(host("Me-App-SBX-1"), "https://api.sandbox.ebay.com");
        assert_eq!(host("Me-App-PRD-1"), "https://api.ebay.com");
    }

    #[test]
    fn listings_show_price_shipping_and_seller_with_safe_links() {
        let data = json!({"itemSummaries":[
            {"title":"Kindle Paperwhite 11th gen","price":{"value":"89.99","currency":"USD"},"condition":"Used","buyingOptions":["FIXED_PRICE","BEST_OFFER"],"shippingOptions":[{"shippingCost":{"value":"0.00","currency":"USD"}}],"seller":{"username":"books4u","feedbackPercentage":"99.6","feedbackScore":5120},"itemLocation":{"country":"US"},"itemWebUrl":"https://www.ebay.com/itm/123"},
            {"title":"Phishy","price":{"value":"1.00","currency":"USD"},"itemWebUrl":"https://evil.example/itm/1","shippingOptions":[{"shippingCost":{"value":"4.50","currency":"USD"}}]},
            {"title":"No price"}
        ]});
        let cards = parse_listings(&data);
        assert_eq!(cards.len(), 2);
        assert_eq!(cards[0].subtitle, "Used · Buy it now, Best offer");
        assert_eq!(cards[0].fields[1], CardField::new("Shipping", "Free"));
        assert_eq!(
            cards[0].fields[2].value,
            "books4u · 99.6% positive (5120 ratings)"
        );
        assert_eq!(
            cards[0].url.as_deref(),
            Some("https://www.ebay.com/itm/123")
        );
        assert_eq!(cards[1].url, None);
        assert_eq!(cards[1].fields[1].value, "4.50 USD");
        assert!(parse_listings(&json!({"total":0})).is_empty());
    }
}
