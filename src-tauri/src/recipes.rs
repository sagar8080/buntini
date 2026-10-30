//! Recipes: everyday workflows whose structure Rust knows, so the model only fills what
//! it must. A recipe can fix fields of a requested write before planning starts
//! (`prepare`), bind a whole next step from the user's words or typed evidence (`bind`),
//! and finish from verified evidence without the model (`answer`). Everything a recipe
//! produces still goes through the task contract, approval and receipt path.
//!
//! Recipes run in registry order and the first to act wins, so a narrower recipe goes
//! before a broader one. Each hook returns `Ok(None)` (or `false`) when it doesn't apply;
//! an error means the request matched but cannot be completed safely.
use crate::{
    edit_targets::EditPlan,
    focus::Referent,
    planning::BoundAction,
    task_contract::{Outcome, TaskContract},
    workflows::Step,
};
use serde_json::{json, Value};

/// What a recipe may look at: the saved goal and journal, and the tools on offer now.
pub struct Plan<'a> {
    pub goal: &'a str,
    pub steps: &'a [Step],
    pub offered: &'a Value,
}

impl Plan<'_> {
    fn offers(&self, tool: &str) -> bool {
        self.offered
            .as_array()
            .is_some_and(|tools| tools.iter().any(|definition| definition["name"] == tool))
    }
}

pub trait Recipe: Sync {
    fn id(&self) -> &'static str;
    /// Fix fields of a requested write from what "this" was, before planning starts.
    fn prepare(&self, _goal: &str, _referent: &Referent, _contract: &mut TaskContract) -> bool {
        false
    }
    /// Arguments for the next unfinished outcome, bound without the model.
    fn bind(&self, _plan: &Plan, _next: &Outcome) -> Result<Option<BoundAction>, String> {
        Ok(None)
    }
    /// A complete answer from verified evidence, without the model.
    fn answer(&self, _plan: &Plan) -> Result<Option<String>, String> {
        Ok(None)
    }
}

/// "What is my first calendar event on 2026-11-02?"
struct FirstCalendarEvent;
impl Recipe for FirstCalendarEvent {
    fn id(&self) -> &'static str {
        "first_calendar_event"
    }
    fn answer(&self, plan: &Plan) -> Result<Option<String>, String> {
        crate::planning::calendar_answer(plan.goal, plan.steps)
    }
}

/// "Create a note titled X with body Y"
struct LiteralNote;
impl Recipe for LiteralNote {
    fn id(&self) -> &'static str {
        "literal_note"
    }
    fn bind(&self, plan: &Plan, next: &Outcome) -> Result<Option<BoundAction>, String> {
        if next.tool.as_deref() != Some("create_note") || !plan.offers("create_note") {
            return Ok(None);
        }
        crate::explicit_fields::explicit_note(plan.goal, plan.steps)
    }
}

/// "Add a reminder titled X due 2026-11-02 18:00"
struct LiteralReminder;
impl Recipe for LiteralReminder {
    fn id(&self) -> &'static str {
        "literal_reminder"
    }
    fn bind(&self, plan: &Plan, next: &Outcome) -> Result<Option<BoundAction>, String> {
        if next.tool.as_deref() != Some("add_reminder") || !plan.offers("add_reminder") {
            return Ok(None);
        }
        crate::explicit_fields::explicit_reminder(plan.goal, plan.steps)
    }
}

/// "Remind me 30 minutes before my first event on 2026-11-02"
struct EventPreparation;
impl Recipe for EventPreparation {
    fn id(&self) -> &'static str {
        "event_preparation"
    }
    fn bind(&self, plan: &Plan, next: &Outcome) -> Result<Option<BoundAction>, String> {
        if next.tool.as_deref() != Some("add_reminder") || !plan.offers("add_reminder") {
            return Ok(None);
        }
        crate::planning::calendar_reminder(plan.goal, plan.steps)
    }
}

/// "Remind me about this tomorrow at 9": named after the thing on screen.
struct ReminderAboutThis;
impl Recipe for ReminderAboutThis {
    fn id(&self) -> &'static str {
        "reminder_about_this"
    }
    fn prepare(&self, goal: &str, referent: &Referent, contract: &mut TaskContract) -> bool {
        if !crate::focus::reminds_about_this(goal) {
            return false;
        }
        referent.reminder_title().is_some_and(|title| {
            contract.bind_write_field("add_reminder", "title", Value::String(title), "focus_referent")
        })
    }
}

/// "Reply to this saying I'll be there": a threaded reply to the open email, also when
/// the user selected text inside it. Which email, its sender and its subject come from
/// Mail; the model writes only the body, which the approval sheet shows.
struct ReplyToThis;
impl Recipe for ReplyToThis {
    fn id(&self) -> &'static str {
        "reply_to_this"
    }
    fn prepare(&self, goal: &str, referent: &Referent, contract: &mut TaskContract) -> bool {
        let Some(mail) = referent.mail() else {
            return false;
        };
        replies_on_screen(goal, referent)
            && bind_reply(
                contract,
                [&mail.id, &mail.message_id, &mail.sender, &mail.subject],
                crate::triage::about(&mail.sender, &mail.subject, &mail.received),
                "focus_referent",
            )
    }
}

/// Whether `goal` replies to the one email open in Mail: it says "reply to this" (or
/// "her", "the email"), exactly one email is selected (a conversation selection is every
/// message in it, and the first need not be the one the user reads), and any sender it
/// names ("the email from Ana") is that email's sender.
pub fn replies_on_screen(goal: &str, referent: &Referent) -> bool {
    let Some(mail) = referent.mail() else {
        return false;
    };
    let named_other = match crate::triage::pick(goal) {
        Some(crate::triage::Pick::Sender(name)) => !crate::triage::from_sender(&name, &mail.sender),
        _ => false,
    };
    mail.selected <= 1 && !named_other && crate::focus::replies_to_this(goal)
}

/// What the task says when a reply names the open email but several are selected.
pub const SEVERAL_SELECTED: &str = "More than one email is selected in Mail. Select just the email to answer, then ask again.";

/// What the task says when a reply has no email to answer.
pub const NOTHING_TO_REPLY: &str = "I can’t tell which email to answer. Say who it’s from, like “reply to Ana”, open it in Mail and ask again, or ask “What needs a reply?” and then say which one. For a new email, say “Draft an email to …”.";

/// "Reply to the second one", "reply to Ana's email": the email "What needs a reply?"
/// listed, picked by the user's own words (`triage::pick`). Binds like `ReplyToThis`.
pub fn reply_to_listed(email: &crate::triage::Listed, contract: &mut TaskContract) -> bool {
    bind_reply(
        contract,
        [&email.id, &email.message_id, &email.sender, &email.subject],
        email.describe(),
        "mail_listing",
    )
}

/// How many emails an inbox search for a reply reads (Mail's own cap for `read_mail`).
const SEARCHED: u64 = 8;

/// "Reply to Ana", "reply to Sam's email about the lease": who the email is from and
/// what it is about, in the user's own words. None unless the reply names a sender.
fn searched_for(goal: &str) -> Option<(String, Vec<String>)> {
    let Some(crate::triage::Pick::Sender(name)) = crate::triage::pick(goal) else {
        return None;
    };
    let text = crate::skills::words(goal);
    let (object, true) = crate::focus::reply_object(&text)? else {
        return None;
    };
    const ENDS: &[&str] = &["saying", "telling", "and", "asking", "that", "letting", "thanking", "to", "with", "please", "but", "so", "then", "in"];
    const SKIP: &[&str] = &["the", "my", "a", "an", "our", "his", "her", "their", "your", "on", "of", "at", "for"];
    let topic = object
        .split_once(" about ")
        .map(|(_, about)| {
            about
                .split(' ')
                .take_while(|word| !ENDS.contains(word))
                .filter(|word| !SKIP.contains(word) && !word.is_empty())
                .take(4)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    Some((name, topic))
}

/// The inbox read a reply needs when its email is neither open nor listed: the sender's
/// name the user wrote, as the search. None once that read is in the journal.
pub fn reply_read(goal: &str, steps: &[Step]) -> Option<Value> {
    let (name, _) = searched_for(goal)?;
    searched(steps, &name).is_none().then(|| json!({"query":name,"limit":SEARCHED}))
}

/// The latest verified inbox search for `name` in this task.
fn searched<'a>(steps: &'a [Step], name: &str) -> Option<&'a Step> {
    steps.iter().rev().find(|step| {
        step.tool == "read_mail"
            && !step.effect
            && step.state == "succeeded"
            && step.arguments["query"].as_str().is_some_and(|query| query.eq_ignore_ascii_case(name))
            && step.evidence.as_ref().is_some_and(|evidence| crate::apps::validate_evidence("read_mail", &step.arguments, evidence, false).is_ok())
    })
}

/// The emails that search found from the person the user named (and about what they
/// said, by subject words), newest first, with whether the search saw every match.
/// Only Mail's own ids and headers are used; an email's text decides nothing.
fn found_replies(goal: &str, steps: &[Step]) -> Result<Option<(Vec<crate::triage::Listed>, bool)>, String> {
    let Some((name, topic)) = searched_for(goal) else {
        return Ok(None);
    };
    let Some(step) = searched(steps, &name) else {
        return Ok(None);
    };
    let evidence = step.evidence.as_ref().unwrap();
    let text = |record: &Value, key: &str| record[key].as_str().unwrap_or_default().to_string();
    let found: Vec<crate::triage::Listed> = evidence["records"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|record| !text(record, "id").is_empty() && !text(record, "messageId").is_empty())
        .filter(|record| crate::triage::from_sender(&name, &text(record, "sender")))
        .filter(|record| {
            let subject = crate::skills::words(&text(record, "subject"));
            let subject: Vec<&str> = subject.split(' ').collect();
            topic.iter().all(|word| subject.iter().any(|have| have == word || (word.len() >= 4 && have.starts_with(word.as_str()))))
        })
        .enumerate()
        .map(|(index, record)| crate::triage::Listed {
            number: index + 1,
            id: text(record, "id"),
            message_id: text(record, "messageId"),
            sender: text(record, "sender"),
            subject: text(record, "subject"),
            received: text(record, "receivedLocal"),
        })
        .collect();
    if found.is_empty() {
        // "ana silva" as the user would write it: "Ana Silva".
        let shown = name
            .split(' ')
            .map(|word| {
                let mut letters = word.chars();
                letters.next().map(|first| first.to_uppercase().chain(letters).collect()).unwrap_or_default()
            })
            .collect::<Vec<String>>()
            .join(" ");
        let about = if topic.is_empty() { String::new() } else { format!(" about “{}”", topic.join(" ")) };
        return Err(format!("I couldn’t find an email from {shown}{about} in your inbox from the last four months. Open it in Mail and ask again."));
    }
    Ok(Some((found, evidence["complete"] == true)))
}

/// "Reply to Ana" after the inbox search: when it found exactly one email from her (and
/// saw every match), that email is bound like an open one. Several, or one from a
/// search that stopped short, are offered in the approval sheet (`reply_choices`).
pub fn reply_from_search(goal: &str, steps: &[Step], contract: &mut TaskContract) -> Result<bool, String> {
    if !contract.unbound_reply() {
        return Ok(false);
    }
    Ok(match found_replies(goal, steps)? {
        Some((found, true)) if found.len() == 1 => bind_reply(
            contract,
            [&found[0].id, &found[0].message_id, &found[0].sender, &found[0].subject],
            found[0].describe(),
            "mail_search",
        ),
        _ => false,
    })
}

/// The emails a reply may answer when the search found several: each sets the email,
/// its sender and its subject together, so a pick can't mix two emails.
pub fn reply_choices(goal: &str, steps: &[Step]) -> Result<Vec<crate::actions::Choice>, String> {
    let Some((found, complete)) = found_replies(goal, steps)? else {
        return Ok(vec![]);
    };
    Ok(found
        .into_iter()
        .take(SEARCHED as usize)
        .filter_map(|email| {
            let address = crate::focus::sender_address(&email.sender)?;
            let mut label = format!("{} · {address}", email.describe());
            if !complete {
                label += " · older emails not searched";
            }
            let also = [
                ("message_id", email.message_id.clone()),
                ("to", address),
                ("subject", reply_subject(&email.subject)),
                ("answers", email.describe()),
            ]
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect();
            Some(crate::actions::Choice { field: "message".into(), label, value: email.id, also })
        })
        .collect())
}

/// The proposal a reply's approval is checked against. When the user picked the email
/// in the sheet, everything about it comes from the search, found again by the id they
/// picked; nothing else the sheet sent can choose the email or its recipient.
pub fn picked_reply(tool: &str, goal: &str, steps: &[Step], proposed: &Value, approved: &Value) -> Result<Value, String> {
    if tool != "reply_email" || proposed.get("message").is_some() {
        return Ok(proposed.clone());
    }
    let picked = approved["message"].as_str().unwrap_or_default();
    let choice = reply_choices(goal, steps)?
        .into_iter()
        .find(|choice| !picked.is_empty() && choice.value == picked)
        .ok_or("Pick which email to answer first.")?;
    let mut fixed = proposed.clone();
    fixed["message"] = json!(choice.value);
    for key in ["message_id", "to", "answers"] {
        fixed[key] = json!(choice.also[key]);
    }
    Ok(fixed)
}

/// Binds everything Mail's reply needs from one email (its number, Message-ID, sender
/// and subject) or nothing: a reply that knows its recipient but not its email, or the
/// reverse, could answer the wrong thread.
fn bind_reply(contract: &mut TaskContract, [id, message_id, sender, subject]: [&String; 4], answers: String, source: &str) -> bool {
    let Some(address) = crate::focus::sender_address(sender) else {
        return false;
    };
    let replies: Vec<&Outcome> = contract
        .outcomes
        .iter()
        .filter(|o| o.kind == "write" && o.tool.as_deref() == Some("reply_email"))
        .collect();
    let [reply] = replies[..] else {
        return false;
    };
    if id.is_empty()
        || message_id.is_empty()
        || contract.constraints.iter().any(|c| c.outcome_id.as_deref() == Some(&reply.id))
    {
        return false;
    }
    for (field, value) in [
        ("message", id.clone()),
        ("message_id", message_id.clone()),
        ("to", address),
        ("subject", reply_subject(subject)),
        ("answers", answers),
    ] {
        contract.bind_write_field("reply_email", field, Value::String(value), source);
    }
    true
}

/// "Mark buy milk done", "move my 3pm to 4", "add milk to my shopping note": read where
/// the item lives, then bind the change to the one item the user named. Several matches
/// are bound without an id for the user to pick in the sheet; no match is a question.
struct EditItem;
impl Recipe for EditItem {
    fn id(&self) -> &'static str {
        "edit_item"
    }
    fn bind(&self, plan: &Plan, next: &Outcome) -> Result<Option<BoundAction>, String> {
        let tool = match (next.kind.as_str(), next.tool.as_deref()) {
            ("read", Some(read)) if next.description.starts_with(crate::task_contract::EDIT_READ) => match read {
                // The read's description is the clause it was made for.
                "read_reminders" if crate::completion::requested_actions(&next.description[crate::task_contract::EDIT_READ.len()..]).0.contains(&"reschedule_reminder") => "reschedule_reminder",
                "read_reminders" => "complete_reminder",
                "read_calendar" => "update_calendar_event",
                "find_notes" => "append_to_note",
                _ => return Ok(None),
            },
            ("write", Some(tool)) => match crate::actions::EDIT_TOOLS.iter().find(|edit| **edit == tool) {
                Some(edit) => edit,
                None => return Ok(None),
            },
            _ => return Ok(None),
        };
        if next.kind == "write" && !plan.offers(tool) {
            return Ok(None);
        }
        let goal = crate::edit_targets::outcome_goal(&next.description, plan.goal, tool);
        Ok(match crate::edit_targets::edit_plan(goal, tool, plan.steps)? {
            EditPlan::Read { tool: read, arguments } => {
                if !plan.offers(read) {
                    return Err("I can’t read that app for this request. Check it’s allowed in Settings → Apps.".into());
                }
                Some(BoundAction {
                    tool: read,
                    source: json!({"kind":"user_fields","source":"user_request","fields":arguments}),
                    arguments,
                })
            }
            _ if next.kind == "read" => None,
            EditPlan::One(candidate) => Some(BoundAction {
                tool,
                arguments: candidate.arguments.clone(),
                source: crate::edit_targets::provenance(tool, &candidate),
            }),
            EditPlan::Several(candidates) => Some(BoundAction {
                tool,
                arguments: crate::edit_targets::unpicked(&candidates),
                source: json!({"kind":"source_record","stepId":candidates[0].step_id,"pick":"user"}),
            }),
        })
    }
}

/// "Lease renewal" as "Re: Lease renewal", leaving an existing "Re:" alone.
fn reply_subject(subject: &str) -> String {
    let subject = subject.trim();
    if subject.to_lowercase().starts_with("re:") {
        subject.to_string()
    } else if subject.is_empty() {
        "Re:".into()
    } else {
        format!("Re: {subject}")
    }
}

static RECIPES: &[&dyn Recipe] = &[
    &EditItem,
    &FirstCalendarEvent,
    &LiteralNote,
    &LiteralReminder,
    &EventPreparation,
    &ReminderAboutThis,
    &ReplyToThis,
];

/// Lets every applicable recipe fix fields from what "this" was. Returns the ids that did.
pub fn prepare(goal: &str, referent: &Referent, contract: &mut TaskContract) -> Vec<&'static str> {
    RECIPES
        .iter()
        .filter(|recipe| recipe.prepare(goal, referent, contract))
        .map(|recipe| recipe.id())
        .collect()
}

/// The first recipe's binding for the next outcome.
pub fn bind(plan: &Plan, next: &Outcome) -> Result<Option<BoundAction>, String> {
    for recipe in RECIPES {
        if let Some(bound) = recipe.bind(plan, next)? {
            return Ok(Some(bound));
        }
    }
    Ok(None)
}

/// The first recipe's finished answer.
pub fn answer(plan: &Plan) -> Result<Option<String>, String> {
    for recipe in RECIPES {
        if let Some(answer) = recipe.answer(plan)? {
            return Ok(Some(answer));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn mail(sender: &str, subject: &str) -> Referent {
        Referent::Mail(crate::apps::SelectedMail {
            id: "7".into(),
            message_id: "<m@x>".into(),
            received: "2026-10-29 09:15".into(),
            sender: sender.into(),
            subject: subject.into(),
            text: "Are you free on Friday?".into(),
            truncated: false,
            selected: 1,
        })
    }

    #[test]
    fn registry_ids_are_unique() {
        let mut ids: Vec<&str> = RECIPES.iter().map(|r| r.id()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), RECIPES.len());
    }

    #[test]
    fn a_reply_to_the_open_email_fixes_its_recipient_and_subject() {
        let goal = "Reply to this saying I'll be there";
        let mut contract = TaskContract::from_request(goal);
        contract.refresh(goal, &[]);
        let applied = prepare(goal, &mail("Ana Silva <ana@example.com>", "Friday"), &mut contract);
        assert_eq!(applied, ["reply_to_this"]);
        contract.refresh(goal, &[]);
        let next = contract.next_outcome().unwrap();
        assert_eq!(next.tool.as_deref(), Some("reply_email"));
        assert_eq!(next.required_arguments["message"], "7");
        assert_eq!(next.required_arguments["message_id"], "<m@x>");
        assert_eq!(next.required_arguments["to"], "ana@example.com");
        assert_eq!(next.required_arguments["subject"], "Re: Friday");
        assert_eq!(next.required_arguments["answers"], "Ana Silva · “Friday” · Thu 29 Oct");
        // The model writes the body; everything else is filled from the email.
        let mut proposal = json!({"body":"I'll be there."});
        contract.fill_resolved_fields("reply_email", &mut proposal);
        assert!(contract.validate_proposal("reply_email", &proposal, &[]).is_ok());
        let mut other = proposal.clone();
        other["to"] = json!("eve@example.com");
        assert!(contract.validate_proposal("reply_email", &other, &[]).is_err());
        assert!(contract
            .validate_proposal("draft_email", &json!({"to":"ana@example.com","subject":"Re: Friday","body":"I'll be there."}), &[])
            .is_err(), "a reply is never a new email");
    }

    fn listed(number: usize, sender: &str) -> crate::triage::Listed {
        crate::triage::Listed {
            number,
            id: format!("{number}0"),
            message_id: format!("m-{number}@x.com"),
            sender: sender.into(),
            subject: "Lease".into(),
            received: "2026-10-21 09:00".into(),
        }
    }

    #[test]
    fn a_reply_to_a_listed_email_binds_that_email_whole() {
        let goal = "Reply to the second one saying yes";
        let mut contract = TaskContract::from_request(goal);
        contract.refresh(goal, &[]);
        assert!(contract.unbound_reply());
        assert!(reply_to_listed(&listed(2, "Sam <sam@x.com>"), &mut contract));
        contract.refresh(goal, &[]);
        assert!(!contract.unbound_reply());
        let next = contract.next_outcome().unwrap();
        assert_eq!(next.required_arguments["message"], "20");
        assert_eq!(next.required_arguments["message_id"], "m-2@x.com");
        assert_eq!(next.required_arguments["to"], "sam@x.com");
        assert_eq!(next.required_arguments["subject"], "Re: Lease");
        assert!(contract.constraints.iter().all(|c| c.source == "mail_listing"));
        // Once bound it stays bound: neither a second listed email nor the screen rebinds it.
        assert!(!reply_to_listed(&listed(1, "Ana <ana@x.com>"), &mut contract));
        assert!(prepare("reply to this", &mail("Eve <eve@x.com>", "Other"), &mut contract).is_empty());
        // An email without a Message-ID or a sender address binds nothing at all.
        for email in [
            crate::triage::Listed { message_id: String::new(), ..listed(1, "Ana <ana@x.com>") },
            listed(1, "Ana"),
        ] {
            let mut contract = TaskContract::from_request(goal);
            assert!(!reply_to_listed(&email, &mut contract));
            assert!(contract.constraints.is_empty());
        }
    }

    #[test]
    fn a_reply_to_text_selected_in_an_email_still_knows_the_sender() {
        let goal = "reply to her saying yes";
        let open = mail("Ana Silva <ana@example.com>", "Friday").mail().cloned();
        let selected = Referent::Selection {
            app: "Mail".into(),
            text: "Are you free on Friday?".into(),
            mail: open,
        };
        let mut contract = TaskContract::from_request(goal);
        contract.refresh(goal, &[]);
        assert_eq!(prepare(goal, &selected, &mut contract), ["reply_to_this"]);
        contract.refresh(goal, &[]);
        let next = contract.next_outcome().unwrap();
        assert_eq!(next.required_arguments["to"], "ana@example.com");
        assert_eq!(next.required_arguments["subject"], "Re: Friday");
        // A selection elsewhere has no sender to reply to.
        let elsewhere = Referent::Selection { app: "Notes".into(), text: "Friday?".into(), mail: None };
        let mut contract = TaskContract::from_request(goal);
        assert!(prepare(goal, &elsewhere, &mut contract).is_empty());
    }

    #[test]
    fn replies_need_an_email_on_screen_and_a_reply_request() {
        let goal = "Reply to this saying yes";
        let mut contract = TaskContract::from_request(goal);
        let page = Referent::Page { browser: "safari", title: "Docs".into() };
        assert!(prepare(goal, &page, &mut contract).is_empty());
        let other = "Draft an email to Ana about Friday";
        let mut contract = TaskContract::from_request(other);
        assert!(prepare(other, &mail("ana@example.com", "Friday"), &mut contract).is_empty());
        // A sender without an address can't be replied to.
        let mut contract = TaskContract::from_request(goal);
        assert!(prepare(goal, &mail("Ana Silva", "Friday"), &mut contract).is_empty());
    }

    #[test]
    fn a_reply_binds_the_open_email_only_when_it_is_the_one_meant() {
        let open = mail("Sam Lee <sam@example.com>", "Friday");
        // The user named someone else: the open email isn't the one to answer.
        let goal = "reply to the email from Ana saying this works";
        let mut contract = TaskContract::from_request(goal);
        assert!(prepare(goal, &open, &mut contract).is_empty());
        assert!(contract.unbound_reply());
        let goal = "reply to the email from Sam saying this works";
        let mut contract = TaskContract::from_request(goal);
        assert_eq!(prepare(goal, &open, &mut contract), ["reply_to_this"]);
        // A whole conversation selected: which message is meant isn't known.
        let Referent::Mail(mut several) = open else { unreachable!() };
        several.selected = 3;
        let goal = "reply to this saying yes";
        let mut contract = TaskContract::from_request(goal);
        assert!(prepare(goal, &Referent::Mail(several), &mut contract).is_empty());
        // A page in front doesn't answer "the second one", so the saved list does.
        let page = Referent::Page { browser: "safari", title: "Docs".into() };
        assert!(!replies_on_screen("reply to the second one saying this works for me", &page));
        // The noun form binds like the verb form.
        let goal = "draft a reply to this email saying I'll be there";
        let mut contract = TaskContract::from_request(goal);
        assert_eq!(prepare(goal, &mail("Ana <ana@example.com>", "Friday"), &mut contract), ["reply_to_this"]);
    }

    #[test]
    fn reply_subjects_are_not_doubled() {
        assert_eq!(reply_subject("Lunch"), "Re: Lunch");
        assert_eq!(reply_subject("RE: Lunch"), "RE: Lunch");
        assert_eq!(reply_subject("  "), "Re:");
    }

    fn search(query: &str, rows: &[[&str; 5]], total: usize) -> Step {
        Step {
            id: "search".into(),
            tool: "read_mail".into(),
            arguments: json!({"query":query,"limit":8}),
            state: "succeeded".into(),
            effect: false,
            proposed_at: String::new(),
            approved_at: None,
            finished_at: None,
            evidence: Some(crate::apps::mail_fixture(query, rows, total)),
        }
    }

    const ANA_LEASE: [&str; 5] = ["2026-10-28 09:00", "<lease@x>", "Ana Silva <ana@example.com>", "Lease renewal", "4821"];
    const ANA_DINNER: [&str; 5] = ["2026-10-27 18:00", "<dinner@x>", "Ana Silva <ana@example.com>", "Dinner on Friday?", "4790"];
    const BANANA: [&str; 5] = ["2026-10-26 08:00", "<fruit@x>", "Shop <news@shop.example>", "Banana deals", "4700"];

    #[test]
    fn a_reply_to_someone_named_searches_the_inbox_for_them_first() {
        assert_eq!(searched_for("Reply to Ana saying yes"), Some(("ana".into(), vec![])));
        assert_eq!(searched_for("reply to Sam's email about the lease saying I'll sign"), Some(("sam".into(), vec!["lease".into()])));
        assert_eq!(searched_for("reply to the email from Ana Silva about dinner on Friday"), Some(("ana silva".into(), vec!["dinner".into(), "friday".into()])));
        for goal in ["Reply to this saying yes", "reply to the second one", "Draft an email to Ana"] {
            assert_eq!(searched_for(goal), None, "{goal}");
        }
        let goal = "Reply to Ana saying yes";
        let mut contract = TaskContract::from_request(goal);
        assert_eq!(contract.reply_lookup(&[]), Some(json!({"query":"ana","limit":8})));
        // Only that search, by the name the user wrote, is a read the reply may make.
        assert!(contract.validate_proposal("read_mail", &json!({"query":"ana","limit":8}), &[]).is_ok());
        assert!(contract.validate_proposal("read_mail", &json!({"query":"lease","limit":8}), &[]).is_err());
        let tools = contract.relevant_tools(&[], &crate::tools::offer(&["read_mail", "reply_email"]));
        let read = tools.as_array().unwrap().iter().find(|tool| tool["name"] == "read_mail").unwrap();
        assert_eq!(read["parameters"]["properties"]["query"]["enum"], json!(["ana"]));
        // The one email from her is bound like an open one, from Mail's own ids.
        let steps = [search("ana", &[ANA_LEASE, BANANA], 2)];
        assert_eq!(contract.reply_lookup(&steps), None);
        assert!(reply_from_search(goal, &steps, &mut contract).unwrap());
        assert!(!contract.unbound_reply());
        contract.refresh(goal, &steps);
        let next = contract.next_outcome().unwrap();
        assert_eq!(next.required_arguments["message"], "4821");
        assert_eq!(next.required_arguments["message_id"], "<lease@x>");
        assert_eq!(next.required_arguments["to"], "ana@example.com");
        assert_eq!(next.required_arguments["subject"], "Re: Lease renewal");
        let mut proposal = json!({"body":"Yes"});
        contract.fill_resolved_fields("reply_email", &mut proposal);
        assert!(contract.validate_proposal("reply_email", &proposal, &steps).is_ok());
        assert!(contract.constraints.iter().all(|c| c.source == "mail_search"));
    }

    #[test]
    fn several_emails_from_them_are_offered_and_the_pick_decides_everything_about_the_email() {
        let goal = "Reply to Ana saying yes";
        let steps = [search("ana", &[ANA_LEASE, ANA_DINNER, BANANA], 3)];
        let mut contract = TaskContract::from_request(goal);
        assert!(!reply_from_search(goal, &steps, &mut contract).unwrap());
        assert!(contract.unbound_reply());
        let choices = reply_choices(goal, &steps).unwrap();
        assert_eq!(choices.iter().map(|c| c.value.as_str()).collect::<Vec<_>>(), ["4821", "4790"]);
        assert!(choices.iter().all(|c| c.field == "message" && c.label.contains("ana@example.com")));
        assert_eq!(choices[1].also["subject"], "Re: Dinner on Friday?");
        assert_eq!(choices[1].also["message_id"], "<dinner@x>");
        // What they said it was about narrows it to one.
        let about = "reply to Ana's email about dinner saying yes";
        let mut narrowed = TaskContract::from_request(about);
        assert!(reply_from_search(about, &steps, &mut narrowed).unwrap());
        assert_eq!(narrowed.constraints.iter().find(|c| c.field == "message").unwrap().value, "4790");
        // On approval the picked id finds the email again; the sheet's recipient is ignored.
        let proposed = json!({"body":"Yes"});
        let sheet = json!({"message":"4790","message_id":"<lease@x>","to":"eve@example.com","subject":"Re: Dinner","body":"Yes"});
        let fixed = picked_reply("reply_email", goal, &steps, &proposed, &sheet).unwrap();
        assert_eq!((fixed["message"].as_str(), fixed["message_id"].as_str(), fixed["to"].as_str()), (Some("4790"), Some("<dinner@x>"), Some("ana@example.com")));
        let kept = crate::actions::keep_fixed("reply_email", &fixed, &sheet);
        assert_eq!((kept["to"].as_str(), kept["subject"].as_str()), (Some("ana@example.com"), Some("Re: Dinner")));
        for picked in [json!({"body":"Yes"}), json!({"message":"4700","body":"Yes"}), json!({"message":"9999","body":"Yes"})] {
            assert!(picked_reply("reply_email", goal, &steps, &proposed, &picked).is_err(), "{picked}");
        }
        // A bound reply is checked against what was bound, as before.
        let bound = json!({"message":"4821","body":"Yes"});
        assert_eq!(picked_reply("reply_email", goal, &steps, &bound, &sheet).unwrap(), bound);
    }

    #[test]
    fn a_search_that_stopped_short_or_found_nobody_never_guesses() {
        let goal = "Reply to Ana saying yes";
        // Eight of twenty matches read: another email from Ana may be one not seen.
        let partial = [search("ana", &[ANA_LEASE, BANANA], 20)];
        let mut contract = TaskContract::from_request(goal);
        assert!(!reply_from_search(goal, &partial, &mut contract).unwrap());
        let choices = reply_choices(goal, &partial).unwrap();
        assert_eq!(choices.len(), 1);
        assert!(choices[0].label.ends_with("older emails not searched"));
        // A subject or text that mentions her is not an email from her, and neither is
        // an address whose domain has her name in it.
        let domain = ["2026-10-26 09:00", "<deal@x>", "Deals <offers@ana.shop>", "This week", "4701"];
        let mut contract = TaskContract::from_request(goal);
        assert!(reply_from_search(goal, &[search("ana", &[domain], 1)], &mut contract).is_err());
        assert!(contract.constraints.is_empty(), "no recipient is chosen from a domain");
        let nobody = [search("ana", &[BANANA], 1)];
        let mut contract = TaskContract::from_request(goal);
        let error = reply_from_search(goal, &nobody, &mut contract).unwrap_err();
        assert!(error.contains("couldn’t find an email from Ana"), "{error}");
        assert!(contract.constraints.is_empty());
        let about = "reply to Ana about the boiler";
        let error = reply_choices(about, &[search("ana", &[ANA_LEASE], 1)]).unwrap_err();
        assert!(error.contains("from Ana about “boiler”"), "{error}");
    }
}
