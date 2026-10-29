//! Fixed schemas and routing contracts shared through `worker/tools.json`.
//! Contracts describe capabilities and existing verification boundaries. They never
//! authorize a call: controller policy, approval and executor validation remain
//! authoritative. Private metadata never enters provider schemas.
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::OnceLock,
};

pub const RESOURCES: &[&str] = &[
    "automation",
    "browser",
    "calendar",
    "currency",
    "files",
    "flights",
    "knowledge",
    "mail",
    "memory",
    "music",
    "notes",
    "places",
    "reminders",
    "shopping",
    "system",
    "timer",
    "weather",
    "web",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolOperation {
    Read,
    Write,
    Mixed,
}
impl ToolOperation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Mixed => "mixed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetryPolicy {
    RetryReadAfterFailure,
    ReadRequiresNewApproval,
    ReconcileBeforeRetry,
    ByOperation,
}
impl RetryPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RetryReadAfterFailure => "retry_read_after_failure",
            Self::ReadRequiresNewApproval => "read_requires_new_approval",
            Self::ReconcileBeforeRetry => "reconcile_before_retry",
            Self::ByOperation => "by_operation",
        }
    }
}

/// Identifiers of existing execution/verification boundaries, not model claims.
/// A DispatchReceipt proves less than a VerifiedAppReceipt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SuccessVerifier {
    CalculationResult,
    StructuredMathResult,
    ScopedFileResults,
    ScopedMemoryResults,
    SystemSnapshot,
    OnlineTaskResult,
    IntegrationResponse,
    AppReadResult,
    VerifiedAppReceipt,
    DispatchReceipt,
    ControlReceipt,
    MonitorReceipt,
    BrowserSnapshotReceipt,
}
impl SuccessVerifier {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CalculationResult => "calculation_result",
            Self::StructuredMathResult => "structured_math_result",
            Self::ScopedFileResults => "scoped_file_results",
            Self::ScopedMemoryResults => "scoped_memory_results",
            Self::SystemSnapshot => "system_snapshot",
            Self::OnlineTaskResult => "online_task_result",
            Self::IntegrationResponse => "integration_response",
            Self::AppReadResult => "app_read_result",
            Self::VerifiedAppReceipt => "verified_app_receipt",
            Self::DispatchReceipt => "dispatch_receipt",
            Self::ControlReceipt => "control_receipt",
            Self::MonitorReceipt => "monitor_receipt",
            Self::BrowserSnapshotReceipt => "browser_snapshot_receipt",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolExample {
    pub request: String,
    pub arguments: Value,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolContract {
    pub resources: Vec<String>,
    pub operation: ToolOperation,
    pub use_when: Vec<String>,
    pub avoid_when: Vec<String>,
    pub prerequisites: Vec<String>,
    pub output_kind: String,
    pub success_verifier: SuccessVerifier,
    /// What the named verifier actually establishes, including weaker dispatch receipts.
    pub verification_scope: String,
    pub retry_policy: RetryPolicy,
    pub examples: Vec<ToolExample>,
}
impl ToolContract {
    pub fn has_resource(&self, resource: &str) -> bool {
        self.resources.iter().any(|item| item == resource)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ToolDefinition {
    name: String,
    description: String,
    parameters: Value,
    contract: ToolContract,
}
type Catalog = BTreeMap<String, ToolDefinition>;

fn all() -> &'static Catalog {
    static TOOLS: OnceLock<Catalog> = OnceLock::new();
    TOOLS.get_or_init(|| {
        parse_catalog(include_str!("../../worker/tools.json"))
            .expect("worker/tools.json must contain valid fixed schemas and contracts")
    })
}
fn nonempty(value: &str) -> bool {
    !value.trim().is_empty()
}

/// A misspelled resource or example that no longer satisfies its schema must not
/// silently change routing. Validate the shipped catalog once when loading it.
fn parse_catalog(raw: &str) -> Result<Catalog, String> {
    let catalog: Catalog = serde_json::from_str(raw).map_err(|error| error.to_string())?;
    if catalog.is_empty() {
        return Err("Tool catalog must not be empty".into());
    }
    for (name, tool) in &catalog {
        let fail = |detail: &str| format!("{name}: {detail}");
        if name != &tool.name || !nonempty(&tool.description) {
            return Err(fail("invalid name or description"));
        }
        let c = &tool.contract;
        if c.resources.is_empty()
            || c.resources.iter().any(|r| !RESOURCES.contains(&r.as_str()))
            || c.resources.iter().collect::<BTreeSet<_>>().len() != c.resources.len()
        {
            return Err(fail("resources must be unique known domains"));
        }
        if [&c.use_when, &c.avoid_when]
            .iter()
            .any(|values| values.is_empty())
            || [&c.use_when, &c.avoid_when, &c.prerequisites]
                .iter()
                .any(|values| values.iter().any(|v| !nonempty(v)))
            || !nonempty(&c.output_kind)
            || !nonempty(&c.verification_scope)
        {
            return Err(fail("contract guidance must be explicit and nonempty"));
        }
        let valid_retry = match c.operation {
            ToolOperation::Read => matches!(
                c.retry_policy,
                RetryPolicy::RetryReadAfterFailure | RetryPolicy::ReadRequiresNewApproval
            ),
            ToolOperation::Write => c.retry_policy == RetryPolicy::ReconcileBeforeRetry,
            ToolOperation::Mixed => c.retry_policy == RetryPolicy::ByOperation,
        };
        if !valid_retry {
            return Err(fail("retry policy does not match operation"));
        }
        let schema = &tool.parameters;
        if schema["type"] != "object"
            || schema["additionalProperties"] != false
            || !schema["properties"].is_object()
        {
            return Err(fail("parameters must be a closed object schema"));
        }
        let properties = schema["properties"].as_object().unwrap();
        if let Some(required) = schema.get("required") {
            let required = required
                .as_array()
                .ok_or_else(|| fail("required must be an array"))?;
            if required.iter().any(|field| {
                field
                    .as_str()
                    .is_none_or(|field| !properties.contains_key(field))
            }) || required
                .iter()
                .filter_map(Value::as_str)
                .collect::<BTreeSet<_>>()
                .len()
                != required.len()
            {
                return Err(fail("required contains unknown or duplicate fields"));
            }
        }
        if properties.values().any(|spec| {
            !matches!(
                spec["type"].as_str(),
                Some("string" | "integer" | "number" | "boolean" | "array")
            ) || (spec["type"] == "array"
                && (spec["items"]["type"] != "string"
                    || spec["maxItems"]
                        .as_u64()
                        .is_none_or(|max| max > 32 || max == 0)
                    || spec["minItems"].as_u64().unwrap_or(0)
                        > spec["maxItems"].as_u64().unwrap_or(0)
                    || spec["items"]["maxLength"]
                        .as_u64()
                        .is_none_or(|max| max > 4096 || max == 0)))
        }) {
            return Err(fail("unsupported argument schema type"));
        }
        if c.examples.is_empty() {
            return Err(fail("at least one example is required"));
        }
        for example in &c.examples {
            if !nonempty(&example.request) {
                return Err(fail("example request must not be empty"));
            }
            validate_example(schema, &example.arguments).map_err(|detail| fail(&detail))?;
        }
    }
    Ok(catalog)
}

/// Catalog examples only. Actual calls use stricter domain validators before
/// approval/execution; a schema-valid example does not authorize any operation.
fn validate_example(schema: &Value, arguments: &Value) -> Result<(), String> {
    let arguments = arguments
        .as_object()
        .ok_or("example arguments must be an object")?;
    let properties = schema["properties"]
        .as_object()
        .ok_or("missing properties")?;
    if arguments.keys().any(|key| !properties.contains_key(key))
        || schema
            .get("required")
            .and_then(Value::as_array)
            .is_some_and(|required| {
                required
                    .iter()
                    .any(|key| !arguments.contains_key(key.as_str().unwrap_or("")))
            })
    {
        return Err("example has missing or unknown arguments".into());
    }
    for (key, value) in arguments {
        let spec = &properties[key];
        let valid = match spec["type"].as_str() {
            Some("string") => value.as_str().is_some_and(|text| {
                let length = text.chars().count() as u64;
                length >= spec["minLength"].as_u64().unwrap_or(0)
                    && length <= spec["maxLength"].as_u64().unwrap_or(u64::MAX)
            }),
            Some("boolean") => value.is_boolean(),
            Some("array") => value.as_array().is_some_and(|items| {
                items.len() as u64 >= spec["minItems"].as_u64().unwrap_or(0)
                    && items.len() as u64 <= spec["maxItems"].as_u64().unwrap_or(0)
                    && items.iter().all(|item| {
                        item.as_str().is_some_and(|text| {
                            let length = text.chars().count() as u64;
                            length >= spec["items"]["minLength"].as_u64().unwrap_or(0)
                                && length <= spec["items"]["maxLength"].as_u64().unwrap_or(0)
                        })
                    })
            }),
            Some(kind @ ("integer" | "number")) => value.as_f64().is_some_and(|number| {
                number.is_finite()
                    && (kind != "integer" || value.is_i64() || value.is_u64())
                    && number >= spec["minimum"].as_f64().unwrap_or(f64::NEG_INFINITY)
                    && number <= spec["maximum"].as_f64().unwrap_or(f64::INFINITY)
            }),
            _ => false,
        };
        let allowed = spec.get("enum").is_none_or(|choices| {
            choices
                .as_array()
                .is_some_and(|choices| choices.contains(value))
        });
        if !valid || !allowed {
            return Err(format!("example argument {key} violates its schema"));
        }
    }
    Ok(())
}

/// Fixed, typed routing metadata. A contract is not permission to execute a tool.
pub fn contract(name: &str) -> Option<&'static ToolContract> {
    all().get(name).map(|tool| &tool.contract)
}

/// Canonical name borrowed from the validated catalog (never a model-owned string).
pub fn name(name: &str) -> Option<&'static str> {
    all().get(name).map(|tool| tool.name.as_str())
}

/// Provider-compatible definition containing only name, description and parameters.
pub fn definition(name: &str) -> Option<Value> {
    all().get(name).map(|tool| json!({"name":tool.name, "description":tool.description, "parameters":tool.parameters}))
}

/// Definitions in requested order, skipping unknown names.
pub fn offer(names: &[&str]) -> Value {
    Value::Array(names.iter().filter_map(|name| definition(name)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_the_app_offers_has_a_shared_schema_and_contract() {
        for name in [
            "calculate",
            "split_costs",
            "scale_quantities",
            "compare_options",
            "sum_by_status",
            "search_files",
            "search_memory",
            "system_info",
            "look_up_online",
            "remember",
        ]
        .into_iter()
        .chain(crate::actions::TOOLS)
        .chain(crate::monitor_actions::TOOLS)
        .chain(crate::control_actions::TOOLS)
        .chain(crate::browser_workspace::TOOLS)
        .chain(crate::apps::READ_TOOLS)
        .chain(crate::integrations::TOOLS.iter().map(|tool| tool.name))
        {
            let tool =
                definition(name).unwrap_or_else(|| panic!("{name} missing from worker/tools.json"));
            assert_eq!(tool["name"], name);
            assert_eq!(tool["parameters"]["type"], "object");
            assert_eq!(tool["parameters"]["additionalProperties"], false);
            assert!(contract(name).is_some());
            let keys: BTreeSet<_> = tool
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect();
            assert_eq!(keys, BTreeSet::from(["name", "description", "parameters"]));
        }
        assert_eq!(offer(&["system_info", "nope"]).as_array().unwrap().len(), 1);
        assert!(contract("nope").is_none());
    }

    #[test]
    fn calculation_schema_is_a_bounded_string_array_with_a_real_verifier() {
        let definition = definition("calculate").unwrap();
        let schema = &definition["parameters"];
        assert_eq!(
            contract("calculate").unwrap().operation,
            ToolOperation::Read
        );
        assert_eq!(
            contract("calculate").unwrap().success_verifier,
            SuccessVerifier::CalculationResult
        );
        assert_eq!(contract("calculate").unwrap().resources, vec!["knowledge"]);
        assert!(validate_example(schema, &json!({"expressions":["18+15","(240+84)/4"]})).is_ok());
        for invalid in [
            json!({"expressions":[]}),
            json!({"expressions":vec!["1";9]}),
            json!({"expressions":[false]}),
            json!({"expressions":["x".repeat(257)]}),
        ] {
            assert!(validate_example(schema, &invalid).is_err(), "{invalid}");
        }
        let mut catalog: Value =
            serde_json::from_str(include_str!("../../worker/tools.json")).unwrap();
        catalog["calculate"]["parameters"]["properties"]["expressions"]["items"]["type"] =
            "object".into();
        assert!(
            parse_catalog(&catalog.to_string()).is_err(),
            "nested/arbitrary arrays cannot silently enter fixed schemas"
        );
    }

    #[test]
    fn structured_math_catalog_uses_bounded_flat_rows_and_recomputed_receipts() {
        for name in [
            "split_costs",
            "scale_quantities",
            "compare_options",
            "sum_by_status",
        ] {
            let tool = definition(name).unwrap();
            let contract = contract(name).unwrap();
            assert_eq!(contract.operation, ToolOperation::Read);
            assert_eq!(
                contract.success_verifier,
                SuccessVerifier::StructuredMathResult
            );
            assert_eq!(contract.resources, vec!["knowledge"]);
            for spec in tool["parameters"]["properties"]
                .as_object()
                .unwrap()
                .values()
            {
                if spec["type"] == "array" {
                    assert_eq!(spec["items"]["type"], "string");
                    assert!(spec["maxItems"].as_u64().unwrap() <= 24);
                    assert!(spec["items"]["maxLength"].as_u64().unwrap() <= 512);
                }
            }
            for example in &contract.examples {
                assert!(crate::structured_math::validate(name, &example.arguments).is_ok());
                let evidence = crate::structured_math::run(name, &example.arguments).unwrap();
                assert!(crate::structured_math::verify(
                    name,
                    &example.arguments,
                    &evidence
                ));
                assert!(!crate::structured_math::render(&evidence).is_empty());
            }
        }
    }

    #[test]
    fn catalog_rejects_missing_contract_unknown_domains_and_invalid_verifiers() {
        let original: Value =
            serde_json::from_str(include_str!("../../worker/tools.json")).unwrap();
        for (field, replacement) in [
            ("resources", json!(["calender"])),
            ("resources", json!(["calendar", "calendar"])),
            ("success_verifier", json!("model_said_done")),
            ("retry_policy", json!("retry_read_after_failure")),
            ("use_when", json!([])),
        ] {
            let mut altered = original.clone();
            altered["add_reminder"]["contract"][field] = replacement;
            assert!(
                parse_catalog(&altered.to_string()).is_err(),
                "accepted invalid {field}"
            );
        }
        let mut missing = original;
        missing["add_reminder"]
            .as_object_mut()
            .unwrap()
            .remove("contract");
        assert!(parse_catalog(&missing.to_string()).is_err());
    }

    #[test]
    fn examples_cannot_drift_from_actual_argument_schemas() {
        let original: Value =
            serde_json::from_str(include_str!("../../worker/tools.json")).unwrap();
        for (tool, arguments) in [
            ("add_reminder", json!({"due":"2026-11-03 09:00"})),
            ("add_reminder", json!({"title":"Call Ana","list":"Work"})),
            ("read_mail", json!({"unread":"true"})),
            ("control_music", json!({"action":"shuffle"})),
            ("set_timer", json!({"seconds":1.5})),
            ("set_timer", json!({"seconds":0})),
            ("search_files", json!({"query":""})),
        ] {
            let mut altered = original.clone();
            altered[tool]["contract"]["examples"][0]["arguments"] = arguments;
            assert!(
                parse_catalog(&altered.to_string()).is_err(),
                "accepted invalid {tool} example"
            );
        }
    }

    #[test]
    fn contracts_keep_reads_mixed_operations_and_weak_receipts_distinct() {
        assert_eq!(
            contract("read_calendar").unwrap().operation,
            ToolOperation::Read
        );
        assert!(contract("read_calendar").unwrap().has_resource("calendar"));
        assert!(!contract("read_calendar").unwrap().has_resource("mail"));
        assert_eq!(
            contract("control_music").unwrap().operation,
            ToolOperation::Mixed
        );
        assert_eq!(
            contract("control_music").unwrap().retry_policy,
            RetryPolicy::ByOperation
        );
        assert_eq!(
            contract("browse_page").unwrap().retry_policy,
            RetryPolicy::ReadRequiresNewApproval
        );
        for name in ["open_app", "open_link", "run_shortcut"] {
            assert_eq!(
                contract(name).unwrap().success_verifier,
                SuccessVerifier::DispatchReceipt
            );
        }
        for name in [
            "create_note",
            "draft_email",
            "add_reminder",
            "add_calendar_event",
        ] {
            assert_eq!(
                contract(name).unwrap().success_verifier,
                SuccessVerifier::VerifiedAppReceipt
            );
        }
        for tool in all().values() {
            assert!(!tool.contract.operation.as_str().is_empty());
            assert!(!tool.contract.retry_policy.as_str().is_empty());
            assert!(!tool.contract.success_verifier.as_str().is_empty());
        }
    }
}
