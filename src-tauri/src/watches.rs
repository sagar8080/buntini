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
        return Err("No supported structured price was published in the selected currency. JavaScript-only prices and free-text estimates are not tracked.".into());
    };
    if matching.iter().any(|offer| offer.is_aggregate) {
        return Err(
            "This page publishes a price range. Choose a page with one exact product and price."
                .into(),
        );
    }
    if matching.iter().any(|offer| {
        offer.product.to_lowercase() != first.product.to_lowercase()
            || offer.price != first.price
            || offer
                .availability
                .as_ref()
                .map(|value| value.to_lowercase())
                != first
                    .availability
                    .as_ref()
                    .map(|value| value.to_lowercase())
            || offer.seller.as_ref().map(|value| value.to_lowercase())
                != first.seller.as_ref().map(|value| value.to_lowercase())
    }) {
        return Err("This page publishes conflicting products or prices. Choose a dedicated product variant page.".into());
    }
    Ok((*first).clone())
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
pub(crate) async fn run(
    app: &tauri::AppHandle,
    id: &str,
    scheduled: bool,
) -> Result<PriceWatch, String> {
    let state = app.state::<AppState>();
    let token = Arc::new(AtomicBool::new(false));
    let active_id = format!("price-watch-{id}");
    let before = {
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
        watch.next_run_at = next(watch.interval_minutes);
        store.save_price_watch(&watch)?;
        active.insert(active_id.clone(), token.clone());
        before
    };
    let _claim = crate::notifications::ActiveMonitor::new(&state, active_id, token.clone());
    state.audit("price_watch.fetch", id);
    let fetch = crate::research::fetch_public_html(&before.url);
    tokio::pin!(fetch);
    let result = loop {
        // Check before the first poll as well as after every timer wakeup, so
        // a revoked queued check cannot start its initial DNS/HTTP work.
        if token.load(Ordering::SeqCst) {
            break Err("Price check cancelled.".into());
        }
        tokio::select! {
            result = &mut fetch => break result,
            _ = tokio::time::sleep(std::time::Duration::from_millis(40)) => {}
        }
    }
    .and_then(|page| crate::pricing::extract_offers(&page.html, &page.url))
    .and_then(|offers| exact_offer(&offers, &before.currency));
    let result = if token.load(Ordering::Relaxed) {
        Err("Price check cancelled.".into())
    } else {
        result
    };
    let persisted = (|| -> Result<PriceWatch, String> {
        let store = state.store.lock().unwrap();
        match store.price_watch(id)? {
            None => Err("This price watch was deleted while checking.".into()),
            Some(mut watch) => {
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
                store.save_price_watch(&watch).map(|_| watch)
            }
        }
    })();
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
        let mut unavailable = offer("10");
        unavailable.availability = Some("OutOfStock".into());
        assert!(exact_offer(&[offer("10"), unavailable], "USD").is_err());
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
        watch.last_availability = Some("https://schema.org/OutOfStock".into());
        assert!(!below(&watch));
    }
}
