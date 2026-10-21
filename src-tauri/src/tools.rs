//! Tool schemas shared with the Python worker and the Apple Intelligence helper
//! (`worker/tools.json`). Models only ever see these fixed definitions; the controller
//! still validates every call it receives.
use serde_json::Value;
use std::sync::OnceLock;

fn all() -> &'static serde_json::Map<String, Value> {
    static TOOLS: OnceLock<serde_json::Map<String, Value>> = OnceLock::new();
    TOOLS.get_or_init(|| serde_json::from_str(include_str!("../../worker/tools.json")).expect("worker/tools.json is valid JSON"))
}

/// The full definition (`name`, `description`, `parameters`) of a known tool.
pub fn definition(name: &str) -> Option<Value> {
    all().get(name).cloned()
}

/// Definitions for the named tools, in order, skipping unknown names.
pub fn offer(names: &[&str]) -> Value {
    Value::Array(names.iter().filter_map(|name| definition(name)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_the_app_offers_has_a_shared_schema() {
        for name in ["search_files", "system_info"].into_iter().chain(crate::integrations::TOOLS.iter().map(|tool| tool.name)) {
            let tool = definition(name).unwrap_or_else(|| panic!("{name} missing from worker/tools.json"));
            assert_eq!(tool["name"], name);
            assert_eq!(tool["parameters"]["type"], "object");
            assert_eq!(tool["parameters"]["additionalProperties"], false, "{name} must reject unknown arguments");
        }
        assert_eq!(offer(&["system_info", "nope"]).as_array().unwrap().len(), 1);
    }
}
