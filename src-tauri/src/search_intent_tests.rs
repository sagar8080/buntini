//! Independent fixture corpus for the public search-routing contract. There is
//! no browser, model, Keychain or provider traffic in these tests.
use crate::search_intent::{browser_navigation, explicit_request, SearchIntent};
use serde_json::Value;

fn corpus() -> Value {
    serde_json::from_str(include_str!("../../evals/search-intent-cases.json")).unwrap()
}

#[test]
fn held_out_research_routes_preserve_the_whole_goal_and_exact_query() {
    for case in corpus()["research"].as_array().unwrap() {
        let request = case["request"].as_str().unwrap();
        let intent =
            explicit_request(request).unwrap_or_else(|| panic!("Did not route: {request}"));
        assert_eq!(intent.goal, request, "{request}");
        assert_eq!(intent.query, case["query"].as_str().unwrap(), "{request}");
        assert_eq!(
            intent.purpose,
            case["purpose"].as_str().unwrap(),
            "{request}"
        );
        let domains: Vec<&str> = case["domains"]
            .as_array()
            .map(|items| items.iter().map(|item| item.as_str().unwrap()).collect())
            .unwrap_or_default();
        assert_eq!(intent.preferred_domains, domains, "{request}");
        assert!(
            browser_navigation(request).is_none(),
            "Research was mistaken for navigation: {request}"
        );
    }
}

#[test]
fn held_out_private_negated_and_informational_requests_do_not_dispatch_search() {
    for request in corpus()["notResearch"].as_array().unwrap() {
        let request = request.as_str().unwrap();
        assert!(
            explicit_request(request).is_none(),
            "Unexpected online authority: {request}"
        );
    }
}

#[test]
fn held_out_navigation_requires_an_explicit_request_to_open_results() {
    for case in corpus()["navigation"].as_array().unwrap() {
        let request = case["request"].as_str().unwrap();
        let (engine, query) =
            browser_navigation(request).unwrap_or_else(|| panic!("Did not route: {request}"));
        assert_eq!(engine, case["engine"].as_str().unwrap(), "{request}");
        assert_eq!(query, case["query"].as_str().unwrap(), "{request}");
        assert!(
            explicit_request(request).is_none(),
            "Navigation changed into research: {request}"
        );
    }
    for request in corpus()["notNavigation"].as_array().unwrap() {
        let request = request.as_str().unwrap();
        assert!(
            browser_navigation(request).is_none(),
            "Unexpected navigation authority: {request}"
        );
    }
}

#[test]
fn held_out_research_requirements_remain_literal_spans_and_preserve_query_scope() {
    for case in corpus()["requirements"].as_array().unwrap() {
        let goal = case["goal"].as_str().unwrap();
        let query = case["query"].as_str().unwrap();
        let intent = SearchIntent::from_lookup(goal, query);
        assert_eq!(intent.goal, goal);
        assert_eq!(intent.query, query);
        assert_eq!(intent.purpose, case["purpose"].as_str().unwrap(), "{goal}");
        let mut kinds: Vec<&str> = intent
            .requirements
            .iter()
            .map(|requirement| {
                assert_eq!(
                    goal.get(requirement.start..requirement.end),
                    Some(requirement.text.as_str()),
                    "Invalid evidence span: {goal}"
                );
                requirement.kind.as_str()
            })
            .collect();
        kinds.sort();
        kinds.dedup();
        let mut expected: Vec<&str> = case["kinds"]
            .as_array()
            .unwrap()
            .iter()
            .map(|kind| kind.as_str().unwrap())
            .collect();
        expected.sort();
        assert_eq!(kinds, expected, "{goal}");
    }
}
