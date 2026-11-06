use crate::{
    controller::AppState,
    pricing::{canonical_decimal, decimal_lte, PriceOffer},
    types::*,
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tauri::{Manager, State};

fn next(interval: u32) -> String {
    (chrono::Utc::now() + chrono::Duration::minutes(interval.into())).to_rfc3339()
}
fn interval_valid(interval: u32) -> Result<(), String> {
    if (15..=525600).contains(&interval) {
        Ok(())
    } else {
        Err("Price watches must run between every 15 minutes and once a year.".into())
    }
}
#[tauri::command]
pub fn save_price_watch(
    url: String,
    label: String,
    currency: String,
    threshold: String,
    interval_minutes: u32,
    state: State<'_, AppState>,
) -> Result<PriceWatch, String> {
    interval_valid(interval_minutes)?;
    let canonical = crate::research::validate_public_url(&url)?.to_string();
    if label.chars().count() > 120 || label.chars().any(char::is_control) {
        return Err("Use a watch label of up to 120 visible characters.".into());
    }
    let currency = currency.to_ascii_uppercase();
    if currency.len() != 3 || !currency.bytes().all(|c| c.is_ascii_uppercase()) {
        return Err("Use a three-letter currency code such as USD.".into());
    }
    let threshold = canonical_decimal(&threshold)
        .ok_or("Enter a positive decimal target price with up to six decimal places.")?;
    let store = state.store.lock().unwrap();
    if store.price_watches()?.len() >= 20 {
        return Err("You can save up to 20 price watches.".into());
    }
    let watch = PriceWatch {
        id: uuid::Uuid::new_v4().to_string(),
        url: canonical,
        label: if label.trim().is_empty() {
            "Price watch".into()
        } else {
            label.trim().into()
        },
        currency,
        threshold,
        interval_minutes,
        enabled: true,
        created_at: now(),
        next_run_at: next(interval_minutes),
        last_checked_at: None,
        last_price: None,
        last_error: None,
        last_availability: None,
        history: vec![],
    };
    store.save_price_watch(&watch)?;
    store.audit(
        "price_watch.allowed",
        &format!(
            "{}: explicit user authorization for recurring public HTTPS fetch",
            watch.id
        ),
    )?;
    Ok(watch)
}
#[tauri::command]
pub fn update_price_watch(
    id: String,
    enabled: bool,
    interval_minutes: u32,
    state: State<'_, AppState>,
) -> Result<PriceWatch, String> {
    let active = state.active.lock().unwrap();
    if !enabled {
        if let Some(cancel) = active.get(&format!("price-watch-{id}")) {
            cancel.store(true, Ordering::SeqCst);
        }
    }
    interval_valid(interval_minutes)?;
    let store = state.store.lock().unwrap();
    let mut watch = store.price_watch(&id)?.ok_or("Price watch not found.")?;
    watch.enabled = enabled;
    watch.interval_minutes = interval_minutes;
    watch.next_run_at = next(interval_minutes);
    store.save_price_watch(&watch)?;
    store.audit("price_watch.updated", &format!("{id}: enabled={enabled}"))?;
    Ok(watch)
}
#[tauri::command]
pub fn delete_price_watch(id: String, state: State<'_, AppState>) -> Result<(), String> {
    let active = state.active.lock().unwrap();
    if let Some(cancel) = active.get(&format!("price-watch-{id}")) {
        cancel.store(true, Ordering::SeqCst);
    }
    state.store.lock().unwrap().delete_price_watch(&id)
}

pub fn exact_offer(offers: &[PriceOffer], currency: &str) -> Result<PriceOffer, String> {
    let matching = offers
        .iter()
        .filter(|offer| offer.currency == currency)
        .collect::<Vec<_>>();
    let Some(first) = matching.first() else {
        return Err("This shop doesn’t publish a price in that currency in a form I can read reliably.".into());
    };
    if matching.iter().any(|offer| offer.is_aggregate) {
        return Err(
            "This page publishes a price range. Choose a page with one exact product and price."
                .into(),
        );
    }
    let lower = |value: &Option<String>| value.as_ref().map(|value| value.to_lowercase());
    // Colours and sizes of one thing at one price have one price to watch. Different prices
    // on a page never do: which one is yours can't be told, and a cheaper one could alert.
    if matching.iter().any(|offer| offer.price != first.price || lower(&offer.seller) != lower(&first.seller)) {
        return Err("This page lists different prices for its options. Choose a page for just the one you want.".into());
    }
    let mut chosen = (*first).clone();
    if matching.iter().any(|offer| lower(&offer.availability) != lower(&first.availability)) {
        // Only some options are in stock, and whether yours is can't be told from here.
        chosen.availability = Some("LimitedAvailability".into());
    }
    Ok(chosen)
}
fn below(watch: &PriceWatch) -> bool {
    watch.last_error.is_none()
        && watch
            .last_price
            .as_ref()
            .map(|price| decimal_lte(price, &watch.threshold))
            .unwrap_or(false)
        && !watch
            .last_availability
            .as_ref()
            .map(|value| {
                ["outofstock", "soldout", "discontinued"]
                    .iter()
                    .any(|term| value.to_lowercase().contains(term))
            })
            .unwrap_or(false)
}
fn changed(before: &PriceWatch, after: &PriceWatch) -> bool {
    before.last_price != after.last_price
        || before.last_error != after.last_error
        || before.last_availability != after.last_availability
}

fn record_observation(
    watch: &mut PriceWatch,
    result: Result<PriceOffer, String>,
    cancelled: bool,
) -> Result<(), String> {
    if cancelled {
        return Err("Price check cancelled.".into());
    }
    watch.last_checked_at = Some(now());
    match result {
        Ok(offer) => {
            watch.last_price = Some(offer.price.clone());
            watch.last_availability = offer.availability;
            watch.last_error = None;
            watch.history.push(PricePoint {
                price: offer.price,
                currency: offer.currency,
                observed_at: now(),
                url: watch.url.clone(),
            });
            if watch.history.len() > 60 {
                watch.history.drain(..watch.history.len() - 60);
            }
        }
        Err(error) => watch.last_error = Some(error),
    }
    Ok(())
}

/// The price a page is selling at now. The plain read is quick and takes a shop's own record;
/// a shop that turns it away, or only shows its price once scripts have run, gets loaded in
/// Buntini's private browser with no window on screen.
pub(crate) async fn read_price(url: &str, currency: &str, cancel: Arc<AtomicBool>) -> Result<PriceOffer, String> {
    let fetch = crate::research::fetch_watched_html(url);
    tokio::pin!(fetch);
    let direct = loop {
        // Check before the first poll as well as after every timer wakeup, so
        // a revoked queued check cannot start its initial DNS/HTTP work.
        if cancel.load(Ordering::SeqCst) {
            break Err("Price check cancelled.".to_string());
        }
        tokio::select! {
            result = &mut fetch => break result,
            _ = tokio::time::sleep(std::time::Duration::from_millis(40)) => {}
        }
    }
    .and_then(|page| crate::pricing::extract_offers(&page.html, &page.url))
    .and_then(|offers| exact_offer(&offers, currency));
    if direct.is_ok() || cancel.load(Ordering::SeqCst) {
        return direct;
    }
    let page = crate::browser_workspace::read_rendered_price(url, cancel).await?;
    crate::pricing::offer_from_rendered(&page, currency)
}

pub(crate) fn money(amount: &str, currency: &str) -> String {
    let (whole, fraction) = amount.split_once('.').unwrap_or((amount, ""));
    let digits: Vec<char> = whole.chars().collect();
    let grouped: String = digits.iter().enumerate().flat_map(|(at, digit)| {
        let comma = at > 0 && (digits.len() - at) % 3 == 0;
        comma.then_some(',').into_iter().chain(std::iter::once(*digit))
    }).collect();
    let number = if fraction.is_empty() { grouped } else { format!("{grouped}.{fraction:0<2}") };
    match currency {
        "USD" => format!("${number}"),
        "EUR" => format!("€{number}"),
        "GBP" => format!("£{number}"),
        other => format!("{other} {number}"),
    }
}

/// What the first look at a new watch found, for its confirmation.
fn first_look(watch: &PriceWatch, result: &Result<PriceOffer, String>) -> String {
    match result {
        Ok(offer) => {
            let stock = match offer.availability.as_deref() {
                Some("InStock") => ", in stock",
                Some("OutOfStock") => ", sold out",
                Some("LimitedAvailability") => ", some options in stock",
                _ => "",
            };
            let now = format!("It’s {}{stock} now.", money(&offer.price, &offer.currency));
            if below(watch) {
                format!("{now} That’s already at or below your {} target.", money(&watch.threshold, &watch.currency))
            } else {
                now
            }
        }
        Err(error) => format!("I couldn’t read its price: {error}"),
    }
}

/// Reads a watch just made, keeps what it found, and says so. A watch stands either way.
pub(crate) async fn first_check(state: &AppState, id: &str, cancel: Arc<AtomicBool>) -> Option<String> {
    let (url, currency) = {
        let store = state.store.lock().unwrap();
        let watch = store.price_watch(id).ok()??;
        (watch.url, watch.currency)
    };
    let result = read_price(&url, &currency, cancel.clone()).await;
    let store = state.store.lock().unwrap();
    let mut watch = store.price_watch(id).ok()??;
    record_observation(&mut watch, result.clone(), cancel.load(Ordering::SeqCst)).ok()?;
    store.save_price_watch(&watch).ok()?;
    Some(first_look(&watch, &result))
}

pub(crate) async fn run(
    app: &tauri::AppHandle,
    id: &str,
    scheduled: bool,
) -> Result<PriceWatch, String> {
    let state = app.state::<AppState>();
    let token = Arc::new(AtomicBool::new(false));
    let active_id = format!("price-watch-{id}");
    let (before, run_id) = {
        let mut active = state.active.lock().unwrap();
        if !active.is_empty() {
            return Err("Another task is running. Try the price check after it finishes.".into());
        }
        let store = state.store.lock().unwrap();
        let mut watch = store.price_watch(id)?.ok_or("Price watch not found.")?;
        if scheduled
            && (!watch.enabled
                || !chrono::DateTime::parse_from_rfc3339(&watch.next_run_at)
                    .map(|date| date <= chrono::Utc::now())
                    .unwrap_or(false))
        {
            return Err("This price watch is paused or no longer due.".into());
        }
        let before = watch.clone();
        let transaction = store
            .db
            .unchecked_transaction()
            .map_err(|error| error.to_string())?;
        let run_id = crate::delivery::monitor_started(
            &store,
            &active_id,
            scheduled.then_some(watch.next_run_at.as_str()),
            chrono::Utc::now(),
        )?;
        watch.next_run_at = next(watch.interval_minutes);
        store.save_price_watch(&watch)?;
        transaction.commit().map_err(|error| error.to_string())?;
        active.insert(active_id.clone(), token.clone());
        (before, run_id)
    };
    let _claim = crate::notifications::ActiveMonitor::new(&state, active_id, token.clone());
    state.audit("price_watch.fetch", id);
    let result = read_price(&before.url, &before.currency, token.clone()).await;
    let persisted = (|| -> Result<PriceWatch, String> {
        let store = state.store.lock().unwrap();
        let transaction = store
            .db
            .unchecked_transaction()
            .map_err(|error| error.to_string())?;
        let outcome = match store.price_watch(id)? {
            None => Err("This price watch was deleted while checking.".to_string()),
            Some(mut watch) => record_observation(&mut watch, result, token.load(Ordering::SeqCst))
                .and_then(|_| store.save_price_watch(&watch).map(|_| watch)),
        };
        let (status, error) = match &outcome {
            Ok(watch) if watch.last_error.is_none() => ("completed", None),
            Ok(watch) => ("error", watch.last_error.as_deref()),
            Err(error) if token.load(Ordering::SeqCst) => ("cancelled", Some(error.as_str())),
            Err(error) => ("error", Some(error.as_str())),
        };
        crate::delivery::monitor_finished(&store, &run_id, status, error)?;
        transaction.commit().map_err(|error| error.to_string())?;
        outcome
    })();
    let _ = tauri::Emitter::emit(
        app,
        "background-updated",
        serde_json::json!({"monitorKey":format!("price-watch-{id}")}),
    );
    let watch = persisted?;
    let changed = changed(&before, &watch);
    let below_threshold = below(&watch);
    if changed && !token.load(Ordering::Relaxed) {
        let notification = (below_threshold || watch.last_error.is_some()).then_some((
            if below_threshold { "Price target reached" } else { "Price watch needs attention" },
            if below_threshold { "A watched price reached your target. Open Buntini to review the source and availability." } else { "A price watch could not be verified. Open Buntini to review it." },
        ));
        crate::notifications::dispatch_monitor(
            app,
            crate::notifications::Monitor::Price { id, scheduled },
            &token,
            "price-watch-result",
            serde_json::json!({"watch":watch,"belowThreshold":below_threshold,"changed":true}),
            notification,
        )
        .await?;
    }
    state.audit(
        "price_watch.checked",
        &format!(
            "{id}: {}",
            if watch.last_error.is_none() {
                "verified"
            } else {
                "unavailable"
            }
        ),
    );
    Ok(watch)
}
#[tauri::command]
pub async fn run_price_watch(id: String, app: tauri::AppHandle) -> Result<PriceWatch, String> {
    run(&app, &id, false).await
}
#[cfg(test)]
mod tests {
    use super::*;
    fn offer(price: &str) -> PriceOffer {
        PriceOffer {
            product: "Widget".into(),
            url: "https://shop.test/product".into(),
            price: price.into(),
            currency: "USD".into(),
            availability: None,
            seller: None,
            is_aggregate: false,
            high_price: None,
        }
    }
    #[test]
    fn ambiguous_price_pages_never_trigger_a_cheapest_price() {
        assert!(exact_offer(&[offer("10"), offer("20")], "USD").is_err());
        let mut aggregate = offer("10");
        aggregate.is_aggregate = true;
        assert!(exact_offer(&[aggregate], "USD").is_err());
        assert_eq!(exact_offer(&[offer("10")], "USD").unwrap().price, "10");
        // Sizes often share one name, so stock that differs is some options, not a conflict.
        let mut unavailable = offer("10");
        unavailable.availability = Some("OutOfStock".into());
        assert_eq!(exact_offer(&[offer("10"), unavailable], "USD").unwrap().availability.as_deref(), Some("LimitedAvailability"));
    }
    #[test]
    fn a_new_watch_says_what_it_found() {
        assert_eq!(money("3199.95", "USD"), "$3,199.95");
        assert_eq!(money("129", "USD"), "$129");
        assert_eq!(money("24.5", "GBP"), "£24.50");
        assert_eq!(money("1000000", "JPY"), "JPY 1,000,000");
        let mut watch = PriceWatch { id: "w".into(), url: "https://shop.example.com/p".into(), label: "Z 8".into(), currency: "USD".into(), threshold: "3000".into(), interval_minutes: 60, enabled: true, created_at: now(), next_run_at: now(), last_checked_at: None, last_price: Some("3199.95".into()), last_error: None, last_availability: Some("InStock".into()), history: vec![] };
        let mut found = offer("3199.95");
        found.availability = Some("InStock".into());
        assert_eq!(first_look(&watch, &Ok(found.clone())), "It’s $3,199.95, in stock now.");
        watch.last_price = Some("2899".into());
        found.price = "2899".into();
        assert_eq!(first_look(&watch, &Ok(found)), "It’s $2,899, in stock now. That’s already at or below your $3,000 target.");
        assert_eq!(first_look(&watch, &Err("Nope.".into())), "I couldn’t read its price: Nope.");
    }
    #[test]
    fn options_at_one_price_have_one_price_to_watch() {
        let named = |product: &str, price: &str, availability: &str| {
            let mut one = offer(price);
            one.product = product.into();
            one.availability = Some(availability.into());
            one
        };
        let same = exact_offer(&[named("Era 100 - Black", "219", "InStock"), named("Era 100 - White", "219", "InStock")], "USD").unwrap();
        assert_eq!((same.price.as_str(), same.availability.as_deref()), ("219", Some("InStock")));
        let mixed = exact_offer(&[named("Runner - Size 8", "110", "OutOfStock"), named("Runner - Size 9", "110", "InStock")], "USD").unwrap();
        assert_eq!((mixed.price.as_str(), mixed.availability.as_deref()), ("110", Some("LimitedAvailability")));
        assert!(exact_offer(&[named("Board - Red", "199", "InStock"), named("Board - Blue", "219", "InStock")], "USD").is_err());
    }
    #[test]
    fn unchanged_watch_checks_are_quiet_and_stock_blocks_threshold() {
        let mut watch = PriceWatch {
            id: "watch".into(),
            url: "https://shop.example.com/product".into(),
            label: "Widget".into(),
            currency: "USD".into(),
            threshold: "10".into(),
            interval_minutes: 60,
            enabled: true,
            created_at: now(),
            next_run_at: now(),
            last_checked_at: Some(now()),
            last_price: Some("9.99".into()),
            last_error: None,
            last_availability: Some("https://schema.org/InStock".into()),
            history: vec![],
        };
        let mut after = watch.clone();
        after.last_checked_at = Some("later".into());
        after.history.push(PricePoint {
            price: "9.99".into(),
            currency: "USD".into(),
            observed_at: now(),
            url: watch.url.clone(),
        });
        assert!(!changed(&watch, &after));
        assert!(below(&watch));
        let before_cancel = serde_json::to_value(&watch).unwrap();
        assert!(record_observation(&mut watch, Ok(offer("1")), true).is_err());
        assert_eq!(serde_json::to_value(&watch).unwrap(), before_cancel, "cancellation must not overwrite a verified price or create a false recovery notification");
        assert!(
            record_observation(&mut watch, Err("Price check cancelled.".into()), true).is_err()
        );
        assert_eq!(serde_json::to_value(&watch).unwrap(), before_cancel);
        watch.last_availability = Some("https://schema.org/OutOfStock".into());
        assert!(!below(&watch));
    }
}
