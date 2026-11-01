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
    /// The tools a request that named a tool the user added may use (`mcp::route`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub connector_tools: Vec<String>,
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
    /// For a value read from a source record (a contact's address): which step and record.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record: Option<Value>,
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

/// How the read before a change to an existing item is described, so a recipe can bind it.
pub(crate) const EDIT_READ: &str = "Read the item to change for: ";

/// Whether the next write is moving or renaming an event, which may read the coming
/// days one by one to find the event the user named.
fn searching_calendar(outcome: Option<&Outcome>) -> bool {
    outcome.is_some_and(|o| o.kind == "write" && o.tool.as_deref() == Some("update_calendar_event"))
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
    let text = completion::joined_edit_lists(&completion::unquoted(
        &goal.replace("Additional details: ", "\n"),
    ));
    let mut result: Vec<String> = Vec::new();
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
                        "turn",
                        "lock",
                        "unlock",
                        "dim",
                        "brighten",
                        "run",
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
    // Text added to a note is written as the user said it; other clauses stay lowercase.
    result
        .iter()
        .map(|clause| match completion::requested_actions(clause).0.contains(&"append_to_note") {
            true => completion::unjoined(clause, goal),
            false => clause.clone(),
        })
        .collect()
}
/// "What did Ana say about the lease?", "Did Sam mention the trip?": recalling what
/// someone said, wherever it was, rather than reading one app now.
fn recalls(clause: &str) -> bool {
    let text = clause.to_lowercase();
    let said = [" say", " said", " tell me", " told me", " mention", " write", " wrote", " send me", " sent me", " ask", " asked"];
    ((text.starts_with("what did ") || text.starts_with("when did ") || text.starts_with("did ") || text.starts_with("where did "))
        && said.iter().any(|word| text.contains(word)))
        || ["what do you know about ", "what do i know about ", "do you remember ", "do you know anything about "]
            .iter()
            .any(|lead| text.starts_with(lead))
}
pub(crate) fn read_tools(clause: &str) -> Vec<&'static str> {
    // Online topic words such as "calendar apps" do not authorize local reads.
    if crate::search_intent::explicit_request(clause).is_some() {
        return vec!["look_up_online"];
    }
    if recalls(clause) {
        return vec!["search_memory"];
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
    // "Which shortcuts do I have?" lists the user's own shortcuts.
    if has(&["shortcuts"])
        && words
            .first()
            .is_some_and(|word| ["what", "which", "list", "show"].contains(word))
    {
        return vec!["list_shortcuts"];
    }
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
    // "Find Ana's email address": a person in Contacts, not a search of the inbox. A
    // clause that names the inbox, calendar, notes or reminders still reads that source.
    let person = has(&["contact", "contacts"])
        || clause.contains("email address")
        || clause.contains("phone number");
    let unaddressed = clause.replace("email address", " ");
    let elsewhere = tokens(&unaddressed).iter().any(|word| {
        [
            "mail", "email", "emails", "inbox", "calendar", "event", "events", "appointments",
            "meetings", "note", "notes", "reminders",
        ]
        .contains(word)
    });
    if has(&["files", "file", "folder", "folders"]) {
        vec!["search_files"]
    } else if person && !elsewhere {
        vec!["find_contact"]
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
            "reply_email",
            "create_saved_search",
            "create_price_watch",
            crate::routines::TOOL,
            crate::goals::TOOL,
            "remember",
            "set_timer",
            crate::mac_settings::TOOL,
            "control_music",
            "switch_tab",
            "browse_page",
            "complete_reminder",
            "reschedule_reminder",
            "update_calendar_event",
            "append_to_note",
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
        SuccessVerifier::ScopedMemoryResults => evidence
            .get("records")
            .is_some_and(Value::is_array)
            && evidence["kind"] == "memory",
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

/// The answer to a request sent to a tool the user added: what is known from its receipt
/// first (it replied; Buntini can't check what it changed), then the model's words about
/// the reply. English parsing of built-in actions doesn't apply to the tool's request.
fn connector_answer(text: &str, steps: &[Step]) -> Result<String, String> {
    let ran: Vec<(&Step, ActionReceipt)> = steps
        .iter()
        .filter(|step| crate::mcp::is_tool(&step.tool) && step.state == "succeeded" && verified_write(step))
        .filter_map(|step| {
            let receipt = step.evidence.as_ref()?.get("receipt")?.clone();
            Some((step, serde_json::from_value(receipt).ok()?))
        })
        .collect();
    if ran.is_empty() {
        return Err("The tool you added hasn’t confirmed a reply. This task is unfinished.".into());
    }
    // Only calls the user approved could have changed anything. After reads alone, an
    // answer saying something was done is not one the receipts back.
    if !ran.iter().any(|(step, _)| step.effect) && completion::claims_own_write(text) {
        return Err("Only a tool that reads ran, so nothing was changed. This task is unfinished: the call that does what you asked hasn’t been approved.".into());
    }
    let mut said: Vec<&str> = Vec::new();
    for (_, receipt) in &ran {
        if !said.contains(&receipt.message.as_str()) {
            said.push(&receipt.message);
        }
    }
    Ok(format!("{}\n\n{}", said.join("\n"), text.trim()))
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
    /// "Use GitHub to …": the request goes to the tools the user added under that name
    /// alone, as one call (shown for approval unless marked safe to read) and an answer.
    pub fn use_connector(&mut self, tools: Vec<String>) {
        self.connector_tools = tools;
        self.constraints.clear();
        let goal = self.current_request.clone();
        self.bootstrap(&goal);
    }
    fn connector_request(&self) -> bool {
        !self.connector_tools.is_empty()
    }
    fn bootstrap(&mut self, goal: &str) {
        self.outcomes.clear();
        if self.connector_request() {
            let tools = self.connector_tools.clone();
            self.push_outcome("write", goal, tools, json!({}), vec![]);
            self.push_outcome("answer", goal, vec![], json!({}), vec![]);
            self.intent_status = "connector_request".into();
            return;
        }
        // A goal is one write too: its aim, check-ins, last day and end are the user's own
        // words, and each check-in is its own task later.
        if let Some(request) = crate::goals::parse_request(goal) {
            let arguments = request.arguments();
            let fields = arguments.as_object().into_iter().flatten().map(|(field, _)| field.clone()).collect();
            self.push_outcome("write", goal, vec![crate::goals::TOOL.into()], arguments.clone(), fields);
            let id = self.outcomes[0].id.clone();
            for (field, value) in arguments.as_object().into_iter().flatten() {
                self.constraints.push(Constraint { outcome_id: Some(id.clone()), field: field.clone(), value: value.clone(), source: "user_literal".into(), revision: self.revision, record: None });
            }
            self.intent_status = "explicit_goal".into();
            return;
        }
        // A routine is one write whose request, days and time are the user's own words.
        // What the routine will later do is its own task each time it runs.
        if let Some(routine) = crate::routines::parse_request(goal) {
            let arguments = routine.arguments();
            self.push_outcome("write", goal, vec![crate::routines::TOOL.into()], arguments.clone(), vec!["task".into(), "days".into(), "time".into()]);
            let id = self.outcomes[0].id.clone();
            for (field, value) in arguments.as_object().into_iter().flatten() {
                self.constraints.push(Constraint { outcome_id: Some(id.clone()), field: field.clone(), value: value.clone(), source: "user_literal".into(), revision: self.revision, record: None });
            }
            self.intent_status = "explicit_routine".into();
            return;
        }
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
                // A change to an existing item first reads where that item lives. Its
                // id is bound from that read or picked in the sheet, never required of
                // a proposal (`edit_targets::check` enforces where it came from).
                if let Some((read, arguments)) =
                    crate::edit_targets::source(tool, &clause, reference_date)
                {
                    required.retain(|field| field != "id");
                    if !self.outcomes.iter().any(|o| {
                        o.kind == "read"
                            && o.accepted_tools.iter().any(|t| t == read)
                            && o.required_arguments == arguments
                    }) {
                        self.push_outcome(
                            "read",
                            &format!("{EDIT_READ}{clause}"),
                            vec![read.into()],
                            arguments,
                            vec![],
                        );
                    }
                }
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
        // "I just added my Marigold warranty note. Find it": "it" is that document, so
        // the search is for its name without the kind of document it is.
        let plain = completion::unquoted(goal);
        let pronoun_read = plain.split(['.',';','\n']).any(|clause| {
            let clause=completion::request_clause(clause.trim());
            ["find it","read it","check it"].iter().any(|prefix|clause==*prefix || clause.starts_with(&format!("{prefix} ")))
        });
        let declared = pronoun_read.then(|| goal.split(['.',';','\n']).find_map(|sentence| {
            let sentence=sentence.trim();
            let lower=sentence.to_ascii_lowercase();
            ["i just added my ","i added my ","i just saved my ","i saved my "]
                .iter().find_map(|prefix|lower.starts_with(prefix).then(||sentence[prefix.len()..].trim()))
        })).flatten().filter(|title| {
            let lower=title.to_ascii_lowercase();
            [" note"," document"," file"].iter().any(|suffix|lower.ends_with(suffix)) && title.len()<=200
        }).map(|title| title.rsplit_once(' ').map(|(head,_)|head).unwrap_or(title).trim()).filter(|query| !query.is_empty());
        if let Some(query) = declared {
            let unscoped = self.outcomes.iter().enumerate()
                .filter(|(_,o)| o.kind=="read" && o.accepted_tools.iter().any(|t| t=="search_files" || t=="find_notes"))
                .map(|(index,o)| (index, o.required_arguments.as_object().is_none_or(|args| args.is_empty())))
                .collect::<Vec<_>>();
            match unscoped[..] {
                [] if !self.outcomes.iter().any(|outcome|outcome.kind=="read") => {
                    self.push_outcome("read","Read the document the user just identified",vec!["find_notes".into(),"search_files".into()],json!({"query":query}),vec!["query".into()]);
                }
                [(index, true)] => {
                    let outcome = &mut self.outcomes[index];
                    outcome.accepted_tools = vec!["find_notes".into(),"search_files".into()];
                    outcome.required_arguments = json!({"query":query});
                    outcome.required_fields = vec!["query".into()];
                }
                _ => {}
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
                self.constraints.push(Constraint {outcome_id:Some(outcome.id.clone()),field:field.into(),value,source:"user_literal".into(),revision:self.revision,record:None});
            }
        }
        // Each timer keeps the length its own words gave it, so the second of two
        // timers is never a copy of the first.
        for outcome in self.outcomes.iter_mut().filter(|o| o.kind=="write" && o.tool.as_deref()==Some("set_timer")) {
            if let Some(seconds) = crate::explicit_fields::timer_seconds(&outcome.description) {
                outcome.required_arguments["seconds"] = json!(seconds);
                self.constraints.push(Constraint {outcome_id:Some(outcome.id.clone()),field:"seconds".into(),value:json!(seconds),source:"user_literal".into(),revision:self.revision,record:None});
            }
        }
        // "My training session runs from 14:15 to 14:50 on May 21, 2031": an event from the
        // user's own words keeps the date, start and length they gave, read the way a
        // note's are (`source_fields::note_event`). The place is `supplied_location`'s.
        let events = self.outcomes.iter().enumerate()
            .filter(|(_,o)| o.kind=="write" && o.tool.as_deref()==Some("add_calendar_event"))
            .map(|(index,_)| index).collect::<Vec<_>>();
        if let ([index], false) = (&events[..], self.outcomes.iter().any(|o| o.kind=="read")) {
            for (field, value) in crate::source_fields::note_event(&completion::unquoted(goal)) {
                let outcome = &mut self.outcomes[*index];
                if field == "location" || outcome.required_arguments.get(field).is_some() {
                    continue;
                }
                outcome.required_arguments[field] = value.clone();
                self.constraints.push(Constraint {outcome_id:Some(outcome.id.clone()),field:field.into(),value,source:"user_literal".into(),revision:self.revision,record:None});
            }
        }
        // "My books must be returned by 18:00 on May 21, 2031. Set a reminder fifteen
        // minutes beforehand": the due time is the one the request gives, less the lead
        // it names. A lead it names in a way this can't read leaves the time unbound.
        let reminders = self.outcomes.iter().enumerate()
            .filter(|(_,o)| o.kind=="write" && o.tool.as_deref()==Some("add_reminder"))
            .map(|(index,_)| index).collect::<Vec<_>>();
        if let ([index], false) = (&reminders[..], self.outcomes.iter().any(|o| o.kind=="read")) {
            let plain = completion::unquoted(goal).to_lowercase();
            let time = crate::source_fields::note_event(&plain).into_iter().find(|(field,_)| *field=="start")
                .and_then(|(_,value)| chrono::NaiveDateTime::parse_from_str(value.as_str()?, "%Y-%m-%d %H:%M").ok());
            let leads = plain.split(['.',';','\n'])
                .filter(|sentence| sentence.split(|c:char| !c.is_alphanumeric()).any(|w| ["before","beforehand","earlier","ahead","prior"].contains(&w)))
                .collect::<Vec<_>>();
            let lead = match leads[..] {
                [] => Some(0),
                [sentence] => crate::explicit_fields::timer_seconds(sentence).map(|seconds| seconds as i64 / 60),
                _ => None,
            };
            if let (Some(time), Some(lead)) = (time, lead) {
                let outcome = &mut self.outcomes[*index];
                if outcome.required_arguments.get("due").is_none() {
                    let due = json!((time - chrono::Duration::minutes(lead)).format("%Y-%m-%d %H:%M").to_string());
                    outcome.required_arguments["due"] = due.clone();
                    self.constraints.push(Constraint {outcome_id:Some(outcome.id.clone()),field:"due".into(),value:due,source:"user_literal".into(),revision:self.revision,record:None});
                }
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
                                record: None,
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
                        // A safe-to-read call to an added tool is not an effect, but it
                        // still completes the call the request asked for.
                        (step.effect || step.tool == "browse_page" || crate::mcp::is_tool(&step.tool))
                            && verified_write(step)
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
        self.bind_contact_recipient(steps);
        self.bind_note_event(steps);
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
    /// "Add that workshop to my calendar" after reading the one note it is in: the
    /// event's date and time, length and place come from the note's own words
    /// (`source_fields::note_event`), not the model's. A kind of value the request
    /// itself gives stays the user's, and once an event is proposed the sheet decides.
    fn bind_note_event(&mut self, steps: &[Step]) {
        if steps.iter().any(|s| s.tool == "add_calendar_event") {
            return;
        }
        let Some(event) = self.next_outcome().filter(|o| {
            o.kind == "write" && o.status == "ready" && o.tool.as_deref() == Some("add_calendar_event")
        }) else {
            return;
        };
        let reads = self
            .outcomes
            .iter()
            .filter(|o| o.kind == "read" && o.status == "complete" && event.dependencies.contains(&o.id))
            .flat_map(|o| &o.completed_by)
            .filter_map(|id| steps.iter().find(|s| &s.id == id && ["find_notes", "search_files"].contains(&s.tool.as_str())))
            .collect::<Vec<_>>();
        let [step] = reads[..] else {
            return;
        };
        let Some([record]) = step.evidence.as_ref().and_then(|e| e["records"].as_array()).map(|r| &r[..]) else {
            return;
        };
        let (Some(text), true) = (record["excerpt"].as_str(), record["contentComplete"] == true) else {
            return;
        };
        let text = record["title"].as_str().and_then(|title| text.strip_prefix(title)).unwrap_or(text);
        let request = &self.current_request;
        let said = |field: &str| match field {
            "start" => crate::source_fields::note_event(request).iter().any(|(f, _)| *f == "start")
                || crate::completion::unquoted(request).split(|c: char| !c.is_alphanumeric()).any(|w| {
                    ["today", "tomorrow", "tonight", "monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday"].contains(&w)
                }),
            "minutes" => crate::explicit_fields::timer_seconds(request).is_some(),
            _ => crate::location_fields::supplied_location(request).is_some(),
        };
        let fields = crate::source_fields::note_event(text)
            .into_iter()
            .filter(|(field, _)| !said(field))
            .collect::<Vec<_>>();
        let record = json!({"app":if step.tool == "find_notes" {"notes"} else {"files"},"stepId":step.id,"recordId":record["id"]});
        for (field, value) in fields {
            if self.bind_write_field("add_calendar_event", field, value, "note_record") {
                if let Some(constraint) = self.constraints.last_mut() {
                    constraint.record = Some(record.clone());
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
            record: None,
        });
        true
    }
    /// The next requested email draft while nothing the user wrote, approved or had on
    /// screen has fixed its recipient. Only then may Contacts be read for it.
    fn awaiting_recipient(&self) -> Option<&Outcome> {
        self.next_outcome().filter(|o| {
            o.kind == "write"
                && o.status == "ready"
                && o.tool.as_deref() == Some("draft_email")
                && !self
                    .constraints
                    .iter()
                    .any(|c| c.outcome_id.as_deref() == Some(&o.id) && c.field == "to")
        })
    }
    /// The person to look up before drafting, when the user named the recipient and no
    /// lookup for that name has run yet. The controller reads Contacts with it itself.
    pub fn contact_lookup(&self, steps: &[Step]) -> Option<String> {
        self.awaiting_recipient()?;
        if steps.iter().any(|s| s.tool == "draft_email") {
            return None;
        }
        let name = crate::source_fields::named_recipient(&self.current_request)?;
        (crate::source_fields::user_named(&self.current_request, &name)
            && !steps
                .iter()
                .any(|s| s.tool == "find_contact" && s.arguments["name"] == name.as_str()))
        .then_some(name)
    }
    /// Addresses the draft from a Contacts read in this journal, only when exactly one
    /// person with exactly one address matched a name the user wrote. Once a draft has
    /// been proposed, the recipient is the user's to change in the approval sheet.
    fn bind_contact_recipient(&mut self, steps: &[Step]) {
        if steps.iter().any(|s| s.tool == "draft_email") {
            return;
        }
        let crate::source_fields::ContactRecipient::One {
            address,
            step_id,
            record_id,
        } = crate::source_fields::contact_recipient(&self.current_request, steps)
        else {
            return;
        };
        if self.bind_write_field("draft_email", "to", Value::String(address), "contact_record") {
            if let Some(constraint) = self.constraints.last_mut() {
                constraint.record =
                    Some(json!({"app":"contacts","stepId":step_id,"recordId":record_id}));
            }
        }
    }
    /// Addresses the user can pick from when several people or addresses matched the
    /// person they named. Empty once the recipient is fixed or there is nothing to pick.
    pub fn recipient_choices(&self, steps: &[Step]) -> Vec<crate::source_fields::ContactChoice> {
        if self.awaiting_recipient().is_none() {
            return vec![];
        }
        match crate::source_fields::contact_recipient(&self.current_request, steps) {
            crate::source_fields::ContactRecipient::Several(choices) => choices,
            _ => vec![],
        }
    }
    /// The next requested shortcut run while nothing has fixed which shortcut it is.
    fn awaiting_shortcut(&self) -> Option<&Outcome> {
        self.next_outcome().filter(|o| {
            o.kind == "write"
                && o.status == "ready"
                && o.tool.as_deref() == Some("run_shortcut")
                && !self
                    .constraints
                    .iter()
                    .any(|c| c.outcome_id.as_deref() == Some(&o.id) && c.field == "name")
        })
    }
    /// The user's own words for a requested shortcut run: its own clause, so "and lock
    /// the front door" or "Don't unlock the door" never picks another shortcut. A single
    /// run also takes a name the user quoted anywhere ("run my “Start work” shortcut").
    fn shortcut_query(&self, outcome: &Outcome) -> String {
        let runs = self
            .outcomes
            .iter()
            .filter(|o| o.tool.as_deref() == Some("run_shortcut"))
            .count();
        let words = if runs == 1 {
            crate::edit_targets::clause(&self.current_request, "run_shortcut")
                .unwrap_or_else(|| self.current_request.clone())
        } else {
            outcome.description.clone()
        }
        .replace(['“', '”', '"', '‘', '’', '`'], " ");
        let mut query = String::new();
        for word in words.split_whitespace() {
            if query.chars().count() + word.chars().count() + 1 > 100 {
                break;
            }
            if !query.is_empty() {
                query.push(' ');
            }
            query.push_str(word);
        }
        query
    }
    /// The words to read the user's shortcuts with before a run, when nothing has fixed
    /// which shortcut it is and no read for those words is in this journal yet. The
    /// controller reads them itself, so a model never has to guess a shortcut's name.
    pub fn shortcut_lookup(&self, steps: &[Step]) -> Option<String> {
        let outcome = self.awaiting_shortcut()?;
        let query = self.shortcut_query(outcome);
        (!query.is_empty() && self.shortcut_read(steps, &query).is_none()).then_some(query)
    }
    /// The latest verified read of the user's shortcuts for exactly these words.
    fn shortcut_read<'a>(&self, steps: &'a [Step], query: &str) -> Option<&'a Step> {
        steps.iter().rev().find(|step| {
            step.tool == "list_shortcuts"
                && !step.effect
                && step.state == "succeeded"
                && step.arguments["query"] == query
                && step.evidence.as_ref().is_some_and(|evidence| {
                    crate::apps::validate_evidence(&step.tool, &step.arguments, evidence, false)
                        .is_ok()
                })
        })
    }
    /// The user's shortcuts that fit the next requested run, most specific first, read
    /// in this journal. Empty before that read or when the run is already fixed.
    pub fn shortcut_choices(&self, steps: &[Step]) -> Vec<String> {
        let Some(outcome) = self.awaiting_shortcut() else {
            return vec![];
        };
        self.shortcut_read(steps, &self.shortcut_query(outcome))
            .and_then(|step| step.evidence.as_ref())
            .and_then(|evidence| evidence["records"].as_array())
            .into_iter()
            .flatten()
            .filter_map(|record| record["name"].as_str().map(str::to_owned))
            .collect()
    }
    /// The shortcuts the user picks from in the sheet when `name` is proposed: every one
    /// that fits, unless `name` plainly names what they asked for or nothing else fits.
    pub fn shortcut_alternatives(&self, steps: &[Step], name: &str) -> Vec<String> {
        let Some(outcome) = self.awaiting_shortcut() else {
            return vec![];
        };
        let choices = self.shortcut_choices(steps);
        if choices.len() < 2 || crate::apps::shortcut_covers(&self.shortcut_query(outcome), name) {
            return vec![];
        }
        choices
    }
    /// The shortcut to propose for the next requested run, with where its name came from:
    /// the one the user's words plainly name, else the most specific that fits (the sheet
    /// then offers the others). Err when none of the user's shortcuts fits, so nothing is
    /// guessed. None before the read, or once the run is proposed.
    pub fn shortcut_proposal(&self, steps: &[Step]) -> Result<Option<(String, Value)>, String> {
        let Some(outcome) = self.awaiting_shortcut() else {
            return Ok(None);
        };
        let query = self.shortcut_query(outcome);
        let Some(step) = self.shortcut_read(steps, &query) else {
            return Ok(None);
        };
        let choices = self.shortcut_choices(steps);
        // When none plainly names what was asked, the first is only a placeholder: the
        // sheet offers every one that fits with none chosen (`shortcut_alternatives`).
        let Some(name) = choices
            .iter()
            .find(|name| crate::apps::shortcut_covers(&query, name))
            .or(choices.first())
        else {
            return Err("None of your shortcuts fits that, so nothing ran. Make one in the Shortcuts app, name it after what it does (for example “Living Room Lights On”), then ask me again.".into());
        };
        Ok(Some((
            name.clone(),
            json!({"kind":"shortcut_record","source":"shortcuts","stepId":step.id,"recordId":name,"query":query,"choices":choices.len()}),
        )))
    }
    /// Fills fields of the next write that Buntini resolved from a source (the open
    /// email's sender, the one matching contact) when a model left them out; a different
    /// value is still refused by `validate_proposal`. What a note states about the event
    /// copied from it replaces the model's version, which is the mistake it prevents.
    pub fn fill_resolved_fields(&self, tool: &str, arguments: &mut Value) {
        let Some(outcome) = self
            .next_outcome()
            .filter(|o| o.kind == "write" && o.accepted_tools.iter().any(|t| t == tool))
        else {
            return;
        };
        let Some(fields) = arguments.as_object_mut() else {
            return;
        };
        for constraint in self.constraints.iter().filter(|c| {
            c.outcome_id.as_deref() == Some(&outcome.id)
                && ["contact_record", "focus_referent", "mail_listing", "mail_search", "note_record"].contains(&c.source.as_str())
        }) {
            if constraint.source == "note_record" {
                fields.insert(constraint.field.clone(), constraint.value.clone());
            } else {
                fields
                    .entry(constraint.field.clone())
                    .or_insert_with(|| constraint.value.clone());
            }
        }
    }
    /// "Reply to Ana" with no email bound yet: the inbox search for the sender the user
    /// named, until it is in the journal (`recipes::reply_read`).
    pub fn reply_lookup(&self, steps: &[Step]) -> Option<Value> {
        if !self.unbound_reply() {
            return None;
        }
        crate::recipes::reply_read(&self.current_request, steps)
    }
    /// Whether a requested reply has nothing bound for the email it answers. Only the
    /// email on screen, one Buntini listed or one its inbox search found by the sender
    /// the user named can be (recipes.rs); without any there is nothing to reply to, so
    /// the task says so instead of planning a guess.
    pub fn unbound_reply(&self) -> bool {
        self.outcomes.iter().any(|o| {
            o.kind == "write"
                && o.status != "complete"
                && o.tool.as_deref() == Some("reply_email")
                && !self
                    .constraints
                    .iter()
                    .any(|c| c.outcome_id.as_deref() == Some(&o.id) && c.field == "message")
        })
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
                record: None,
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
        let contacts: Vec<_> = self
            .constraints
            .iter()
            .filter(|c| c.outcome_id == outcome_id && c.source == "contact_record")
            .cloned()
            .collect();
        let notes: Vec<_> = self
            .constraints
            .iter()
            .filter(|c| c.outcome_id == outcome_id && c.source == "note_record")
            .cloned()
            .collect();
        // The email on screen or one Buntini listed: Mail's own values, not the model's.
        let mail: Vec<_> = self
            .constraints
            .iter()
            .filter(|c| c.outcome_id == outcome_id && ["focus_referent", "mail_listing", "mail_search"].contains(&c.source.as_str()))
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
            } else if let Some(constraint) = contacts
                .iter()
                .find(|c| c.field == *field && c.value == *value)
            {
                // The one address of the one person the user named, read from Contacts.
                let record = constraint.record.clone().unwrap_or_default();
                provenance.origin = "contact_record".into();
                provenance.source_step_id = record["stepId"].as_str().map(str::to_owned);
                provenance.record_id = record["recordId"].as_str().map(str::to_owned);
                provenance.field_ref = Some("emails/0".into());
                provenance.source = Some(
                    json!({"kind":"task_constraint","source":"contacts","outcomeId":constraint.outcome_id,"revision":constraint.revision,"field":field}),
                );
            } else if let Some(constraint) = mail
                .iter()
                .find(|c| c.field == *field && c.value == *value)
            {
                provenance.origin = constraint.source.clone();
                provenance.field_ref = Some(field.clone());
                provenance.source = Some(
                    json!({"kind":"task_constraint","source":"mail","outcomeId":constraint.outcome_id,"revision":constraint.revision,"field":field}),
                );
            } else if let Some(constraint) = notes
                .iter()
                .find(|c| c.field == *field && c.value == *value)
            {
                // What the note read for this event says, in its own words.
                let record = constraint.record.clone().unwrap_or_default();
                provenance.origin = "note_record".into();
                provenance.source_step_id = record["stepId"].as_str().map(str::to_owned);
                provenance.record_id = record["recordId"].as_str().map(str::to_owned);
                provenance.field_ref = Some(field.clone());
                provenance.source = Some(
                    json!({"kind":"task_constraint","source":record["app"],"outcomeId":constraint.outcome_id,"revision":constraint.revision,"field":field}),
                );
            }
            if let Some(source) = source.filter(|_| provenance.origin != "note_record") {
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
                } else if source.get("kind").and_then(Value::as_str) == Some("shortcut_record")
                    && field == "name"
                    && source.get("recordId") == Some(value)
                {
                    // A name read from the user's own shortcuts for their words.
                    provenance.origin = "shortcut_record".into();
                    provenance.source_step_id =
                        source.get("stepId").and_then(Value::as_str).map(str::to_owned);
                    provenance.record_id = value.as_str().map(str::to_owned);
                    provenance.field_ref = Some("name".into());
                } else if source.get("kind").and_then(Value::as_str) == Some("source_record") {
                    // A change to an item read in this task: which read and record it is.
                    let user = source.pointer("/userFields").and_then(|f| f.get(field)) == Some(value);
                    provenance.origin = if user { "user_literal" } else { "source_record" }.into();
                    provenance.source_step_id = source["stepId"].as_str().map(str::to_owned);
                    provenance.record_id = source["recordId"].as_str().map(str::to_owned);
                    provenance.field_ref = Some(field.clone());
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
                    record: None,
                });
                if let Some(outcome) = self.outcomes.iter_mut().find(|o| o.id == outcome_id) {
                    outcome.required_arguments[field] = value.clone();
                }
            }
            // A field the user cleared in the sheet (a To address) is theirs to leave out.
            for field in before
                .as_object()
                .into_iter()
                .flatten()
                .map(|(field, _)| field)
                .filter(|field| after.get(field.as_str()).is_none())
            {
                self.constraints.retain(|c| {
                    c.outcome_id.as_deref() != Some(outcome_id.as_str()) || c.field != *field
                });
                if let Some(arguments) = self
                    .outcomes
                    .iter_mut()
                    .find(|o| o.id == outcome_id)
                    .and_then(|o| o.required_arguments.as_object_mut())
                {
                    arguments.remove(field);
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
    /// A request that names no source, place, app, number or lookup ("three cozy ideas
    /// for a rainy morning", "hey") is answered straight away. Offering it every read
    /// tool made the on-device model spend seconds choosing one, and often pick a search
    /// it didn't need, before the first word appeared.
    pub fn plain_conversation(&self, steps: &[Step]) -> bool {
        const CUES: &[&str] = &[
            // Apps and data on this Mac
            "calendar", "schedule", "meeting", "meetings", "event", "events", "appointment", "agenda", "reminder", "reminders", "remind", "todo", "to-do", "email", "emails", "mail", "inbox", "reply", "note",
            "notes", "contact", "contacts", "file", "files", "folder", "document", "documents", "pdf", "music", "song",
            "playing", "playlist", "tab", "tabs", "page", "browser", "shortcut", "shortcuts", "timer", "alarm",
            // Recall
            "remember", "recall", "said", "told", "mentioned", "last", "earlier", "forgot", "know", "my",
            // This Mac
            "mac", "computer", "laptop", "system", "battery", "memory", "ram", "cpu", "storage", "disk", "space", "slow", "wifi", "volume", "brightness",
            "dark", "lock", "open", "app", "wallpaper", "dock", "wi-fi", "screensaver", "appearance", "desktop",
            // Online
            "weather", "forecast", "rain", "raining", "snow", "temperature", "news", "latest", "current", "price",
            "prices", "cost", "buy", "cheapest", "flight", "flights", "hotel", "exchange", "rate", "convert", "currency",
            "stock", "score", "near", "nearby", "restaurant", "directions", "search", "look", "find", "online", "web",
            "website", "http", "https", "www", "wikipedia", "who", "where",
        ];
        if !steps.is_empty() || self.connector_request() {
            return false;
        }
        let plain_outcomes = self.outcomes.iter().all(|outcome| {
            outcome.kind == "answer" && outcome.resources.is_empty() && outcome.accepted_tools.is_empty()
        });
        let request = self.current_request.to_lowercase();
        let arithmetic = request.chars().any(|c| c.is_ascii_digit()) && request.chars().any(|c| "+-*/%×÷=".contains(c));
        let words: Vec<&str> = request.split(|c: char| !c.is_alphanumeric() && c != '-').filter(|w| !w.is_empty()).collect();
        // A day on its own is not a cue ("a rainy Saturday morning"); asking what is on it is.
        const DAYS: &[&str] = &["today", "tomorrow", "tonight", "weekend", "monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday"];
        let schedule_question = words.iter().any(|word| DAYS.contains(word))
            && ["what's on", "whats on", "what is on", "do i have", "am i free", "am i busy", "anything on", "plans for", "what's happening"]
                .iter()
                .any(|phrase| request.replace('’', "'").contains(phrase));
        plain_outcomes && !arithmetic && !schedule_question && !words.iter().any(|word| CUES.contains(word))
    }
    pub fn relevant_tools(&self, steps: &[Step], definitions: &Value) -> Value {
        if self.plain_conversation(steps) {
            return Value::Array(Vec::new());
        }
        let unresolved = steps
            .iter()
            .any(|s| ["failed", "unknown", "running", "waiting"].contains(&s.state.as_str()));
        let next = self.next_outcome();
        let next_write = next.filter(|o| o.kind == "write" && o.status == "ready");
        // Looking up the person a draft is for is part of preparing that draft, once:
        // after that lookup the user picks among its matches in the sheet.
        let contact_read = self.contact_lookup(steps).is_some();
        // So is reading the user's shortcuts before a run, with the words they used.
        let shortcut_read = self.shortcut_lookup(steps);
        let shortcuts = self.shortcut_choices(steps);
        // And searching the inbox for the sender of an email a reply names.
        let reply_read = self.reply_lookup(steps);
        // A safe-to-read call answers a lookup, but the request may ask the tool to do
        // something more ("open an issue"): until a call the user approved has run, the
        // tools they named stay offered, a few calls at most, each shown or checked anew.
        let connector_more = self.connector_request()
            && !steps.iter().any(|step| crate::mcp::is_tool(&step.tool) && step.effect)
            && steps.iter().filter(|step| crate::mcp::is_tool(&step.tool)).count() < 4;
        let selected = definitions
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|definition| {
                let name = definition["name"].as_str()?;
                // Calls to tools the user added are only ever the requested call.
                let effect = crate::actions::TOOLS.contains(&name)
                    || crate::monitor_actions::TOOLS.contains(&name)
                    || crate::control_actions::TOOLS.contains(&name)
                    || crate::mcp::is_tool(name);
                let write_allowed = !unresolved
                    && (next_write.is_some_and(|o| o.accepted_tools.iter().any(|t| t == name))
                        || (connector_more && self.connector_tools.iter().any(|t| t == name)));
                if effect && !write_allowed && name != "control_music" {
                    return None;
                }
                let relevant_read = (contact_read && name == "find_contact")
                    || (shortcut_read.is_some() && name == "list_shortcuts")
                    || (reply_read.is_some() && name == "read_mail")
                    || (name == "read_calendar" && searching_calendar(next))
                    || next.is_some_and(|o| {
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
                if let Some(query) = shortcut_read.as_ref().filter(|_| name == "list_shortcuts") {
                    result["parameters"]["properties"]["query"]["enum"] = json!([query]);
                    result["parameters"]["required"] = json!(["query"]);
                }
                if let Some(search) = reply_read.as_ref().filter(|_| name == "read_mail") {
                    result["parameters"]["properties"]["query"]["enum"] = json!([search["query"]]);
                    result["parameters"]["required"] = json!(["query"]);
                }
                if name == "run_shortcut" && write_allowed && !shortcuts.is_empty() {
                    result["parameters"]["properties"]["name"]["enum"] = json!(shortcuts);
                }
                if write_allowed && !crate::mcp::is_tool(name) {
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
            || crate::mcp::is_tool(tool)
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
            // Moving an event the user named without a day: only the next day the
            // search for it needs, never another day or source.
            if tool == "read_calendar" && searching_calendar(Some(outcome)) {
                let goal = crate::edit_targets::outcome_goal(&outcome.description, &self.current_request, "update_calendar_event");
                return match crate::edit_targets::edit_plan(goal, "update_calendar_event", steps) {
                    Ok(crate::edit_targets::EditPlan::Read { tool: "read_calendar", arguments: wanted })
                        if wanted["day"] == arguments["day"] => Ok(()),
                    _ => Err("This source read is not relevant to the next unfinished task outcome.".into()),
                };
            }
            // Before a draft, Contacts may be read only for the one recipient the user
            // named, never for anyone else the request or a source mentions.
            if tool == "find_contact" && self.awaiting_recipient().is_some() {
                return if arguments["name"].as_str().is_some_and(|name| {
                    crate::source_fields::is_named_recipient(&self.current_request, name)
                }) {
                    Ok(())
                } else {
                    Err("Buntini looks up only the person you asked to write to.".into())
                };
            }
            // Before a reply to someone whose email isn't open or listed, the inbox is
            // searched for them by the name the user wrote, and for nothing else.
            if tool == "read_mail" {
                if let Some(search) = self.reply_lookup(steps) {
                    return if arguments["query"].as_str().is_some_and(|query| Some(query) == search["query"].as_str()) {
                        Ok(())
                    } else {
                        Err("Buntini searches your email only for the person you’re replying to.".into())
                    };
                }
            }
            // Before a shortcut runs, the user's shortcuts are read with their own words.
            if tool == "list_shortcuts" {
                if let Some(outcome) = self.awaiting_shortcut() {
                    return if arguments["query"] == self.shortcut_query(outcome).as_str() {
                        Ok(())
                    } else {
                        Err("Buntini looks for your shortcuts only with the words you used.".into())
                    };
                }
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
        // A shortcut runs only by a name read from the user's own shortcuts for this request.
        if tool == "run_shortcut"
            && self.awaiting_shortcut().is_some()
            && !arguments["name"]
                .as_str()
                .is_some_and(|name| self.shortcut_choices(steps).iter().any(|choice| choice == name))
        {
            return Err("Buntini runs only one of your own shortcuts that fits your request.".into());
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
        crate::edit_targets::check(&self.edit_goals(tool), tool, arguments, steps, false)
    }
    /// The words of each change to an existing item of this kind the request asks for,
    /// one per outcome, so two edits of the same kind each keep their own item.
    pub fn edit_goals(&self, tool: &str) -> Vec<String> {
        let mut goals: Vec<String> = Vec::new();
        for outcome in &self.outcomes {
            if outcome.kind == "write" && outcome.tool.as_deref() == Some(tool) {
                let goal = crate::edit_targets::outcome_goal(&outcome.description, &self.current_request, tool).to_string();
                if !goals.contains(&goal) {
                    goals.push(goal);
                }
            }
        }
        if goals.is_empty() {
            goals.push(self.current_request.clone());
        }
        goals
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
            return Err(format!("I didn’t get this done yet: {}. I couldn’t check it, so I’m not calling it finished.",outcome.description));
        }
        if self.outcomes.iter().any(|o| o.kind == "answer") && text.trim().is_empty() {
            return Err("The requested answer is still missing.".into());
        }
        if self.connector_request() {
            return connector_answer(text, steps);
        }
        completion::validate_answer(&self.current_request, text, steps)
            .map(|answer| completion::grounded_personal_answer(&self.current_request, &answer, steps))
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
        // What an added tool replied is answered from, never summarised by its receipt.
        if self.connector_request() || !self.outcomes.iter().any(|outcome|outcome.kind=="write")
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

    #[test]
    fn small_talk_and_creative_requests_are_offered_no_tools() {
        let offered = crate::tools::offer(&["search_memory", "system_info", "look_up_online", "read_calendar", "calculate"]);
        for goal in ["Give me three cozy ideas for a rainy Saturday morning, a couple of sentences each.", "Hey, Bantini.", "Write a haiku about autumn"] {
            let mut contract = TaskContract::from_request(goal);
            contract.refresh(goal, &[]);
            assert_eq!(contract.relevant_tools(&[], &offered), json!([]), "{goal}");
        }
        for goal in ["what's on Saturday?", "am I free tomorrow afternoon", "what's the weather in Porto", "how much memory is free on my mac", "what did Ana say about the lease", "what is 12 * 7", "who won the match last night"] {
            let mut contract = TaskContract::from_request(goal);
            contract.refresh(goal, &[]);
            assert!(contract.relevant_tools(&[], &offered).as_array().is_some_and(|tools| !tools.is_empty()), "{goal}");
        }
    }

    #[test]
    fn tell_me_and_show_me_ask_for_the_same_pinned_read() {
        let today = chrono::Local::now().date_naive();
        let tomorrow = (today + chrono::Days::new(1)).format("%Y-%m-%d").to_string();
        for goal in ["Tell me what’s on my calendar tomorrow", "show me what's on my calendar tomorrow", "what's on my calendar tomorrow"] {
            let mut contract = TaskContract::from_request(goal);
            contract.refresh(goal, &[]);
            let next = contract.next_outcome().unwrap();
            assert_eq!(next.tool.as_deref(), Some("read_calendar"), "{goal}");
            assert_eq!(next.required_arguments["day"], tomorrow.as_str(), "{goal}");
        }
    }

    #[test]
    fn a_note_list_keeps_the_users_own_and_and_commas() {
        assert_eq!(clauses("add oat milk and eggs to my shopping note"), ["add oat milk and eggs to my shopping note"]);
        assert_eq!(clauses("Add bread, jam and tea to my Shopping note"), ["Add bread, jam and tea to my Shopping note"]);
        assert_eq!(clauses("add salt & pepper to my shopping note"), ["add salt & pepper to my shopping note"]);
        assert_eq!(clauses("Add Ana's number to my Shopping note"), ["Add Ana's number to my Shopping note"]);
    }

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
    fn a_safe_read_never_stands_in_for_the_call_a_request_asked_for() {
        let goal = "Use GitHub to open an issue titled Login bug";
        let mut contract = TaskContract::from_request(goal);
        contract.use_connector(vec!["mcp_github_list_issues".into(), "mcp_github_create_issue".into()]);
        let read = Step {
            effect: false,
            approved_at: None,
            evidence: Some(json!({"receipt":ActionReceipt {outcome:ActionOutcome::Succeeded,tool:"mcp_github_list_issues".into(),object_id:None,destination:Some("GitHub".into()),message:"GitHub ran list_issues and replied. Buntini can’t check what it changed.".into(),evidence:json!({}),error:None}})),
            ..step("s1", "mcp_github_list_issues", json!({}), false)
        };
        let steps = vec![read];
        contract.refresh(goal, &steps);
        let definitions = json!([
            {"name":"mcp_github_list_issues","parameters":{"type":"object","properties":{}}},
            {"name":"mcp_github_create_issue","parameters":{"type":"object","properties":{}}}
        ]);
        let offered = contract.relevant_tools(&steps, &definitions);
        assert!(offered.as_array().unwrap().iter().any(|tool| tool["name"] == "mcp_github_create_issue"), "{offered}");
        assert!(contract.validate_answer("I've opened the issue Login bug.", &steps).is_err());
        let answer = contract.validate_answer("There are two open issues; none is called Login bug.", &steps).unwrap();
        assert!(answer.starts_with("GitHub ran list_issues and replied."), "{answer}");
        // Once a call the user approved has run, nothing more is offered.
        let mut approved = step("s2", "mcp_github_create_issue", json!({}), true);
        approved.evidence = Some(json!({"receipt":ActionReceipt {outcome:ActionOutcome::Succeeded,tool:"mcp_github_create_issue".into(),object_id:None,destination:Some("GitHub".into()),message:"GitHub ran create_issue and replied. Buntini can’t check what it changed.".into(),evidence:json!({}),error:None}}));
        let steps = vec![steps[0].clone(), approved];
        contract.refresh(goal, &steps);
        assert!(contract.relevant_tools(&steps, &definitions).as_array().unwrap().is_empty());
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
    fn shortcut_read(id: &str, query: &str, names: &[&str]) -> Step {
        let names: Vec<String> = names.iter().map(|name| name.to_string()).collect();
        Step {
            evidence: Some(crate::apps::shortcut_evidence(query, &names)),
            ..step(id, "list_shortcuts", json!({"query":query}), false)
        }
    }
    #[test]
    fn a_shortcut_run_is_bound_to_one_of_the_users_shortcuts_for_their_words() {
        let goal = "Turn on the living room lights";
        let mut contract = TaskContract::from_request(goal);
        contract.refresh(goal, &[]);
        assert_eq!(contract.next_outcome().unwrap().tool.as_deref(), Some("run_shortcut"));
        // First the user's shortcuts are read, with the words they used, once.
        assert_eq!(contract.shortcut_lookup(&[]).as_deref(), Some(goal));
        assert!(contract.validate_proposal("list_shortcuts", &json!({"query":goal}), &[]).is_ok());
        assert!(contract.validate_proposal("list_shortcuts", &json!({"query":"unlock the door"}), &[]).is_err());
        assert!(contract.validate_proposal("run_shortcut", &json!({"name":"Living Room Lights On"}), &[]).is_err(), "not before the read");
        let offered = contract.relevant_tools(&[], &crate::tools::offer(&["list_shortcuts", "run_shortcut"]));
        assert_eq!(offered[0]["parameters"]["properties"]["query"]["enum"], json!([goal]));
        assert_eq!(contract.shortcut_proposal(&[]).unwrap(), None);

        let steps = vec![shortcut_read("read-1", goal, &["Lights On", "Living Room Lights On", "Living Room Lights Off"])];
        contract.refresh(goal, &steps);
        assert_eq!(contract.shortcut_lookup(&steps), None);
        assert_eq!(contract.shortcut_choices(&steps), ["Living Room Lights On", "Lights On"]);
        let (name, source) = contract.shortcut_proposal(&steps).unwrap().unwrap();
        assert_eq!(name, "Living Room Lights On");
        assert_eq!((source["kind"].as_str(), source["stepId"].as_str()), (Some("shortcut_record"), Some("read-1")));
        let offered = contract.relevant_tools(&steps, &crate::tools::offer(&["list_shortcuts", "run_shortcut"]));
        assert_eq!(names(&offered), ["run_shortcut"]);
        assert_eq!(offered[0]["parameters"]["properties"]["name"]["enum"], json!(["Living Room Lights On", "Lights On"]));
        assert!(contract.validate_proposal("run_shortcut", &json!({"name":"Lights On"}), &steps).is_ok());
        for other in ["Living Room Lights Off", "Delete Everything", "living room lights on"] {
            assert!(contract.validate_proposal("run_shortcut", &json!({"name":other}), &steps).is_err(), "{other}");
        }
        contract.record_proposal("run-1", &json!({"name":name}), Some(&source));
        assert_eq!(contract.provenance["run-1"]["name"].origin, "shortcut_record");
        assert_eq!(contract.provenance["run-1"]["name"].source_step_id.as_deref(), Some("read-1"));

        // Nothing fits: nothing is proposed or guessed.
        let none = vec![shortcut_read("read-1", goal, &["Lights Off", "Start work"])];
        contract.refresh(goal, &none);
        assert!(contract.shortcut_proposal(&none).unwrap_err().contains("None of your shortcuts fits"));
    }
    #[test]
    fn each_of_several_shortcut_runs_is_read_with_its_own_clause() {
        let goal = "Turn on the lights and lock the front door";
        let mut contract = TaskContract::from_request(goal);
        contract.refresh(goal, &[]);
        assert_eq!(contract.shortcut_lookup(&[]).as_deref(), Some("turn on the lights"));
        let names = ["Lights On", "Lock Front Door"];
        let mut steps = vec![shortcut_read("read-1", "turn on the lights", &names)];
        contract.refresh(goal, &steps);
        assert_eq!(contract.shortcut_proposal(&steps).unwrap().unwrap().0, "Lights On");
        steps.push(step("run-1", "run_shortcut", json!({"name":"Lights On"}), true));
        contract.refresh(goal, &steps);
        assert_eq!(contract.shortcut_lookup(&steps).as_deref(), Some("lock the front door"));
        steps.push(shortcut_read("read-2", "lock the front door", &names));
        contract.refresh(goal, &steps);
        assert_eq!(contract.shortcut_proposal(&steps).unwrap().unwrap().0, "Lock Front Door");
        // A quoted name the user wrote counts as their words when it is the only run.
        let quoted = "Run my “Start work” shortcut";
        let mut contract = TaskContract::from_request(quoted);
        contract.refresh(quoted, &[]);
        assert_eq!(contract.shortcut_lookup(&[]).as_deref(), Some("Run my Start work shortcut"));
        // A single run's words are its own part of the request, never a sentence saying
        // what not to do or another request.
        for (goal, query) in [
            ("Turn on the lights. Don't unlock the front door.", "Turn on the lights"),
            ("Turn on the office lights and tell me if the garage door is open", "Turn on the office lights"),
        ] {
            let mut contract = TaskContract::from_request(goal);
            contract.refresh(goal, &[]);
            assert_eq!(contract.shortcut_lookup(&[]).as_deref(), Some(query), "{goal}");
        }
        assert_eq!(read_tools("which shortcuts do I have"), ["list_shortcuts"]);
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
    fn answering_a_question_in_chat_is_not_a_reply_email() {
        for goal in [
            "Respond to this question in one sentence: why do leases renew?",
            "Reply to the following in a friendly tone: thanks for the update",
        ] {
            let contract = TaskContract::from_request(goal);
            assert!(contract.outcomes.iter().all(|outcome| outcome.kind == "answer"), "{goal}");
            assert!(contract.validate_proposal("draft_email", &json!({"subject":"Re:","body":"Hi"}), &[]).is_err());
        }
        let reply = TaskContract::from_request("Reply to Ana's email saying yes");
        assert!(reply.outcomes.iter().any(|outcome| outcome.tool.as_deref() == Some("reply_email")));
    }

    #[test]
    fn a_reply_answers_a_bound_email_and_a_new_email_never_does() {
        let goal = "Reply to the second one saying yes";
        let mut reply = TaskContract::from_request(goal);
        reply.refresh(goal, &[]);
        let outcome = reply.next_outcome().unwrap();
        assert_eq!((outcome.kind.as_str(), outcome.tool.as_deref()), ("write", Some("reply_email")));
        assert_eq!(outcome.required_fields, ["body"], "the model writes only the body");
        // Until the email it answers is bound, there is nothing to reply to.
        assert!(reply.unbound_reply());
        assert!(reply.validate_proposal("draft_email", &json!({"to":"ana@example.com","subject":"Re: Lease","body":"Yes"}), &[]).is_err());
        for (field, value) in [("message", "4821"), ("message_id", "m-1@x.com"), ("to", "ana@example.com"), ("subject", "Re: Lease")] {
            assert!(reply.bind_write_field("reply_email", field, json!(value), "mail_listing"));
        }
        reply.refresh(goal, &[]);
        assert!(!reply.unbound_reply());
        let mut proposal = json!({"body":"Yes, that works."});
        reply.fill_resolved_fields("reply_email", &mut proposal);
        assert_eq!(proposal["message"], "4821");
        assert_eq!(proposal["to"], "ana@example.com");
        assert!(reply.validate_proposal("reply_email", &proposal, &[]).is_ok());
        let mut redirected = proposal.clone();
        redirected["message"] = json!("99");
        assert!(reply.validate_proposal("reply_email", &redirected, &[]).is_err(), "a model can't pick another email");
        let fresh = TaskContract::from_request("Draft an email to Ana about Friday");
        assert!(!fresh.unbound_reply());
        assert!(fresh.outcomes.iter().all(|outcome| outcome.tool.as_deref() != Some("reply_email")));
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
        for person in ["find ana's email address", "find sam's phone number", "check my contacts for priya"] {
            assert_eq!(read_tools(person), vec!["find_contact"], "{person}");
        }
        assert_eq!(read_tools("find the email from ana"), vec!["read_mail"]);
        // A source the user names wins over the words "phone number" or "contact".
        assert_eq!(read_tools("search my inbox for the plumber's phone number"), vec!["read_mail"]);
        assert_eq!(read_tools("check my email for ana's new email address"), vec!["read_mail"]);
        assert_eq!(read_tools("check my calendar for the contact meeting"), vec!["read_calendar"]);
        for recall in ["What did Ana say about the lease", "did Sam mention the trip", "What do you know about my landlord"] {
            assert_eq!(read_tools(recall), vec!["search_memory"], "{recall}");
        }
        assert_ne!(read_tools("what did I do today"), vec!["search_memory"]);
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

    #[test]
    fn each_of_two_timers_keeps_the_length_its_own_words_gave_it() {
        let goal = "Set a 12-minute timer for the rice and set a 6-minute timer for the vegetables.";
        let mut contract = TaskContract::from_request(goal);
        contract.refresh(goal, &[]);
        let timers = contract.outcomes.iter().filter(|o| o.tool.as_deref() == Some("set_timer")).collect::<Vec<_>>();
        assert_eq!(timers.len(), 2);
        assert_eq!(timers[0].required_arguments["seconds"], 720);
        assert_eq!(timers[1].required_arguments["seconds"], 360);
        assert!(contract.constraints.iter().all(|c| c.source == "user_literal"));
        // The second timer is the next outcome once the first is set, and only 360 fits it.
        let first = step("rice", "set_timer", json!({"seconds":720}), true);
        contract.refresh(goal, std::slice::from_ref(&first));
        assert!(contract.validate_proposal("set_timer", &json!({"seconds":720}), std::slice::from_ref(&first)).is_err());
        assert!(contract.validate_proposal("set_timer", &json!({"seconds":360}), std::slice::from_ref(&first)).is_ok());
    }

    #[test]
    fn a_document_the_user_just_added_is_searched_by_its_name() {
        let goal = "I just added my Marigold warranty note. Please find it and tell me the warranty expiry date.";
        let mut contract = TaskContract::from_request(goal);
        contract.refresh(goal, &[]);
        let read = contract.next_outcome().unwrap();
        assert_eq!(read.kind, "read");
        assert_eq!(read.required_arguments, json!({"query":"Marigold warranty"}));
        assert!(contract.validate_proposal("search_files", &json!({"query":"Marigold warranty note"}), &[]).is_err());
        assert!(contract.validate_proposal("search_files", &json!({"query":"Marigold warranty"}), &[]).is_ok());
    }

    fn note_read(id: &str, query: &str, title: &str, body: &str) -> Step {
        let excerpt = format!("{title} {body}");
        let mut read = step(id, "find_notes", json!({"query":query}), false);
        read.evidence = Some(json!({"complete":true,"count":1,"kind":"notes","observedAt":"2026-10-29T21:29:22.436216+00:00",
            "records":[{"contentComplete":true,"excerpt":excerpt,"excerptLimit":400,"excerptTruncated":false,"id":"x-coredata://note/p111",
                "modifiedAt":"2026-10-29T17:29:00-04:00","modifiedLocal":"2026-10-29 17:29",
                "reference":{"app":"notes","id":"x-coredata://note/p111","kind":"app_local","mustRevalidate":true},
                "sourceCharacterCount":excerpt.chars().count(),"title":title}],
            "schemaVersion":1,"scope":{"app":"notes","excerptLimit":400,"limit":6,"matches":"title_or_plaintext","query":query},
            "text":format!("Notes mentioning “{query}”:\n- {title} (edited Thu 29 Oct 17:29)\n  {excerpt}"),"total":1}));
        read
    }

    #[test]
    fn an_event_copied_from_a_note_takes_the_notes_date_length_and_place() {
        let goal = "Read my note mentioning QA Workshop details, then add that workshop to my calendar using exactly the title QA Workshop. Derive the duration from the note. Do not invite anyone.";
        let mut contract = TaskContract::from_request(goal);
        contract.refresh(goal, &[]);
        let read = note_read("read", "QA Workshop details", "QA Workshop details", "The workshop is on May 20, 2031. It starts at 13:20 and ends at 14:05 in Study Room. There are no attendees to invite.");
        contract.refresh(goal, std::slice::from_ref(&read));
        let event = contract.next_outcome().unwrap();
        assert_eq!(event.tool.as_deref(), Some("add_calendar_event"));
        assert_eq!(
            event.required_arguments,
            json!({"title":"QA Workshop","start":"2031-05-20 13:20","minutes":45,"location":"Study Room"})
        );
        // A model that defaults to this year is corrected, and a different length refused.
        let mut proposed = json!({"title":"QA Workshop","start":"2026-05-20 13:20","minutes":45});
        contract.fill_resolved_fields("add_calendar_event", &mut proposed);
        assert_eq!(proposed["start"], "2031-05-20 13:20");
        assert_eq!(proposed["location"], "Study Room");
        assert!(contract.validate_proposal("add_calendar_event", &proposed, std::slice::from_ref(&read)).is_ok());
        assert!(contract.validate_proposal("add_calendar_event", &json!({"title":"QA Workshop","start":"2031-05-20 13:20","minutes":60,"location":"Study Room"}), std::slice::from_ref(&read)).is_err());
        contract.record_proposal("event", &proposed, None);
        assert_eq!(contract.provenance["event"]["start"].origin, "note_record");
        assert_eq!(contract.provenance["event"]["start"].source_step_id.as_deref(), Some("read"));
        assert_eq!(contract.provenance["event"]["title"].origin, "user_literal");
    }

    #[test]
    fn an_event_keeps_the_date_and_times_the_request_gives() {
        let goal = "My training session runs from 14:15 to 14:50 on May 21, 2031 in Training Room. Add it to my calendar with exactly the title \"QA Training session\". Derive its duration from those times, and do not invite anyone.";
        let mut contract = TaskContract::from_request(goal);
        contract.refresh(goal, &[]);
        let event = contract.next_outcome().unwrap();
        assert_eq!(event.tool.as_deref(), Some("add_calendar_event"));
        assert_eq!(
            event.required_arguments,
            json!({"title":"QA Training session","start":"2031-05-21 14:15","minutes":35,"location":"Training Room"})
        );
        assert!(contract.validate_proposal("add_calendar_event", &json!({"title":"QA Training session","start":"2031-05-21 15:15","minutes":35,"location":"Training Room"}), &[]).is_err());
        // Without a year nothing is bound, so the usual date handling applies.
        let goal = "Add a calendar event from 14:15 to 14:50 on May 21 titled QA Training.";
        let mut contract = TaskContract::from_request(goal);
        contract.refresh(goal, &[]);
        assert!(contract.next_outcome().unwrap().required_arguments.get("start").is_none());
    }

    #[test]
    fn a_reminder_is_due_the_lead_before_the_time_the_request_gives() {
        let due = |goal: &str| {
            let mut contract = TaskContract::from_request(goal);
            contract.refresh(goal, &[]);
            contract.outcomes.iter().find(|o| o.tool.as_deref() == Some("add_reminder")).and_then(|o| o.required_arguments.get("due").cloned())
        };
        assert_eq!(
            due("My library books must be returned by 18:00 on May 21, 2031. Set a reminder fifteen minutes beforehand, using exactly the title \"QA Return library books\"."),
            Some(json!("2031-05-21 17:45"))
        );
        assert_eq!(due("Remind me to call Ana at 09:30 on May 21, 2031."), Some(json!("2031-05-21 09:30")));
        // A lead in days isn't read here, and no year means no binding: the time stays open.
        assert_eq!(due("My exam is at 09:00 on May 21, 2031. Remind me two days before."), None);
        assert_eq!(due("Remind me at 09:30 on May 21 to call Ana."), None);
    }

    #[test]
    fn what_the_request_says_about_an_event_outranks_the_note() {
        let goal = "Read my note mentioning QA Workshop details, then add that workshop to my calendar tomorrow at 9:00 for 30 minutes.";
        let mut contract = TaskContract::from_request(goal);
        contract.refresh(goal, &[]);
        let read = note_read("read", "QA Workshop details", "QA Workshop details", "The workshop is on May 20, 2031. It starts at 13:20 and ends at 14:05 in Study Room.");
        contract.refresh(goal, std::slice::from_ref(&read));
        let event = contract.next_outcome().unwrap();
        assert!(event.required_arguments.get("start").is_none());
        assert!(event.required_arguments.get("minutes").is_none());
        assert_eq!(event.required_arguments["location"], "Study Room");
    }
}

#[cfg(test)]
#[path = "task_contract_eval_tests.rs"]
mod eval_tests;

