//! Search spending guardrails: turns settings into the limits each online service is
//! held to. Every request is reserved before it is sent (see `Store::reserve`).
use crate::types::Settings;

/// Free allowances and the price of each search past them, as published in October 2026.
/// Spending is estimated from this table, so nobody has to look up or type a price.
pub struct Preset {
    pub provider: &'static str,
    pub free_monthly: u32,
    pub cost_per_search: f64,
}
pub const PRESETS: [Preset; 2] = [
    // $5 per 1,000 requests with a $5 monthly credit (a card is required).
    Preset {
        provider: "brave",
        free_monthly: 1000,
        cost_per_search: 0.005,
    },
    // 1,000 free credits a month; a basic search costs one credit, then $0.008.
    Preset {
        provider: "tavily",
        free_monthly: 1000,
        cost_per_search: 0.008,
    },
];

/// The most lookups one month may hold, whatever the budget would buy.
const MAX_MONTHLY: u32 = 10_000;

pub fn preset(provider: &str) -> Option<&'static Preset> {
    PRESETS.iter().find(|preset| preset.provider == provider)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Limits {
    pub daily: u32,
    pub monthly: u32,
    pub budget_usd: f64,
    pub unit_cost: f64,
}

/// Limits for the web search provider. Free-tier mode stays inside the provider's free
/// allowance and never estimates any spend. Past it, the monthly budget is the one cap the
/// person sets: the count is what that budget buys at the table price.
pub fn search_limits(settings: &Settings) -> Limits {
    let daily = settings.daily_search_limit;
    match (settings.free_tier_only, preset(&settings.search_provider)) {
        (true, Some(preset)) => Limits {
            daily,
            monthly: settings.monthly_search_limit.min(preset.free_monthly),
            budget_usd: 0.0,
            unit_cost: 0.0,
        },
        (_, Some(preset)) => Limits {
            daily,
            monthly: (settings.monthly_budget_usd / preset.cost_per_search + 1e-9)
                .floor()
                .min(MAX_MONTHLY as f64) as u32,
            budget_usd: settings.monthly_budget_usd,
            unit_cost: preset.cost_per_search,
        },
        // Settings only allow providers in the table; anything else gets no lookups.
        (_, None) => Limits {
            daily,
            monthly: 0,
            budget_usd: 0.0,
            unit_cost: 0.0,
        },
    }
}

/// Keyed lookups (eBay, Duffel) have no per-search fee in their free tiers; they are
/// counted on their own so they never use up the search provider's allowance.
pub fn service_limits(settings: &Settings) -> Limits {
    Limits {
        daily: settings.daily_search_limit,
        monthly: settings.monthly_search_limit,
        budget_usd: f64::INFINITY,
        unit_cost: 0.0,
    }
}

/// The plain-language notice shown once a month when usage crosses 80%.
pub fn display_name(provider: &str) -> &str {
    match provider {
        "brave" => "Brave Search",
        "tavily" => "Tavily",
        "ebay" => "eBay",
        "duffel" => "Duffel",
        other => other
            .strip_prefix("cloud-")
            .and_then(crate::cloud::provider)
            .map(|cloud| cloud.name)
            .unwrap_or(other),
    }
}

/// Limits for a cloud model call: only the monthly dollar cap applies.
pub fn cloud_limits(settings: &Settings, estimate_usd: f64) -> Limits {
    Limits {
        daily: u32::MAX,
        monthly: u32::MAX,
        budget_usd: settings.cloud_budget_usd,
        unit_cost: estimate_usd,
    }
}

pub fn warning(provider: &str, month_used: u32, limits: Limits, spend: f64) -> String {
    let name = display_name(provider);
    if limits.budget_usd.is_finite()
        && limits.budget_usd > 0.0
        && spend / limits.budget_usd >= month_used as f64 / limits.monthly.max(1) as f64
    {
        format!("Heads up: you’ve used about {:.0}% of this month’s {name} budget (${spend:.2} of ${:.2}).", spend / limits.budget_usd * 100.0, limits.budget_usd)
    } else {
        format!(
            "Heads up: you’ve used {month_used} of this month’s {} {name} searches.",
            limits.monthly
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_tier_mode_stays_inside_the_provider_allowance() {
        let mut settings = Settings {
            search_provider: "tavily".into(),
            free_tier_only: true,
            monthly_search_limit: 5000,
            monthly_budget_usd: 50.0,
            daily_search_limit: 30,
            ..Settings::default()
        };
        assert_eq!(
            search_limits(&settings),
            Limits {
                daily: 30,
                monthly: 1000,
                budget_usd: 0.0,
                unit_cost: 0.0
            }
        );
        settings.monthly_search_limit = 200;
        assert_eq!(
            search_limits(&settings).monthly,
            200,
            "a lower cap saved by an earlier version still wins"
        );
    }

    #[test]
    fn past_the_free_allowance_the_table_price_and_the_budget_set_the_limits() {
        let mut settings = Settings {
            search_provider: "tavily".into(),
            free_tier_only: false,
            monthly_search_limit: 200,
            monthly_budget_usd: 50.0,
            // A price typed into an earlier version no longer counts.
            cost_per_search_usd: 3.0,
            ..Settings::default()
        };
        assert_eq!(
            search_limits(&settings),
            Limits {
                daily: 30,
                monthly: 6250,
                budget_usd: 50.0,
                unit_cost: 0.008
            }
        );
        settings.search_provider = "brave".into();
        settings.monthly_budget_usd = 5.0;
        assert_eq!(search_limits(&settings).monthly, 1000);
        assert_eq!(search_limits(&settings).unit_cost, 0.005);
        settings.monthly_budget_usd = 500.0;
        assert_eq!(search_limits(&settings).monthly, MAX_MONTHLY);
        settings.monthly_budget_usd = 0.0;
        assert_eq!(search_limits(&settings).monthly, 0, "no budget, no paid lookups");
    }

    #[test]
    fn services_are_never_charged_against_the_search_budget() {
        let limits = service_limits(&Settings::default());
        assert_eq!(limits.unit_cost, 0.0);
        assert!(limits.budget_usd.is_infinite());
    }

    #[test]
    fn warnings_name_the_tighter_limit() {
        let count = Limits {
            daily: 30,
            monthly: 1000,
            budget_usd: 0.0,
            unit_cost: 0.0,
        };
        assert_eq!(
            warning("tavily", 800, count, 0.0),
            "Heads up: you’ve used 800 of this month’s 1000 Tavily searches."
        );
        let money = Limits {
            daily: 30,
            monthly: 1000,
            budget_usd: 5.0,
            unit_cost: 0.005,
        };
        assert!(
            warning("brave", 100, money, 4.1).contains("82% of this month’s Brave Search budget")
        );
        let cloud = cloud_limits(&Settings::default(), 0.02);
        assert!(warning("cloud-anthropic", 300, cloud, 4.2)
            .contains("84% of this month’s Anthropic budget"));
    }
}
