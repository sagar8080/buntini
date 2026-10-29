//! Boundaries for the privileged companion WebView.

pub(crate) fn ui_navigation_allowed(url: &reqwest::Url, development: bool) -> bool {
    if !url.username().is_empty() || url.password().is_some() {
        return false;
    }
    match (url.scheme(), url.host_str(), url.port()) {
        ("tauri", Some("localhost"), None) => true,
        ("http", Some("127.0.0.1"), Some(1421)) => development,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::collections::BTreeSet;

    #[test]
    fn privileged_ui_only_navigates_to_bundled_content() {
        for allowed in [
            "tauri://localhost/",
            "tauri://localhost/index.html#settings",
        ] {
            assert!(
                ui_navigation_allowed(&allowed.parse().unwrap(), false),
                "{allowed}"
            );
        }
        for blocked in [
            "https://example.com/",
            "http://tauri.localhost/",
            "https://tauri.localhost/",
            "https://tauri.localhost.evil.com/",
            "tauri://evil.com/",
            "tauri://localhost:443/",
            "http://localhost:1421/",
            "http://127.0.0.1:1422/",
            "http://127.0.0.1:1421/",
            "https://user@tauri.localhost/",
            "file:///etc/passwd",
            "data:text/html,hello",
            "javascript:alert(1)",
            "about:blank",
        ] {
            assert!(
                !ui_navigation_allowed(&blocked.parse().unwrap(), false),
                "{blocked}"
            );
        }
        assert!(ui_navigation_allowed(
            &"http://127.0.0.1:1421/".parse().unwrap(),
            true
        ));
        assert!(!ui_navigation_allowed(
            &"http://127.0.0.1:1422/".parse().unwrap(),
            true
        ));
    }

    #[test]
    fn every_controller_command_is_explicitly_granted_only_to_main() {
        // A new registered command must be reviewed and added to the allowlist;
        // deleting the permission manifest would silently restore Tauri defaults.
        let source = include_str!("lib.rs");
        let handler = source
            .split("tauri::generate_handler![")
            .nth(1)
            .unwrap()
            .split("])")
            .next()
            .unwrap();
        let registered: BTreeSet<&str> = handler
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.rsplit("::").next().unwrap())
            .collect();
        let manifest: Value =
            serde_json::from_str(include_str!("../permissions/companion.json")).unwrap();
        let permission = &manifest["permission"][0];
        assert_eq!(permission["identifier"], "companion-ui");
        let allowed: BTreeSet<&str> = permission["commands"]["allow"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s.as_str().unwrap())
            .collect();
        assert!(!allowed.is_empty());
        assert_eq!(registered, allowed);
        let capability: Value =
            serde_json::from_str(include_str!("../capabilities/default.json")).unwrap();
        assert_eq!(capability["windows"], serde_json::json!(["main"]));
        assert!(capability.get("remote").is_none());
        assert!(capability["permissions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p == "companion-ui"));
        let config: Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(
            config["app"]["windows"][0]["create"], false,
            "main must be constructed with navigation/popup/download guards"
        );
        assert_eq!(
            config["app"]["security"]["capabilities"],
            serde_json::json!(["main"])
        );
    }
}
