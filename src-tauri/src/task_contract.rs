//! Durable intent and evidence bookkeeping. The bootstrap is deliberately
//! conservative English extraction, not a claim of general intent understanding.
//! Only the saved user request can create outcomes; tool output and model text
//! can fill proposed values, never grant permission for additional writes.
use crate::{
    actions::{ActionOutcome, ActionReceipt},
    completion,
    workflows::Step,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TaskContract {
    pub version: u32,
    pub original_request: String,
    pub current_request: String,
    pub revision: u32,
    pub interpretation_date: String,
    pub intent_status: String,
    pub outcomes: Vec<Outcome>,
    pub unresolved_fields: Vec<UnresolvedField>,
    pub constraints: Vec<Constraint>,
    pub provenance: BTreeMap<String, BTreeMap<String, FieldProvenance>>,
    pub revisions: Vec<RequestRevision>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Outcome {
    pub id: String,
    pub kind: String,
    pub description: String,
    pub tool: Option<String>,
    pub accepted_tools: Vec<String>,
    pub resources: Vec<String>,
    pub dependencies: Vec<String>,
    pub status: String,
    pub required_arguments: Value,
    pub required_fields: Vec<String>,
    pub completed_by: Vec<String>,
    pub source_revision: u32,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct UnresolvedField {
    pub outcome_id: String,
    pub field: String,
    pub reason: String,
    pub status: String,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Constraint {
    pub outcome_id: Option<String>,
    pub field: String,
    pub value: Value,
    pub source: String,
    pub revision: u32,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RequestRevision {
    pub revision: u32,
    pub request: String,
    pub field_updates: Value,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct FieldProvenance {
    pub value: Value,
    pub origin: String,
    pub source_step_id: Option<String>,
    pub record_id: Option<String>,
    pub field_ref: Option<String>,
    pub calculation: Option<Value>,
    pub source: Option<Value>,
    pub approved: bool,
    /// An executor receipt confirms execution of the approved value. Approval
    /// alone is not independent evidence that a model-proposed value is correct.
    pub verified: bool,
    pub previous: Option<Box<FieldProvenance>>,
}

fn tokens(text: &str) -> Vec<&str> {
    text.split(|c: char| !c.is_alphanumeric() && c != '-')
        .filter(|s| !s.is_empty())
        .collect()
}
fn dates(text: &str) -> Vec<String> {
    tokens(text)
        .into_iter()
        .filter(|word| chrono::NaiveDate::parse_from_str(word, "%Y-%m-%d").is_ok())
        .map(str::to_owned)
        .collect()
}
fn calendar_days(text: &str, base: chrono::NaiveDate) -> Vec<String> {
    let mut result = Vec::new();
    for word in tokens(text) {
        let day = chrono::NaiveDate::parse_from_str(word, "%Y-%m-%d")
            .ok()
            .or_else(|| match word {
                "today" => Some(base),
                "tomorrow" => base.succ_opt(),
                "yesterday" => base.pred_opt(),
                _ => None,
            });
        if let Some(day) = day {
            let day = day.format("%Y-%m-%d").to_string();
            if !result.contains(&day) {
                result.push(day);
            }
        }
    }
    result
}

fn clauses(goal: &str) -> Vec<String> {
    let text = completion::unquoted(&goal.replace("Additional details: ", "\n"));
    let mut result = Vec::new();
    for sentence in text.split([';', '\n', '.', '!', '?']) {
        let sentence = completion::request_clause(sentence);
        // Explanation/negation scope must be preserved before conjunctions.
        if completion::non_action(sentence) {
            result.push(sentence.into());
            continue;
        }
        let separated = sentence
            .replace(" and then ", ";")
            .replace(", then ", ";")
            .replace(" then ", ";")
            .replace(" also ", ";");
        for part in separated.split(';') {
            // A literal unquoted note body may contain "and". Keep it intact.
            if part.contains(" with body ") {
                result.push(part.trim().trim_matches(',').into());
                continue;
            }
            let pieces: Vec<&str> = part.split(" and ").collect();
            let mut current = String::new();
            for piece in pieces {
                let candidate = completion::request_clause(piece);
                let starts_instruction = tokens(candidate).first().is_some_and(|word| {
                    [
                        "read",
                        "check",
                        "find",
                        "search",
                        "list",
                        "review",
                        "compare",
                        "create",
                        "add",
                        "set",
                        "make",
                        "save",
                        "draft",
                        "open",
                        "remind",
                        "remember",
                        "play",
                        "pause",
                        "switch",
                        "explain",
                        "summarize",
                        "summarise",
                        "calculate",
                        "translate",
                        "tell",
                        "what",
                        "why",
                        "how",
                        "send",
                        "buy",
                        "book",
                    ]
                    .contains(word)
                });
                let inherited_read = tokens(&current)
                    .first()
                    .copied()
                    .filter(|word| {
                        [
                            "read",
                            "check",
                            "find",
                            "search",
                            "list",
                            "review",
                            "compare",
                            "summarize",
                            "summarise",
                        ]
                        .contains(word)
                    })
                    .map(str::to_owned);
                let coordinated_source = !starts_instruction
                    && inherited_read.is_some()
                    && tokens(candidate)
                        .first()
                        .is_some_and(|word| ["my", "the", "this", "these"].contains(word))
                    && tokens(candidate).iter().any(|word| {
                        [
                            "calendar",
                            "email",
                            "emails",
                            "mail",
                            "inbox",
                            "notes",
                            "reminders",
                            "tabs",
                            "files",
                            "folders",
                        ]
                        .contains(word)
                    });
                let candidate = if coordinated_source {
                    format!("{} {candidate}", inherited_read.unwrap())
                } else {
                    candidate.to_string()
                };
                if (starts_instruction || coordinated_source) && !current.is_empty() {
                    result.push(current.trim().trim_matches(',').into());
                    current.clear();
                }
                if !current.is_empty() {
                    current.push_str(" and ");
                }
                current.push_str(&candidate);
            }
            if !current.trim().is_empty() {
                result.push(current.trim().trim_matches(',').into());
            }
        }
    }
    result
}
pub(crate) fn read_tools(clause: &str) -> Vec<&'static str> {
    // Online topic words such as "calendar apps" do not authorize local reads.
    if crate::search_intent::explicit_request(clause).is_some() {
        return vec!["look_up_online"];
    }
    let words = tokens(clause);
    let document = words.iter().flat_map(|word| word.split('-')).any(|word|["lease","leases","quote","quotes","warranty","warranties","memo","agenda","itinerary","itineraries","document","documents","plan","plans","travel","policy","policies"].contains(&word));
    let has = |set: &[&str]| words.iter().any(|w| set.contains(w));
    let explicit = words.first().is_some_and(|word| {
        [
            "read",
            "check",
            "find",
            "search",
            "list",
            "review",
            "compare",
            "summarize",
            "summarise",
        ]
        .contains(word)
    });
    let question = words
        .first()
        .is_some_and(|word| ["what", "when", "which", "where"].contains(word))
        && has(&["my", "this", "these", "current", "today", "tomorrow"]);
    if !explicit && !question {
        return vec![];
    }
    // Pasted text is already present; a transformation does not authorize an
    // unrelated file/app lookup merely because its text mentions a policy.
    if !has(&["my"]) && words.first().is_some_and(|word|["summarize","summarise"].contains(word))
        && clause.split_once(':').is_some_and(|(_,text)|text.trim().len()>=20)
    {
        return vec![];
    }
    if has(&["files", "file", "folder", "folders"]) {
        vec!["search_files"]
    } else if has(&["calendar", "event", "events", "appointments", "meetings"]) {
        vec!["read_calendar"]
    } else if has(&["mail", "email", "emails", "inbox"]) {
        vec!["read_mail"]
    } else if has(&["reminders"]) {
        vec!["read_reminders"]
    } else if has(&["note", "notes"]) {
        if has(&["notes"]) || clause.contains("in notes") || clause.contains("notes app") || clause.contains("apple notes") {
            vec!["find_notes"]
        } else {
            vec!["find_notes", "search_files"]
        }
    } else if document && (explicit || has(&["my"])) {
        vec!["search_files"]
    } else if has(&["page", "website", "webpage"]) {
        vec!["read_page", "browse_page", "look_up_online"]
    } else if has(&["tabs"]) {
        vec!["browser_tabs"]
    } else if has(&["weather", "forecast"]) {
        vec!["weather", "look_up_online"]
    } else if has(&["flight", "flights"]) {
        vec!["flight_search", "look_up_online"]
    } else {
        vec![]
    }
}
fn resources(tools: &[String]) -> Vec<String> {
    let mut result = Vec::new();
    for tool in tools {
        if let Some(contract) = crate::tools::contract(tool) {
            for resource in &contract.resources {
                if !result.contains(resource) {
                    result.push(resource.clone());
                }
            }
        }
    }
    result
}
fn matches_arguments(required: &Value, actual: &Value) -> bool {
    required.as_object().is_none_or(|fields| {
        fields
            .iter()
            .all(|(key, value)| actual.get(key) == Some(value))
    })
}
fn verified_write(step: &Step) -> bool {
    if step.state != "succeeded" {
        return false;
    }
    let Some(receipt) = step
        .evidence
        .as_ref()
        .and_then(|e| e.get("receipt"))
        .cloned()
        .and_then(|v| serde_json::from_value::<ActionReceipt>(v).ok())
    else {
        return false;
    };
    receipt.outcome == ActionOutcome::Succeeded
        && receipt.tool == step.tool
        && !receipt.message.trim().is_empty()
        && (![
            "add_reminder",
            "add_calendar_event",
            "create_note",
            "draft_email",
            "create_saved_search",
            "create_price_watch",
            "remember",
            "set_timer",
            "control_music",
            "switch_tab",
            "browse_page",
        ]
        .contains(&step.tool.as_str())
            || receipt
                .object_id
                .as_deref()
                .is_some_and(|id| !id.is_empty()))
}

fn verified_read(step: &Step) -> bool {
    use crate::tools::SuccessVerifier;
    if step.effect || step.state != "succeeded" {
        return false;
    }
    let Some(evidence) = step
        .evidence
        .as_ref()
        .filter(|v| !v.is_null() && v.get("error").is_none())
    else {
        return false;
    };
    let Some(contract) = crate::tools::contract(&step.tool) else {
        return false;
    };
    match contract.success_verifier {
        SuccessVerifier::StructuredMathResult => crate::structured_math::verify(&step.tool, &step.arguments, evidence),
        SuccessVerifier::CalculationResult => crate::calculation::verify(&step.arguments, evidence),
        SuccessVerifier::ControlReceipt
            if step.tool == "control_music" && step.arguments["action"] == "now_playing" =>
        {
            crate::apps::validate_evidence(&step.tool, &step.arguments, evidence, false).is_ok()
        }
        SuccessVerifier::AppReadResult => crate::apps::validate_evidence(
            &step.tool,
            &step.arguments,
            evidence,
            step.tool == "read_calendar",
        )
        .is_ok(),
        SuccessVerifier::BrowserSnapshotReceipt => verified_write(step),
        SuccessVerifier::ScopedFileResults => {
            evidence.get("files").is_some_and(Value::is_array)
                || evidence.as_str().is_some_and(|s| !s.trim().is_empty())
        }
        SuccessVerifier::SystemSnapshot => {
            evidence.as_object().is_some_and(|o| !o.is_empty())
                || evidence.as_str().is_some_and(|s| !s.trim().is_empty())
        }
        SuccessVerifier::OnlineTaskResult => {
            ["sources", "cards", "pages"]
                .iter()
                .all(|key| evidence.get(key).is_some_and(Value::is_array))
                && evidence.get("summary").is_some_and(Value::is_string)
                && evidence.get("researchReport").filter(|r| !r.is_null()).map_or_else(
                    || {
                        // Old journals lack the research report. Preserve real
                        // integration cards and read pages, but discovery-only
                        // snippets cannot authorize a new dependent write.
                        let query = step.arguments["query"].as_str().unwrap_or_default();
                        let intent = crate::search_intent::SearchIntent::from_lookup(query, query);
                        evidence["summary"].as_str().is_some_and(|summary| !summary.trim().is_empty())
                            && (evidence["cards"].as_array().is_some_and(|cards| !cards.is_empty())
                                || evidence["pages"].as_array().is_some_and(|pages| pages.iter().any(|page| {
                                    serde_json::from_value::<crate::research::ResearchContent>(page.clone()).ok().is_some_and(|page| {
                                        crate::research::validate_public_url(&page.url).is_ok()
                                            && crate::search_pipeline::usable_page(&intent, &page)
                                    })
                                })))
                    },
                    |report| report["evidenceReady"] == true && report["answerVerified"] == true,
                )
        }
        SuccessVerifier::IntegrationResponse => evidence.as_object().is_some_and(|o| !o.is_empty()),
        _ => false,
    }
}

impl TaskContract {
    pub fn from_request(goal: &str) -> Self {
        Self::from_request_at(goal, chrono::Local::now().date_naive())
    }
    pub(crate) fn from_request_at(goal: &str, day: chrono::NaiveDate) -> Self {
        let mut contract = Self {
            version: 1,
            original_request: goal.into(),
            current_request: goal.into(),
            revision: 1,
            interpretation_date: day.format("%Y-%m-%d").to_string(),
            intent_status: "conservative_bootstrap".into(),
            ..Self::default()
        };
        contract.revisions.push(RequestRevision {
            revision: 1,
            request: goal.into(),
            field_updates: Value::Null,
        });
        contract.bootstrap(goal);
        contract
    }
    fn push_outcome(
        &mut self,
        kind: &str,
        description: &str,
        accepted: Vec<String>,
        arguments: Value,
        required: Vec<String>,
    ) {
        let dependencies = self
            .outcomes
            .last()
            .map(|o| vec![o.id.clone()])
            .unwrap_or_default();
        let id = format!("outcome-{}", self.outcomes.len() + 1);
        self.outcomes.push(Outcome {
            id,
            kind: kind.into(),
            description: description.into(),
            tool: accepted.first().cloned(),
            resources: resources(&accepted),
            accepted_tools: accepted,
            dependencies,
            status: "pending".into(),
            required_arguments: arguments,
            required_fields: required,
            completed_by: vec![],
            source_revision: self.revision,
        });
    }
    fn bootstrap(&mut self, goal: &str) {
        self.outcomes.clear();
        let search_segments = crate::search_intent::explicit_clause_spans(goal);
        // Preserve quoted phrases and search operators before the general
        // bootstrap normalizes prose. Only an integrated online result can
        // satisfy this read; opening a search-results tab cannot.
        if completion::requested_actions(goal) == (vec![], false)
            && search_segments.len() == 1
            && search_segments[0].0 == 0
            && search_segments[0].1 == goal.len()
        {
            if let Some(intent) = crate::search_intent::explicit_request(goal) {
                self.push_outcome(
                    "read",
                    goal,
                    vec!["look_up_online".into()],
                    json!({"query":intent.query}),
                    vec!["query".into()],
                );
                self.push_outcome("answer", goal, vec![], json!({}), vec![]);
                self.outcomes.last_mut().unwrap().resources = vec!["web".into()];
                self.intent_status = "explicit_online_lookup".into();
                return;
            }
        }
        let reference_date =
            chrono::NaiveDate::parse_from_str(&self.interpretation_date, "%Y-%m-%d")
                .unwrap_or_else(|_| chrono::Local::now().date_naive());
        // Preserve each raw online clause before punctuation/quote removal in
        // the older general bootstrap. Periods in Node.js or URLs are query
        // content and must not split or weaken the approved search.
        let fragment_clauses = |mut text: &str| {
            // Removing a preceding raw search leaves its joining words at the
            // front of the next fragment (" and read my mail"). They are task
            // separators, not part of that read's command prefix.
            loop {
                text = text
                    .trim_start_matches(|ch: char| ch.is_whitespace() || ch == ',' || ch == ';');
                let lower = text.to_ascii_lowercase();
                if let Some(prefix) = ["and then ", "then ", "and ", "also "]
                    .iter()
                    .find(|prefix| lower.starts_with(**prefix))
                {
                    text = &text[prefix.len()..];
                } else {
                    break;
                }
            }
            clauses(text)
        };
        let mut task_clauses = Vec::new();
        let mut end = 0;
        for (start, next, intent) in search_segments {
            task_clauses.extend(
                fragment_clauses(&goal[end..start])
                    .into_iter()
                    .map(|clause| (clause, None)),
            );
            task_clauses.push((intent.goal.clone(), Some(intent)));
            end = next;
        }
        task_clauses.extend(
            fragment_clauses(&goal[end..])
                .into_iter()
                .map(|clause| (clause, None)),
        );
        let mut unparsed = false;
        for (clause, search) in task_clauses {
            if clause.trim().is_empty() {
                continue;
            }
            let (writes, unsupported) = completion::requested_actions(&clause);
            // Query proposals in mixed tasks must retain the exact relevant
            // user clause, including quoted phrases and explicit site scope.
            let search = search.as_ref();
            let reads = if search.is_some() {
                vec!["look_up_online"]
            } else {
                read_tools(&clause)
            };
            if unsupported {
                self.push_outcome("unsupported", &clause, vec![], json!({}), vec![]);
            }
            // A small set of explicit source dependencies also appears inside
            // action clauses ("remind me before my first calendar event").
            // Literal note bodies are content, never dependency instructions.
            let words = tokens(&clause);
            let source_bound = !clause.contains(" with body ")
                && !writes.is_empty()
                && (words.iter().any(|w| ["before", "after"].contains(w))
                    || clause.contains(" from my ")
                    || clause.contains(" based on "));
            if source_bound && reads.is_empty() {
                let source = if words.iter().any(|w| {
                    [
                        "calendar",
                        "event",
                        "events",
                        "meeting",
                        "meetings",
                        "appointments",
                    ]
                    .contains(w)
                }) {
                    Some("read_calendar")
                } else if words
                    .iter()
                    .any(|w| ["mail", "email", "emails", "inbox"].contains(w))
                {
                    Some("read_mail")
                } else if words.iter().any(|w| ["notes"].contains(w)) {
                    Some("find_notes")
                } else {
                    None
                };
                if let Some(tool) = source {
                    let days = calendar_days(&clause, reference_date);
                    let args = if tool == "read_calendar" && days.len() == 1 {
                        json!({"day":days[0]})
                    } else {
                        json!({})
                    };
                    if !self.outcomes.iter().any(|o| {
                        o.kind == "read"
                            && o.accepted_tools.iter().any(|t| t == tool)
                            && (args.as_object().is_some_and(|o| o.is_empty())
                                || o.required_arguments == args)
                    }) {
                        self.push_outcome(
                            "read",
                            &format!("Read the requested source for: {clause}"),
                            vec![tool.into()],
                            args,
                            vec![],
                        );
                    }
                }
            }
            for tool in writes {
                let mut required: Vec<String> = crate::tools::definition(tool)
                    .and_then(|d| d["parameters"]["required"].as_array().cloned())
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect();
                if tool == "add_reminder"
                    && (tokens(&clause).iter().any(|w| {
                        [
                            "due", "before", "after", "tomorrow", "today", "tonight", "at",
                        ]
                        .contains(w)
                    }) || !dates(&clause).is_empty())
                    && !required.contains(&"due".into())
                {
                    required.push("due".into());
                }
                if tool == "add_calendar_event"
                    && completion::unquoted(goal).split(|c:char|!c.is_alphanumeric()).any(|word|["duration","lasting","minutes","minute"].contains(&word))
                    && !required.contains(&"minutes".into())
                {
                    required.push("minutes".into());
                }
                self.push_outcome(
                    "write",
                    &clause,
                    if tool == "open_link" {
                        vec![tool.into(), "browse_page".into()]
                    } else {
                        vec![tool.into()]
                    },
                    json!({}),
                    required,
                );
            }
            if !reads.is_empty() {
                let arguments = if reads[0] == "read_calendar" {
                    let days = calendar_days(&clause, reference_date);
                    if days.is_empty() {
                        vec![json!({})]
                    } else {
                        days.iter().map(|day| json!({"day":day})).collect()
                    }
                } else if let Some(intent) = search {
                    vec![json!({"query":intent.query})]
                } else {
                    vec![json!({})]
                };
                for args in arguments {
                    self.push_outcome(
                        "read",
                        search.map(|intent| intent.goal.as_str()).unwrap_or(&clause),
                        reads.iter().map(|s| s.to_string()).collect(),
                        args,
                        vec![],
                    );
                }
            } else if completion::requested_actions(&clause).0.is_empty() && !unsupported {
                unparsed = true;
            }
        }
        // An answer is distinct from successful writes. The conservative bootstrap
        // leaves unfamiliar continuations visible for the answer phase.
        if !self.outcomes.iter().any(|outcome|outcome.kind=="read") {
            let plain = completion::unquoted(goal);
            let pronoun_read = plain.split(['.',';','\n']).any(|clause| {
                let clause=completion::request_clause(clause.trim());
                ["find it","read it","check it"].iter().any(|prefix|clause==*prefix || clause.starts_with(&format!("{prefix} ")))
            });
            if pronoun_read {
                let declaration = goal.split(['.',';','\n']).find_map(|sentence| {
                    let sentence=sentence.trim();
                    let lower=sentence.to_ascii_lowercase();
                    ["i just added my ","i added my ","i just saved my ","i saved my "]
                        .iter().find_map(|prefix|lower.starts_with(prefix).then(||sentence[prefix.len()..].trim()))
                });
                if let Some(title)=declaration {
                    let lower=title.to_ascii_lowercase();
                    if [" note"," document"," file"].iter().any(|suffix|lower.ends_with(suffix)) && title.len()<=200 {
                        let query=title.rsplit_once(' ').map(|(head,_)|head).unwrap_or(title).trim();
                        if !query.is_empty() {
                            self.push_outcome("read","Read the document the user just identified",vec!["find_notes".into(),"search_files".into()],json!({"query":query}),vec!["query".into()]);
                        }
                    }
                }
            }
        }
        let (writes, unsupported) = completion::requested_actions(goal);
        let plain = completion::unquoted(goal);
        let words = tokens(&plain);
        let arithmetic = words.iter().any(|word| ["calculate", "compute", "split", "scale", "arithmetic"].contains(word))
            || plain.contains("how much less") || plain.contains("how much i save") || plain.contains("how much do i save");
        if writes.is_empty() && !unsupported && arithmetic {
            let tool = if words.contains(&"split") { "split_costs" }
                else if words.contains(&"scale") { "scale_quantities" }
                else if words.contains(&"subscriptions") && words.contains(&"unused") { "sum_by_status" }
                else if words.contains(&"offers") && plain.contains("at least") { "compare_options" }
                else { "calculate" };
            self.push_outcome("read", "Compute the requested result from supplied quantities, labels and constraints using the fixed operation", vec![tool.into()], json!({}), vec![]);
        }
        if !completion::receipt_only_goal(goal) {
            let mut accepted = Vec::new();
            for outcome in &self.outcomes {
                for tool in &outcome.accepted_tools {
                    if outcome.kind == "read" && !accepted.contains(tool) {
                        accepted.push(tool.clone());
                    }
                }
            }
            self.push_outcome("answer", goal, vec![], json!({}), vec![]);
            let answer = self.outcomes.last_mut().unwrap();
            answer.resources = resources(&accepted);
        }
        // Existing deterministic parsers supply only exact user literals.
        // Failed/ambiguous extraction leaves fields unresolved for clarification.
        let mut literals = Vec::new();
        if let Some(query) = crate::explicit_fields::read_query(goal).filter(|query|query.len()<=100) {
            literals.push(("read","find_notes","query",Value::String(query)));
        }
        for tool in ["create_note","add_reminder","add_calendar_event"] {
            if let Some(title) = crate::explicit_fields::action_title(goal,tool) {
                literals.push(("write",tool,"title",Value::String(title)));
            }
        }
        if let Some(location)=crate::location_fields::supplied_location(goal) {
            literals.push(("write","add_calendar_event","location",Value::String(location)));
            for outcome in self.outcomes.iter_mut().filter(|outcome|outcome.tool.as_deref()==Some("add_calendar_event")) {
                if !outcome.required_fields.contains(&"location".into()) { outcome.required_fields.push("location".into()); }
            }
        }
        for (kind,tool,field,value) in literals {
            let matching = self.outcomes.iter().enumerate().filter(|(_,outcome)| outcome.kind==kind && outcome.accepted_tools.iter().any(|accepted| accepted==tool || (kind=="read" && accepted=="search_files"))).map(|(index,_)|index).collect::<Vec<_>>();
            if matching.len()==1 {
                let outcome = &mut self.outcomes[matching[0]];
                outcome.required_arguments[field] = value.clone();
                self.constraints.push(Constraint {outcome_id:Some(outcome.id.clone()),field:field.into(),value,source:"user_literal".into(),revision:self.revision});
            }
        }
        for bound in [
            crate::explicit_fields::explicit_note(goal, &[]),
            crate::explicit_fields::explicit_reminder(goal, &[]),
        ] {
            if let Ok(Some(bound)) = bound {
                let matches: Vec<usize> = self
                    .outcomes
                    .iter()
                    .enumerate()
                    .filter(|(_, o)| o.kind == "write" && o.tool.as_deref() == Some(bound.tool))
                    .map(|(i, _)| i)
                    .collect();
                if matches.len() == 1 {
                    let outcome = &mut self.outcomes[matches[0]];
                    outcome.required_arguments = bound.arguments.clone();
                    for (field, value) in bound.arguments.as_object().into_iter().flatten() {
                        if !self.constraints.iter().any(|c| {
                            c.outcome_id.as_deref() == Some(&outcome.id) && c.field == *field
                        }) {
                            self.constraints.push(Constraint {
                                outcome_id: Some(outcome.id.clone()),
                                field: field.clone(),
                                value: value.clone(),
                                source: "user_literal".into(),
                                revision: self.revision,
                            });
                        }
                    }
                }
            }
        }
        if unparsed {
            self.intent_status = "partially_understood".into();
        }
    }
    /// Refresh only from the trusted saved user goal and controller journal.
    pub fn refresh(&mut self, goal: &str, steps: &[Step]) {
        if self.version == 0 {
            *self = Self::from_request(goal);
        } else if self.current_request != goal {
            let previous = self.outcomes.clone();
            self.revision += 1;
            self.interpretation_date = chrono::Local::now()
                .date_naive()
                .format("%Y-%m-%d")
                .to_string();
            self.current_request = goal.into();
            self.revisions.push(RequestRevision {
                revision: self.revision,
                request: goal.into(),
                field_updates: Value::Null,
            });
            self.bootstrap(goal);
            // Original request scopes retain their initial date even when a
            // later clarification is supplied after midnight or after restart.
            for outcome in &mut self.outcomes {
                if let Some(old) = previous.iter().find(|old| {
                    old.id == outcome.id
                        && old.kind == outcome.kind
                        && old.description == outcome.description
                        && old.accepted_tools == outcome.accepted_tools
                }) {
                    outcome.required_arguments = old.required_arguments.clone();
                    outcome.source_revision = old.source_revision;
                }
            }
        }
        for constraint in &self.constraints {
            if let Some(outcome) = self
                .outcomes
                .iter_mut()
                .find(|o| Some(o.id.as_str()) == constraint.outcome_id.as_deref())
            {
                outcome.required_arguments[&constraint.field] = constraint.value.clone();
            }
        }
        let mut used = HashSet::new();
        let mut completed = HashSet::new();
        let mut last_step_index: Option<usize> = None;
        for outcome in &mut self.outcomes {
            let ready = outcome.dependencies.iter().all(|id| completed.contains(id));
            outcome.completed_by.clear();
            if outcome.kind == "answer" {
                outcome.status = if ready && outcome.status == "complete" {
                    "complete"
                } else if ready {
                    "ready"
                } else {
                    "pending"
                }
                .into();
                continue;
            }
            let candidate = steps.iter().enumerate().find(|(index, step)| {
                ready
                    && last_step_index.is_none_or(|previous| *index > previous)
                    && !used.contains(&step.id)
                    && outcome.accepted_tools.contains(&step.tool)
                    && matches_arguments(&outcome.required_arguments, &step.arguments)
                    && if outcome.kind == "write" {
                        (step.effect || step.tool == "browse_page") && verified_write(step)
                    } else {
                        verified_read(step)
                    }
            });
            if let Some((index, step)) = candidate {
                last_step_index = Some(index);
                used.insert(step.id.clone());
                outcome.completed_by.push(step.id.clone());
                outcome.status = "complete".into();
                completed.insert(outcome.id.clone());
            } else {
                let pending = steps.iter().rev().find(|step| {
                    outcome.accepted_tools.contains(&step.tool)
                        && matches_arguments(&outcome.required_arguments, &step.arguments)
                        && !["succeeded", "superseded"].contains(&step.state.as_str())
                });
                outcome.status = match pending.map(|step| step.state.as_str()) {
                    Some("waiting") => "waiting",
                    Some("running") => "running",
                    Some("failed" | "unknown" | "cancelled") => "blocked",
                    _ if outcome.kind == "unsupported" => "blocked",
                    _ if ready => "ready",
                    _ => "pending",
                }
                .into();
            }
        }
        for step in steps {
            if !self.provenance.contains_key(&step.id) {
                self.record_proposal(&step.id, &step.arguments, None);
            }
            if let Some(fields) = self.provenance.get_mut(&step.id) {
                for value in fields.values_mut() {
                    if step.approved_at.is_some() {
                        value.approved = true;
                    }
                    value.verified = step.effect && verified_write(step);
                }
            }
        }
        self.unresolved_fields.clear();
        for outcome in &self.outcomes {
            if outcome.kind != "write" || outcome.status == "complete" {
                continue;
            }
            let proposed = steps.iter().rev().find(|s| {
                outcome.accepted_tools.contains(&s.tool)
                    && matches_arguments(&outcome.required_arguments, &s.arguments)
                    && ["waiting", "running"].contains(&s.state.as_str())
            });
            for field in &outcome.required_fields {
                let value = proposed
                    .and_then(|s| s.arguments.get(field))
                    .or_else(|| outcome.required_arguments.get(field));
                let present = value.is_some_and(|v| {
                    !v.is_null() && !v.as_str().is_some_and(|s| s.trim().is_empty())
                });
                let provenance = proposed
                    .and_then(|s| self.provenance.get(&s.id))
                    .and_then(|fields| fields.get(field));
                if !present
                    || provenance.is_some_and(|p| p.origin == "model_proposal" && !p.verified)
                {
                    self.unresolved_fields.push(UnresolvedField { outcome_id:outcome.id.clone(), field:field.clone(), status:if present { "unverified" } else { "missing" }.into(), reason:if present { "Proposed by the model; approval records consent, not independent evidence." } else { "A requested value has not been supplied or resolved." }.into() });
                }
            }
        }
    }
    /// Binds a field of the one requested write that uses `tool` to a value the controller
    /// resolved itself (`source` says how), unless the user's own words already fixed it.
    /// The binding is a constraint, so later proposals and approvals must keep it.
    pub fn bind_write_field(&mut self, tool: &str, field: &str, value: Value, source: &str) -> bool {
        let matching: Vec<usize> = self
            .outcomes
            .iter()
            .enumerate()
            .filter(|(_, o)| o.kind == "write" && o.tool.as_deref() == Some(tool))
            .map(|(index, _)| index)
            .collect();
        let [index] = matching[..] else {
            return false;
        };
        let outcome = &mut self.outcomes[index];
        if self
            .constraints
            .iter()
            .any(|c| c.outcome_id.as_deref() == Some(&outcome.id) && c.field == field)
        {
            return false;
        }
        outcome.required_arguments[field] = value.clone();
        self.constraints.push(Constraint {
            outcome_id: Some(outcome.id.clone()),
            field: field.into(),
            value,
            source: source.into(),
            revision: self.revision,
        });
        true
    }
    pub fn next_outcome(&self) -> Option<&Outcome> {
        self.outcomes.iter().find(|o| o.status != "complete")
    }
    pub fn apply_user_fields(&mut self, outcome_id: &str, fields: &Value) -> Result<(), String> {
        let outcome = self
            .outcomes
            .iter_mut()
            .find(|o| o.id == outcome_id && o.kind == "write" && o.status != "complete")
            .ok_or(
                "That task outcome has already finished, is not an action, or no longer exists.",
            )?;
        let fields = fields
            .as_object()
            .ok_or("Clarified fields must be an object.")?;
        let definition = outcome
            .tool
            .as_deref()
            .and_then(crate::tools::definition)
            .ok_or("The requested action has no known schema.")?;
        for (field, value) in fields {
            let schema = definition["parameters"]["properties"]
                .get(field)
                .ok_or("That field is not supported by this action.")?;
            let valid_type = match schema["type"].as_str() {
                Some("string") => value.as_str().is_some_and(|s| !s.trim().is_empty()),
                Some("number" | "integer") => value.is_number(),
                Some("boolean") => value.is_boolean(),
                _ => false,
            };
            if !valid_type
                || (schema["type"] == "integer" && !value.is_i64() && !value.is_u64())
                || schema["enum"]
                    .as_array()
                    .is_some_and(|values| !values.contains(value))
            {
                return Err(format!(
                    "Supply a valid {} value for {field}.",
                    schema["type"].as_str().unwrap_or("supported")
                ));
            }
        }
        if !fields.is_empty() {
            self.revision += 1;
            self.revisions.push(RequestRevision {
                revision: self.revision,
                request: self.current_request.clone(),
                field_updates: json!({"outcomeId":outcome_id,"fields":fields}),
            });
        }
        for (field, value) in fields {
            outcome.required_arguments[field] = value.clone();
            self.constraints
                .retain(|c| c.outcome_id.as_deref() != Some(outcome_id) || c.field != *field);
            self.constraints.push(Constraint {
                outcome_id: Some(outcome_id.into()),
                field: field.clone(),
                value: value.clone(),
                source: "user_clarification".into(),
                revision: self.revision,
            });
        }
        Ok(())
    }
    pub fn record_proposal(&mut self, step_id: &str, arguments: &Value, source: Option<&Value>) {
        let outcome_id = self
            .outcomes
            .iter()
            .find(|o| o.completed_by.iter().any(|id| id == step_id))
            .or_else(|| self.next_outcome())
            .map(|o| o.id.clone());
        let constraints: Vec<_> = self
            .constraints
            .iter()
            .filter(|c| {
                c.outcome_id == outcome_id
                    && ["user_literal", "user_clarification", "user_approved_edit"]
                        .contains(&c.source.as_str())
            })
            .cloned()
            .collect();
        let fields = self.provenance.entry(step_id.into()).or_default();
        for (field, value) in arguments.as_object().into_iter().flatten() {
            if fields.get(field).is_some_and(|p| p.approved) {
                continue;
            }
            let mut provenance = FieldProvenance {
                value: value.clone(),
                origin: "model_proposal".into(),
                ..FieldProvenance::default()
            };
            if let Some(constraint) = constraints
                .iter()
                .find(|c| c.field == *field && c.value == *value)
            {
                provenance.origin = "user_literal".into();
                provenance.field_ref = Some(field.clone());
                provenance.source = Some(
                    json!({"kind":"task_constraint","source":"user_request","outcomeId":constraint.outcome_id,"revision":constraint.revision,"field":field}),
                );
            }
            if let Some(source) = source {
                provenance.source = Some(source.clone());
                if source.get("source").and_then(Value::as_str) == Some("user_request")
                    && source.get("fields").and_then(|v| v.get(field)) == Some(value)
                {
                    provenance.origin = "user_literal".into();
                    provenance.field_ref = Some(field.clone());
                } else if source.get("kind").and_then(Value::as_str) == Some("user_fields")
                    && source.get("source").and_then(Value::as_str) == Some("user_request")
                    && source.get("fields").is_some_and(Value::is_object)
                    && source["fields"].get(field).is_none()
                {
                    provenance.origin = "runtime_default".into();
                    provenance.field_ref = None;
                } else if source.get("kind").and_then(Value::as_str) == Some("calendar_reminder") {
                    provenance.origin = "deterministic_calculation".into();
                    provenance.source_step_id = source
                        .get("sourceStepId")
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                    provenance.record_id = source
                        .pointer("/event/id")
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                    provenance.field_ref = source
                        .get("eventPath")
                        .and_then(Value::as_str)
                        .map(|p| format!("{p}/{}", if field == "due" { "start" } else { "title" }));
                    provenance.calculation = Some(if field == "due" {
                        json!({"operation":"subtract_minutes","minutes":source["minutesBefore"],"input":source["event"]["start"]})
                    } else {
                        json!({"operation":"prefix","prefix":"Prepare for ","input":source["event"]["title"]})
                    });
                }
            }
            fields.insert(field.clone(), provenance);
        }
    }
    pub fn record_approval(&mut self, step_id: &str, before: &Value, after: &Value) {
        let outcome_id = self
            .next_outcome()
            .filter(|o| o.kind == "write" && matches_arguments(&o.required_arguments, before))
            .map(|o| o.id.clone());
        if let Some(outcome_id) = outcome_id {
            for (field, value) in after
                .as_object()
                .into_iter()
                .flatten()
                .filter(|(field, value)| before.get(field.as_str()) != Some(*value))
            {
                self.constraints.retain(|c| {
                    c.outcome_id.as_deref() != Some(outcome_id.as_str()) || c.field != *field
                });
                self.constraints.push(Constraint {
                    outcome_id: Some(outcome_id.clone()),
                    field: field.clone(),
                    value: value.clone(),
                    source: "user_approved_edit".into(),
                    revision: self.revision,
                });
                if let Some(outcome) = self.outcomes.iter_mut().find(|o| o.id == outcome_id) {
                    outcome.required_arguments[field] = value.clone();
                }
            }
        }
        let fields = self.provenance.entry(step_id.into()).or_default();
        for (field, value) in after.as_object().into_iter().flatten() {
            let previous = fields.get(field).cloned();
            let entry = if before.get(field) != Some(value) {
                FieldProvenance {
                    value: value.clone(),
                    origin: "user_approved_edit".into(),
                    approved: true,
                    previous: previous.map(Box::new),
                    ..FieldProvenance::default()
                }
            } else {
                let mut p = previous.unwrap_or_else(|| FieldProvenance {
                    value: value.clone(),
                    origin: "model_proposal".into(),
                    ..FieldProvenance::default()
                });
                p.approved = true;
                p
            };
            fields.insert(field.clone(), entry);
        }
        fields.retain(|field, _| after.get(field).is_some());
    }
    pub fn relevant_tools(&self, steps: &[Step], definitions: &Value) -> Value {
        let unresolved = steps
            .iter()
            .any(|s| ["failed", "unknown", "running", "waiting"].contains(&s.state.as_str()));
        let next = self.next_outcome();
        let next_write = next.filter(|o| o.kind == "write" && o.status == "ready");
        let selected = definitions
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|definition| {
                let name = definition["name"].as_str()?;
                let effect = crate::actions::TOOLS.contains(&name)
                    || crate::monitor_actions::TOOLS.contains(&name)
                    || crate::control_actions::TOOLS.contains(&name);
                let write_allowed = !unresolved
                    && next_write.is_some_and(|o| o.accepted_tools.iter().any(|t| t == name));
                if effect && !write_allowed && name != "control_music" {
                    return None;
                }
                let relevant_read = next.is_some_and(|o| {
                    o.accepted_tools.iter().any(|t| t == name)
                        || (o.kind == "answer" && (name == "calculate" || crate::structured_math::TOOLS.contains(&name)))
                        || (o.kind == "answer"
                            && (o.resources.is_empty()
                                || crate::tools::contract(name).is_some_and(|c| {
                                    c.resources.iter().any(|r| o.resources.contains(r))
                                })))
                });
                if !effect && !relevant_read {
                    return None;
                }
                if name == "control_music" && !write_allowed && !relevant_read {
                    return None;
                }
                let mut result = definition.clone();
                if name == "control_music" && !write_allowed {
                    result["parameters"]["properties"]["action"]["enum"] = json!(["now_playing"]);
                }
                if write_allowed {
                    if let Some(outcome) = next_write {
                        result["parameters"]["required"] = json!(outcome.required_fields);
                        for (field,value) in outcome.required_arguments.as_object().into_iter().flatten() {
                            if value.is_string() && result["parameters"]["properties"][field]["type"]=="string" {
                                result["parameters"]["properties"][field]["enum"] = json!([value]);
                            }
                        }
                    }
                }
                if let Some(outcome) =
                    next.filter(|o| o.kind == "read" && o.accepted_tools.iter().any(|t| t == name))
                {
                    let mut required = result["parameters"]["required"]
                        .as_array()
                        .cloned()
                        .unwrap_or_default();
                    for (field, value) in
                        outcome.required_arguments.as_object().into_iter().flatten()
                    {
                        if result["parameters"]["properties"].get(field).is_some() {
                            if !required.iter().any(|item| item.as_str() == Some(field)) {
                                required.push(json!(field));
                            }
                            result["parameters"]["properties"][field]["enum"] = json!([value]);
                        }
                    }
                    result["parameters"]["required"] = json!(required);
                }
                Some(result)
            })
            .collect::<Vec<_>>();
        Value::Array(selected)
    }
    pub fn validate_proposal(
        &self,
        tool: &str,
        arguments: &Value,
        steps: &[Step],
    ) -> Result<(), String> {
        let effect = crate::actions::TOOLS.contains(&tool)
            || crate::monitor_actions::TOOLS.contains(&tool)
            || (crate::control_actions::TOOLS.contains(&tool)
                && !(tool == "control_music" && arguments["action"] == "now_playing"));
        if steps.iter().any(|s| {
            ["unknown", "running", "waiting"].contains(&s.state.as_str())
                || (effect && s.state == "failed")
        }) {
            return Err("An earlier step is unresolved. Its outcome must be resolved before another action can run.".into());
        }
        let outcome = self
            .next_outcome()
            .ok_or("All requested task outcomes have already finished.")?;
        if !effect {
            if (outcome.kind == "read" || (outcome.kind == "write" && tool == "browse_page"))
                && outcome.accepted_tools.iter().any(|t| t == tool)
            {
                if !matches_arguments(&outcome.required_arguments, arguments) {
                    return Err("The proposed source read conflicts with the date or query in your request.".into());
                }
                return Ok(());
            }
            if outcome.kind == "answer"
                && (tool == "calculate" || crate::structured_math::TOOLS.contains(&tool) || outcome.resources.is_empty()
                    || crate::tools::contract(tool)
                        .is_some_and(|c| c.resources.iter().any(|r| outcome.resources.contains(r))))
            {
                return Ok(());
            }
            return Err(
                "This source read is not relevant to the next unfinished task outcome.".into(),
            );
        }
        if outcome.kind != "write"
            || outcome.status != "ready"
            || !outcome.accepted_tools.iter().any(|t| t == tool)
        {
            return Err(
                "This action is not the next unfinished outcome authorized by your request.".into(),
            );
        }
        if !matches_arguments(&outcome.required_arguments, arguments) {
            return Err("The proposed action conflicts with your saved task constraints.".into());
        }
        for field in &outcome.required_fields {
            if arguments
                .get(field)
                .is_none_or(|v| v.is_null() || v.as_str().is_some_and(|s| s.trim().is_empty()))
            {
                return Err(format!(
                    "The requested {field} is unresolved. Supply it before approving this action."
                ));
            }
        }
        Ok(())
    }
    pub fn validate_answer(&self, text: &str, steps: &[Step]) -> Result<String, String> {
        if steps
            .iter()
            .any(|step| step.effect && step.state == "unknown")
        {
            return Err("An app action has no confirmed success receipt and its outcome is unknown. Review the destination app; Buntini will not repeat this write.".into());
        }
        if let Some(outcome) = self
            .outcomes
            .iter()
            .find(|o| o.kind != "answer" && o.status != "complete")
        {
            return Err(format!("This task is unfinished: {}. The requested source or action has not been verified.",outcome.description));
        }
        if self.outcomes.iter().any(|o| o.kind == "answer") && text.trim().is_empty() {
            return Err("The requested answer is still missing.".into());
        }
        completion::validate_answer(&self.current_request, text, steps)
    }
    pub fn completed_action_answer(&self, steps: &[Step]) -> Option<String> {
        if self
            .outcomes
            .iter()
            .any(|o| o.kind == "answer" || o.status != "complete")
        {
            return None;
        }
        completion::completed_action_answer(&self.current_request, steps)
    }
    pub fn terminal_receipt_answer(&self, steps: &[Step]) -> Option<String> {
        if !self.outcomes.iter().any(|outcome|outcome.kind=="write")
            || self.outcomes.iter().any(|outcome|outcome.kind!="answer" && outcome.status!="complete") {
            return None;
        }
        let plain=completion::unquoted(&self.current_request);
        let mut remaining=Vec::new();
        for sentence in plain.split(['.',';','\n']) {
            let sentence=sentence.trim();
            if sentence.is_empty() { continue; }
            let temporal_fact=["my ","the ","our "].iter().any(|prefix|sentence.starts_with(prefix))
                && ["must be returned by ","is due ","runs from ","starts at ","ends at "].iter().any(|phrase|sentence.contains(phrase))
                && !sentence.contains(" and ") && !sentence.contains(" then ")
                && sentence.split_whitespace().any(|word|chrono::NaiveTime::parse_from_str(word.trim_matches([',',';']),"%H:%M").is_ok());
            let duration_field=(sentence.starts_with("derive its duration ") || sentence.starts_with("derive the duration "))
                && !sentence.contains(" then ")
                && sentence.split(" and ").skip(1).all(completion::non_action);
            if temporal_fact || duration_field { continue; }
            // Only discard this known field modifier. Any further requested
            // operation remains for the conservative completion parser below.
            remaining.push(sentence.replace(", using exactly the title","").replace(" with exactly the title"," titled"));
        }
        let reduced=remaining.join(". ");
        if !completion::receipt_only_goal(&reduced) { return None; }
        let mut messages=Vec::new();
        for outcome in self.outcomes.iter().filter(|outcome|outcome.kind=="write") {
            for id in &outcome.completed_by {
                let step=steps.iter().find(|step|step.id==*id && verified_write(step))?;
                let receipt:ActionReceipt=serde_json::from_value(step.evidence.as_ref()?.get("receipt")?.clone()).ok()?;
                let field=|key:&str| receipt.evidence.get(key).and_then(Value::as_str).filter(|value|!value.is_empty());
                let title=field("title").unwrap_or("the requested item");
                let message=match step.tool.as_str() {
                    "add_reminder" if field("due").is_some()=>format!("Created and verified reminder \"{title}\", due {}.",field("due")?),
                    "add_calendar_event" if field("start").is_some() && field("end").is_some()=>format!("Created and verified calendar event \"{title}\", {}–{}{}.",field("start")?,field("end")?,field("location").map(|location|format!(", {location}")).unwrap_or_default()),
                    _=>receipt.message,
                };
                messages.push(message);
            }
        }
        (!messages.is_empty()).then(||messages.join("\n\n"))
    }
    pub fn mark_answer_complete(&mut self) {
        if self
            .outcomes
            .iter()
            .filter(|o| o.kind != "answer")
            .all(|o| o.status == "complete")
        {
            for outcome in &mut self.outcomes {
                if outcome.kind == "answer" {
                    outcome.status = "complete".into();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn step(id: &str, tool: &str, args: Value, effect: bool) -> Step {
        Step {
            id: id.into(),
            tool: tool.into(),
            arguments: args.clone(),
            state: "succeeded".into(),
            effect,
            proposed_at: "now".into(),
            approved_at: effect.then(|| "now".into()),
            finished_at: Some("now".into()),
            evidence: Some(if effect {
                json!({"receipt":ActionReceipt {outcome:ActionOutcome::Succeeded,tool:tool.into(),object_id:Some(id.into()),destination:Some("Fixture".into()),message:"Verified fixture.".into(),evidence:json!({}),error:None}})
            } else if tool == "read_calendar" {
                crate::apps::calendar_evidence(
                    args["day"].as_str().unwrap_or("2026-10-23"),
                    &crate::apps::Agenda {
                        items: vec![],
                        total: 0,
                    },
                )
            } else {
                json!({"complete":true})
            }),
        }
    }
    #[test]
    fn a_title_resolved_from_the_screen_constrains_the_reminder_but_not_its_time() {
        let goal = "Remind me about this tomorrow at 9";
        let mut contract = TaskContract::from_request(goal);
        contract.refresh(goal, &[]);
        let title = json!("Lease renewal (email from Ana Silva)");
        assert!(contract.bind_write_field("add_reminder", "title", title.clone(), "focus_referent"));
        // A second binding cannot replace the first, and other tools are untouched.
        assert!(!contract.bind_write_field("add_reminder", "title", json!("Other"), "focus_referent"));
        assert!(!contract.bind_write_field("create_note", "title", json!("Other"), "focus_referent"));
        contract.refresh(goal, &[]);
        let write = contract.next_outcome().unwrap();
        assert_eq!(write.tool.as_deref(), Some("add_reminder"));
        assert_eq!(write.required_arguments["title"], title);
        assert!(contract
            .constraints
            .iter()
            .any(|c| c.field == "title" && c.source == "focus_referent"));
        let offered = contract.relevant_tools(&[], &crate::tools::offer(&["add_reminder"]));
        assert_eq!(
            offered[0]["parameters"]["properties"]["title"]["enum"],
            json!([title])
        );
        assert!(contract
            .validate_proposal("add_reminder", &json!({"title":"Something else","due":"2026-10-30 09:00"}), &[])
            .is_err());
        assert!(contract
            .validate_proposal("add_reminder", &json!({"title":title,"due":"2026-10-30 09:00"}), &[])
            .is_ok());
    }
    fn names(definitions: &Value) -> Vec<&str> {
        definitions
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v["name"].as_str().unwrap())
            .collect()
    }
    #[test]
    fn explicit_online_research_keeps_exact_query_and_does_not_read_private_apps() {
        let goal = "Search the web for \"Calendar apps\" -sponsored and compare prices";
        let mut contract = TaskContract::from_request(goal);
        contract.refresh(goal, &[]);
        let read = contract.next_outcome().unwrap();
        assert_eq!(read.kind, "read");
        assert_eq!(read.accepted_tools, ["look_up_online"]);
        assert_eq!(
            read.required_arguments,
            json!({"query":"\"Calendar apps\" -sponsored and compare prices"})
        );
        assert_eq!(
            names(&contract.relevant_tools(
                &[],
                &crate::tools::offer(&[
                    "look_up_online",
                    "read_calendar",
                    "read_mail",
                    "open_link"
                ])
            )),
            ["look_up_online"]
        );
        assert!(contract
            .validate_proposal("look_up_online", &json!({"query":"calendar apps"}), &[])
            .is_err());
        assert!(contract.validate_answer("Done", &[]).is_err());

        let navigation = step(
            "opened",
            "open_link",
            json!({"url":"https://www.google.com/search?q=calendar"}),
            true,
        );
        contract.refresh(goal, &[navigation]);
        assert_ne!(
            contract.outcomes[0].status, "complete",
            "A search-results window is not research evidence"
        );
    }

    #[test]
    fn compound_online_lookup_preserves_the_requested_follow_up_action() {
        let goal = "Search the web for calendar apps, then create a note";
        let mut contract = TaskContract::from_request(goal);
        contract.refresh(goal, &[]);
        assert_eq!(
            contract
                .outcomes
                .iter()
                .filter(|o| o.kind != "answer")
                .map(|o| o.tool.as_deref().unwrap())
                .collect::<Vec<_>>(),
            ["look_up_online", "create_note"]
        );
        assert_eq!(
            contract.outcomes[1].dependencies,
            [contract.outcomes[0].id.clone()]
        );
        assert_eq!(
            contract.outcomes[0].required_arguments,
            json!({"query":"calendar apps"})
        );
        assert!(contract
            .validate_proposal(
                "create_note",
                &json!({"title":"Apps","body":"Unverified comparison"}),
                &[]
            )
            .is_err());
    }

    #[test]
    fn compound_named_site_query_is_pinned_without_exporting_later_private_fields() {
        let goal = "Search YouTube for \"Node.js\" -ads, then create a note titled \"Private\" with body \"secret details\"";
        let mut contract = TaskContract::from_request(goal);
        contract.refresh(goal, &[]);
        let read = contract.next_outcome().unwrap();
        assert_eq!(
            read.required_arguments,
            json!({"query":"\"Node.js\" -ads site:youtube.com"})
        );
        assert_eq!(read.accepted_tools, ["look_up_online"]);
        assert_eq!(
            contract
                .outcomes
                .iter()
                .filter(|o| o.kind == "write")
                .count(),
            1
        );
        assert!(contract
            .validate_proposal("look_up_online", &json!({"query":"Node.js"}), &[])
            .is_err());
        assert!(contract
            .validate_proposal("look_up_online", &read.required_arguments, &[])
            .is_ok());
        assert_eq!(contract.original_request, goal);
        assert_eq!(
            crate::search_intent::SearchIntent::from_lookup(
                goal,
                read.required_arguments["query"].as_str().unwrap()
            )
            .preferred_domains,
            ["youtube.com"]
        );
    }

    #[test]
    fn online_query_does_not_swallow_later_reads_or_other_site_lookups() {
        let goal = "Search YouTube for Node.js and read my mail, then search Wikipedia for C++";
        let mut contract = TaskContract::from_request(goal);
        contract.refresh(goal, &[]);
        let reads: Vec<_> = contract
            .outcomes
            .iter()
            .filter(|o| o.kind == "read")
            .collect();
        assert_eq!(
            reads
                .iter()
                .map(|o| o.tool.as_deref().unwrap())
                .collect::<Vec<_>>(),
            ["look_up_online", "read_mail", "look_up_online"]
        );
        assert_eq!(
            reads[0].required_arguments,
            json!({"query":"Node.js site:youtube.com"})
        );
        assert_eq!(
            reads[2].required_arguments,
            json!({"query":"C++ site:wikipedia.org"})
        );
        assert_eq!(
            crate::search_intent::SearchIntent::from_lookup(
                goal,
                reads[2].required_arguments["query"].as_str().unwrap()
            )
            .preferred_domains,
            ["wikipedia.org"]
        );
    }

    #[test]
    fn exact_read_obligations_do_not_collapse_same_tool_different_dates() {
        let goal = "Read my calendar for 2026-10-23 and 2026-10-24, then create a note";
        let mut contract = TaskContract::from_request(goal);
        let first = step("day1", "read_calendar", json!({"day":"2026-10-23"}), false);
        contract.refresh(goal, &[first.clone()]);
        assert_eq!(
            contract
                .outcomes
                .iter()
                .filter(|o| o.kind == "read")
                .count(),
            2
        );
        assert_eq!(
            contract
                .outcomes
                .iter()
                .filter(|o| o.kind == "write")
                .count(),
            1
        );
        assert_eq!(contract.outcomes[0].status, "complete");
        assert_eq!(
            contract.outcomes[1].required_arguments,
            json!({"day":"2026-10-24"})
        );
        assert!(contract
            .validate_proposal(
                "create_note",
                &json!({"title":"Both","body":"Both days"}),
                &[first.clone()]
            )
            .is_err());
        assert_eq!(
            names(&contract.relevant_tools(
                &[first.clone()],
                &crate::tools::offer(&["read_calendar", "find_notes", "read_mail", "create_note"])
            )),
            ["read_calendar"]
        );
        let second = step("day2", "read_calendar", json!({"day":"2026-10-24"}), false);
        contract.refresh(goal, &[first.clone(), second.clone()]);
        assert!(contract
            .validate_proposal(
                "create_note",
                &json!({"title":"Both","body":"Both days"}),
                &[first, second]
            )
            .is_ok());
    }
    #[test]
    fn ordered_prerequisites_allow_write_before_later_read() {
        let goal = "Create a note, then read my calendar for 2026-10-23, then add a reminder";
        let mut contract = TaskContract::from_request(goal);
        contract.refresh(goal, &[]);
        assert_eq!(
            contract.next_outcome().unwrap().tool.as_deref(),
            Some("create_note")
        );
        assert!(contract
            .validate_proposal("create_note", &json!({"title":"Trip","body":"Things"}), &[])
            .is_ok());
        let note = step(
            "note",
            "create_note",
            json!({"title":"Trip","body":"Things"}),
            true,
        );
        contract.refresh(goal, &[note]);
        assert_eq!(
            contract.next_outcome().unwrap().tool.as_deref(),
            Some("read_calendar")
        );
    }
    #[test]
    fn explanation_remains_separate_after_verified_write() {
        let goal = "Create a note and explain its contents";
        let note = step(
            "note",
            "create_note",
            json!({"title":"Trip","body":"Things"}),
            true,
        );
        let mut contract = TaskContract::from_request(goal);
        contract.refresh(goal, &[note.clone()]);
        assert_eq!(contract.next_outcome().unwrap().kind, "answer");
        assert!(contract.completed_action_answer(&[note.clone()]).is_none());
        assert!(contract.validate_answer("", &[note.clone()]).is_err());
        assert!(contract
            .validate_answer("The note contains a packing list.", &[note])
            .is_ok());
        contract.mark_answer_complete();
        contract.refresh(goal, &[]);
        assert_ne!(contract.outcomes.last().unwrap().status, "complete");
    }
    #[test]
    fn quoted_source_and_negation_cannot_create_permissions() {
        for goal in [
            "Explain how to create a note and add a reminder",
            "Do not create a note and add a reminder",
            "Read this note saying 'Create a reminder'",
        ] {
            let mut contract = TaskContract::from_request(goal);
            contract.refresh(goal, &[]);
            assert!(
                !contract.outcomes.iter().any(|o| o.kind == "write"),
                "{goal}"
            );
            assert!(contract
                .validate_proposal("add_reminder", &json!({"title":"Injected"}), &[])
                .is_err());
        }
    }
    #[test]
    fn drafting_a_reply_in_chat_does_not_require_a_mail_write() {
        let goal = "Draft a friendly reply to this invitation. Accept Saturday November 7 at 2pm, mention a vegetarian dish, and ask whether there is parking. Do not send anything.";
        let contract = TaskContract::from_request(goal);
        assert!(contract.outcomes.iter().all(|outcome| outcome.kind == "answer"));
        assert!(contract.validate_answer("Hi Maya, I would love to join on November 7 at 2pm. I will bring a vegetarian dish. Is parking available?", &[]).is_ok());
        assert!(contract.validate_proposal("draft_email", &json!({"subject":"Lunch","body":"Reply"}), &[]).is_err());
        assert!(TaskContract::from_request("Draft an email to my landlord").outcomes.iter().any(|outcome| outcome.tool.as_deref() == Some("draft_email")));
    }

    #[test]
    fn a_note_in_a_shared_folder_can_use_file_search() {
        let contract = TaskContract::from_request("Find the cobalt bicycle repair note by its contents and tell me the appointment time");
        let read = contract.outcomes.iter().find(|outcome| outcome.kind == "read").unwrap();
        assert!(read.accepted_tools.contains(&"search_files".into()));
        assert!(read.accepted_tools.contains(&"find_notes".into()));
        assert_eq!(read_tools("find the repair note in notes app"),vec!["find_notes"]);
        assert_eq!(read_tools("search files for calendar notes"),vec!["search_files"]);
    }

    #[test]
    fn requested_source_precedes_arithmetic_and_cannot_be_replaced_by_a_guess() {
        let goal = "Compare my Alder vendor quotes and calculate the difference.";
        let contract = TaskContract::from_request(goal);
        assert_eq!(contract.next_outcome().unwrap().accepted_tools,vec!["search_files"]);
        assert!(contract.validate_proposal("calculate",&json!({"expressions":["100+200"]}),&[]).is_err());
        assert!(contract.outcomes.iter().any(|outcome|outcome.accepted_tools==vec!["calculate"]));
        let calculation = TaskContract::from_request("Split 210 plus 90 equally among three people.");
        assert_eq!(calculation.next_outcome().unwrap().accepted_tools,vec!["split_costs"]);
        assert!(calculation.validate_proposal("split_costs",&json!({"participants":["A","B","C"],"charges":["Room|210|*","Food|90|*"]}),&[]).is_ok());
    }

    #[test]
    fn named_plans_filenames_and_a_just_added_document_require_actual_reads() {
        for goal in ["Read my Seabright travel plan and tell me the departure.","Read quarterly-planning-memo.txt and tell me the decision."] {
            let contract=TaskContract::from_request(goal);
            assert_eq!(contract.next_outcome().unwrap().accepted_tools,vec!["search_files"],"{goal}");
            assert!(contract.validate_answer("I guess the departure is 17:00",&[]).is_err());
        }
        let contract=TaskContract::from_request("I just added my Cedar warranty note. Please find it and tell me its deadline.");
        assert_eq!(contract.next_outcome().unwrap().required_arguments,json!({"query":"Cedar warranty"}));
        assert_eq!(contract.next_outcome().unwrap().kind,"read");
        let pasted=TaskContract::from_request("Summarize this policy: Remote work is Tuesday and Thursday with requests due the previous day.");
        assert!(pasted.outcomes.iter().all(|outcome|outcome.kind=="answer"));
    }

    #[test]
    fn terminal_temporal_actions_use_receipts_without_skipping_unknown_followups() {
        let goal="My library books must be returned by 18:00 on April 8, 2032. Set a reminder fifteen minutes beforehand, using exactly the title \"Return library books\".";
        let mut contract=TaskContract::from_request(goal);
        let mut reminder=step("reminder","add_reminder",json!({"title":"Return library books","due":"2032-04-08 17:45"}),true);
        reminder.evidence.as_mut().unwrap()["receipt"]["evidence"]=json!({"title":"Return library books","due":"2032-04-08 17:45","remindAt":"2032-04-08 17:45"});
        contract.refresh(goal,&[reminder.clone()]);
        let answer=contract.terminal_receipt_answer(&[reminder]).unwrap();
        assert!(answer.contains("17:45") && !answer.contains("15 minutes"));
        let goal="Create a note titled Trip, then craft a haiku";
        let mut contract=TaskContract::from_request(goal);
        let note=step("note","create_note",json!({"title":"Trip","body":"Known body"}),true);
        contract.refresh(goal,&[note.clone()]);
        assert!(contract.terminal_receipt_answer(&[note]).is_none());
    }

    #[test]
    fn requested_due_is_required_and_user_clarification_survives_refresh() {
        let goal = "Add a reminder to call Ana tomorrow";
        let mut contract = TaskContract::from_request(goal);
        contract.refresh(goal, &[]);
        assert!(contract
            .next_outcome()
            .unwrap()
            .required_fields
            .contains(&"due".into()));
        assert!(contract
            .validate_proposal("add_reminder", &json!({"title":"Call Ana"}), &[])
            .is_err());
        let outcome = contract.next_outcome().unwrap().id.clone();
        assert!(contract
            .apply_user_fields(&outcome, &json!({"unknown":"no"}))
            .is_err());
        assert!(contract
            .apply_user_fields(&outcome, &json!({"due":"2026-10-23 09:00"}))
            .is_ok());
        contract.refresh(goal, &[]);
        assert_eq!(
            contract.next_outcome().unwrap().required_arguments["due"],
            "2026-10-23 09:00"
        );
        assert!(contract
            .validate_proposal(
                "add_reminder",
                &json!({"title":"Call Ana","due":"2026-10-23 08:00"}),
                &[]
            )
            .is_err());
    }
    #[test]
    fn provenance_distinguishes_consent_edit_source_and_verification() {
        let goal = "Create a note";
        let mut contract = TaskContract::from_request(goal);
        let args = json!({"title":"Proposed","body":"Body"});
        contract.record_proposal("n", &args, None);
        assert_eq!(contract.provenance["n"]["title"].origin, "model_proposal");
        contract.record_approval("n", &args, &json!({"title":"Edited","body":"Body"}));
        assert_eq!(
            contract.provenance["n"]["title"].origin,
            "user_approved_edit"
        );
        assert_eq!(contract.provenance["n"]["body"].origin, "model_proposal");
        assert!(contract.provenance["n"]["body"].approved);
        assert!(!contract.provenance["n"]["body"].verified);
        contract.record_proposal(
            "n",
            &args,
            Some(&json!({"source":"user_request","fields":args})),
        );
        assert_eq!(contract.provenance["n"]["title"].value, "Edited");
        let note = step(
            "n",
            "create_note",
            json!({"title":"Edited","body":"Body"}),
            true,
        );
        contract.refresh(goal, &[note]);
        assert!(contract.provenance["n"]["body"].verified);
        assert_eq!(
            contract.provenance["n"]["title"]
                .previous
                .as_ref()
                .unwrap()
                .origin,
            "model_proposal"
        );
    }
    #[test]
    fn deterministic_due_keeps_observed_field_and_calculation() {
        let mut contract = TaskContract::from_request("Add a reminder before my first event");
        contract.record_proposal("r",&json!({"title":"Prepare for Standup","due":"2026-10-23 08:30"}),Some(&json!({"kind":"calendar_reminder","sourceStepId":"c","eventPath":"/events/2","event":{"id":"event-3","start":"2026-10-23T09:00:00-04:00","title":"Standup"},"minutesBefore":30})));
        let due = &contract.provenance["r"]["due"];
        assert_eq!(due.origin, "deterministic_calculation");
        assert_eq!(due.source_step_id.as_deref(), Some("c"));
        assert_eq!(due.record_id.as_deref(), Some("event-3"));
        assert_eq!(due.field_ref.as_deref(), Some("/events/2/start"));
        assert_eq!(due.calculation.as_ref().unwrap()["minutes"], 30);
    }
    #[test]
    fn revisions_preserve_verified_effects_and_failed_read_retries() {
        let goal = "Create a note, then read my calendar for 2026-10-23";
        let note = step(
            "note",
            "create_note",
            json!({"title":"Trip","body":"Things"}),
            true,
        );
        let mut failed = step(
            "failed",
            "read_calendar",
            json!({"day":"2026-10-23"}),
            false,
        );
        failed.state = "failed".into();
        let mut contract = TaskContract::from_request(goal);
        contract.refresh(goal, &[note.clone(), failed.clone()]);
        assert_eq!(contract.next_outcome().unwrap().status, "blocked");
        let revised = format!("{goal}\nAdditional details: Explain the schedule after reading it.");
        contract.refresh(&revised, &[note.clone(), failed.clone()]);
        assert_eq!(contract.revision, 2);
        assert_eq!(contract.original_request, goal);
        assert_eq!(contract.outcomes[0].completed_by, ["note"]);
        failed.state = "superseded".into();
        let read = step("retry", "read_calendar", json!({"day":"2026-10-23"}), false);
        contract.refresh(&revised, &[note, failed, read]);
        assert_eq!(contract.next_outcome().unwrap().kind, "answer");
    }
    #[test]
    fn implicit_calendar_dependency_cannot_skip_evidence_or_duplicate_explicit_read() {
        let goal="Create a reminder for my first calendar event on 2026-10-23, due 30 minutes before it starts";
        let mut contract = TaskContract::from_request(goal);
        contract.refresh(goal, &[]);
        assert_eq!(
            contract.next_outcome().unwrap().tool.as_deref(),
            Some("read_calendar")
        );
        assert!(contract
            .validate_proposal("read_calendar", &json!({"day":"2026-10-24"}), &[])
            .is_err());
        assert!(contract
            .validate_proposal("read_calendar", &json!({"day":"2026-10-23"}), &[])
            .is_ok());
        let explicit="Read my calendar for 2026-10-23, then create a reminder to prepare for my first event, due 30 minutes before it starts";
        let contract = TaskContract::from_request(explicit);
        assert_eq!(
            contract
                .outcomes
                .iter()
                .filter(|o| o.kind == "read")
                .count(),
            1
        );
    }
    #[test]
    fn literal_fields_and_approved_edits_remain_authoritative_after_reopen() {
        let goal = "Create a note titled Packing with body Passport";
        let mut contract = TaskContract::from_request(goal);
        contract.refresh(goal, &[]);
        assert_eq!(
            contract.next_outcome().unwrap().required_arguments["title"],
            "Packing"
        );
        let before = json!({"title":"Packing","body":"Passport"});
        let after = json!({"title":"Packing for Paris","body":"Passport"});
        contract.record_proposal(
            "note",
            &before,
            Some(&json!({"source":"user_request","fields":before})),
        );
        contract.record_approval("note", &before, &after);
        let mut reopened: TaskContract =
            serde_json::from_value(serde_json::to_value(&contract).unwrap()).unwrap();
        let note = step("note", "create_note", after, true);
        reopened.refresh(goal, &[note.clone()]);
        assert!(reopened.completed_action_answer(&[note]).is_some());
        assert_eq!(
            reopened.provenance["note"]["title"].origin,
            "user_approved_edit"
        );
        assert_eq!(reopened.provenance["note"]["body"].origin, "user_literal");
    }
    #[test]
    fn navigation_can_finish_from_verified_browser_observation() {
        let goal = "Open https://example.com";
        let mut contract = TaskContract::from_request(goal);
        let mut page = step(
            "page",
            "browse_page",
            json!({"url":"https://example.com"}),
            true,
        );
        page.effect = false;
        contract.refresh(goal, &[page.clone()]);
        assert!(contract
            .validate_answer("Here is the page.", &[page])
            .is_ok());
    }
    #[test]
    fn read_retry_preserves_exact_parameters_and_completed_answer_status() {
        let goal = "Read my calendar for 2026-10-23";
        let mut failed = step(
            "failed",
            "read_calendar",
            json!({"day":"2026-10-23"}),
            false,
        );
        failed.state = "failed".into();
        let mut contract = TaskContract::from_request(goal);
        contract.refresh(goal, &[failed.clone()]);
        assert!(contract
            .validate_proposal("read_calendar", &json!({"day":"2026-10-23"}), &[failed])
            .is_ok());
        let read = step("read", "read_calendar", json!({"day":"2026-10-23"}), false);
        contract.refresh(goal, &[read.clone()]);
        contract.mark_answer_complete();
        contract.refresh(goal, &[read]);
        assert!(contract.outcomes.iter().all(|o| o.status == "complete"));
    }

    #[test]
    fn repeated_write_obligations_require_distinct_verified_receipts() {
        let goal = "Create two notes";
        let mut contract = TaskContract::from_request(goal);
        let note = step(
            "note1",
            "create_note",
            json!({"title":"Trip","body":"Things"}),
            true,
        );
        contract.refresh(goal, &[note.clone()]);
        assert_eq!(
            contract
                .outcomes
                .iter()
                .filter(|o| o.status == "complete")
                .count(),
            1
        );
        assert!(contract.validate_answer("Done", &[note.clone()]).is_err());
        let second = step(
            "note2",
            "create_note",
            json!({"title":"Trip 2","body":"Other things"}),
            true,
        );
        contract.refresh(goal, &[note.clone(), second.clone()]);
        assert!(contract.validate_answer("Done", &[note, second]).is_ok());
    }

    #[test]
    fn relative_calendar_scope_is_pinned_at_creation_and_survives_clarification() {
        let base = chrono::NaiveDate::from_ymd_opt(2026, 10, 22).unwrap();
        for (relative, expected) in [
            ("today", "2026-10-22"),
            ("tomorrow", "2026-10-23"),
            ("yesterday", "2026-10-21"),
        ] {
            let goal = format!("Read my calendar {relative}");
            let mut contract = TaskContract::from_request_at(&goal, base);
            contract.refresh(&goal, &[]);
            assert_eq!(
                contract.next_outcome().unwrap().required_arguments["day"],
                expected
            );
            // Simulate an old persisted contract; refreshing does not recalculate.
            let mut reopened: TaskContract =
                serde_json::from_value(serde_json::to_value(&contract).unwrap()).unwrap();
            reopened.refresh(&goal, &[]);
            assert_eq!(
                reopened.next_outcome().unwrap().required_arguments["day"],
                expected
            );
            reopened.refresh(
                &format!("{goal}\nAdditional details: Explain the schedule."),
                &[],
            );
            assert_eq!(reopened.outcomes[0].required_arguments["day"], expected);
            assert!(reopened
                .validate_proposal("read_calendar", &json!({"day":"2030-01-01"}), &[])
                .is_err());
        }
    }
    #[test]
    fn dependent_write_cannot_use_observation_that_happened_after_it() {
        let goal = "Read my calendar for 2026-10-23, then create a note";
        let write = step(
            "early-note",
            "create_note",
            json!({"title":"Trip","body":"Things"}),
            true,
        );
        let late_read = step(
            "late-read",
            "read_calendar",
            json!({"day":"2026-10-23"}),
            false,
        );
        let mut contract = TaskContract::from_request(goal);
        contract.refresh(goal, &[write.clone(), late_read.clone()]);
        assert!(contract
            .validate_answer("Done", &[write, late_read])
            .is_err());
        assert_ne!(contract.outcomes[1].status, "complete");
    }

    #[test]
    fn coordinated_sources_and_mixed_dates_are_all_required() {
        let goal = "Read my calendar and my email, then create a note";
        let mut contract = TaskContract::from_request(goal);
        contract.refresh(goal, &[]);
        assert_eq!(
            contract
                .outcomes
                .iter()
                .filter(|o| o.kind == "read")
                .map(|o| o.tool.as_deref().unwrap())
                .collect::<Vec<_>>(),
            ["read_calendar", "read_mail"]
        );
        let calendar = step("cal", "read_calendar", json!({"day":"2026-10-23"}), false);
        contract.refresh(goal, &[calendar.clone()]);
        assert_eq!(
            contract.next_outcome().unwrap().tool.as_deref(),
            Some("read_mail")
        );
        assert!(contract
            .validate_proposal(
                "create_note",
                &json!({"title":"Summary","body":"Only calendar"}),
                &[calendar.clone()]
            )
            .is_err());
        assert!(contract.validate_answer("Done", &[calendar]).is_err());
        let goal = "Read my calendar today and 2026-11-05, then create a note";
        let mut contract = TaskContract::from_request_at(
            goal,
            chrono::NaiveDate::from_ymd_opt(2026, 10, 22).unwrap(),
        );
        contract.refresh(goal, &[]);
        assert_eq!(
            contract
                .outcomes
                .iter()
                .filter(|o| o.kind == "read")
                .map(|o| o.required_arguments["day"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["2026-10-22", "2026-11-05"]
        );
        let november = step(
            "november",
            "read_calendar",
            json!({"day":"2026-11-05"}),
            false,
        );
        contract.refresh(goal, &[november.clone()]);
        assert!(contract
            .validate_proposal(
                "create_note",
                &json!({"title":"Summary","body":"Only November"}),
                &[november]
            )
            .is_err());
    }

    #[test]
    fn partial_user_fields_and_runtime_defaults_keep_distinct_origins() {
        let mut contract = TaskContract::from_request("Add a reminder tomorrow");
        contract.refresh("Add a reminder tomorrow", &[]);
        let id = contract.next_outcome().unwrap().id.clone();
        contract
            .apply_user_fields(&id, &json!({"due":"2026-10-23 09:00"}))
            .unwrap();
        contract.record_proposal(
            "reminder",
            &json!({"title":"Model title","due":"2026-10-23 09:00"}),
            None,
        );
        assert_eq!(
            contract.provenance["reminder"]["due"].origin,
            "user_literal"
        );
        assert_eq!(
            contract.provenance["reminder"]["title"].origin,
            "model_proposal"
        );
        let mut contract = TaskContract::from_request("Add a calendar event");
        contract.record_proposal("event",&json!({"title":"Check-in","start":"2026-10-23 09:00","minutes":60}),Some(&json!({"kind":"user_fields","source":"user_request","fields":{"title":"Check-in","start":"2026-10-23 09:00"}})));
        assert_eq!(contract.provenance["event"]["title"].origin, "user_literal");
        assert_eq!(
            contract.provenance["event"]["minutes"].origin,
            "runtime_default"
        );
    }
}

#[cfg(test)]
#[path = "task_contract_eval_tests.rs"]
mod eval_tests;
