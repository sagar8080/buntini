//! Questions about the user's own Mail, Calendar and Reminders, put to the real Apple model
//! over a made-up inbox, calendar and reminders whose right answers are known. Reads only.
//! `BUNTINI_PERSONAL_REPORT=<file> cargo test --lib live_personal -- --ignored --nocapture`
use super::apple_live_tests::live_state;
use super::*;
use crate::apps::{calendar_evidence, calendar_range_evidence, inbox_world, reminder_evidence, Agenda, Event, Reminder};
use chrono::Duration;
use std::io::Write;

struct Question {
    asked: &'static str,
    /// The reads it must make. Empty: it must read nothing.
    reads: &'static [&'static str],
    /// Each must be in the answer.
    says: &'static [&'static str],
    /// None may be in the answer.
    never: &'static [&'static str],
}

const fn q(asked: &'static str, reads: &'static [&'static str], says: &'static [&'static str], never: &'static [&'static str]) -> Question {
    Question { asked, reads, says, never }
}

const MAIL: &[&str] = &["read_mail"];
const DAY: &[&str] = &["read_calendar"];
const TODO: &[&str] = &["read_reminders"];
const NONE: &[&str] = &[];

const QUESTIONS: &[Question] = &[
    // The words that started this, and others for the same thing: the whole inbox.
    q("whats on my mail", MAIL, &["Priya", "12 emails"], &["boss"]),
    q("What’s in my inbox?", MAIL, &["Priya", "12 emails"], &["boss"]),
    q("any new emails?", MAIL, &["Priya", "unread"], &["boss"]),
    q("hows my inbox", MAIL, &["Priya", "12 emails"], &["boss"]),
    q("got mail?", MAIL, &["Priya", "12 emails"], &["boss"]),
    q("how many emails do I have", MAIL, &["12 emails"], &["boss"]),
    q("how many unread emails do i have", MAIL, &["3 unread"], &["boss"]),
    q("summarize my inbox", MAIL, &["Priya", "12 emails"], &["boss"]),
    // Some of the mail, including questions that never say "mail".
    q("anything urgent in my mail", MAIL, &["Priya"], &["boss"]),
    q("anything from Apple in my inbox", MAIL, &["receipt"], &["LinkedIn"]),
    q("did anyone email me about an invoice", MAIL, &["4821"], &["LinkedIn"]),
    q("who emailed me today", MAIL, &["Priya", "Apple", "GitHub"], &["boss", "Lisbon"]),
    q("did Ana write back?", MAIL, &["Lisbon"], &["boss"]),
    q("what did my last email say", MAIL, &["Priya"], &["boss"]),
    q("is the invoice from acme paid", MAIL, &["4821"], &["boss"]),
    // Calendar.
    q("what's on my calendar today", DAY, &["Design review"], &["Dentist"]),
    q("whats on my schedule tomorrow", DAY, &["Dentist"], &["Design review"]),
    q("what do I have on Saturday", DAY, &["Climbing"], &["Dentist"]),
    q("whats on my calendar", DAY, &["Design review"], &[]),
    q("am I free tomorrow at 3pm", DAY, &["Dentist"], &[]),
    q("am I free tomorrow at 9am", DAY, &["Nothing on your calendar at 09:00"], &[]),
    q("when is my next meeting", DAY, &["Design review"], &[]),
    q("do I have any meetings this week", DAY, &["Design review", "Dentist"], &[]),
    q("anything on this weekend", DAY, &["Climbing"], &[]),
    q("when's my dentist appointment", DAY, &["Dentist"], &[]),
    q("who am I meeting tomorrow", DAY, &["Priya"], &[]),
    // Reminders.
    q("what are my reminders", TODO, &["Call mum", "Buy milk"], &[]),
    q("whats due today", TODO, &["insurance"], &[]),
    q("any reminders about the car", TODO, &["insurance"], &["Buy milk"]),
    q("what do I need to do today", TODO, &["insurance"], &[]),
    q("how many reminders do I have", TODO, &["3 open reminders"], &[]),
    // More than one source.
    q("what's on my calendar today and do I have new email", &["read_calendar", "read_mail"], &["Design review", "Priya"], &["boss"]),
    // Not about the user's own data: nothing may be read or claimed.
    q("how do I add an email account on a Mac", NONE, &[], &["Priya"]),
    q("what is an inbox", NONE, &[], &["Priya"]),
    q("hey buntini", NONE, &[], &["Priya", "Dentist"]),
    q("tell me a joke about email", NONE, &[], &["Priya"]),
    q("what's the best calendar app", NONE, &[], &["Dentist"]),
    q("what is the capital of France", NONE, &["Paris"], &["Priya", "Dentist"]),
    q("what's the weather tomorrow", NONE, &[], &["Dentist"]),
    q("thanks", NONE, &[], &["Priya", "Dentist"]),
    q("why do people hate meetings", NONE, &[], &["Dentist"]),
    q("what's 2 + 2", NONE, &["4"], &["Priya", "Dentist"]),
    // Never tuned against: other ways to ask, kept apart so the score means something.
    q("has the bank emailed", MAIL, &["matches “bank”"], &["boss", "Priya"]),
    q("anything new from Priya", MAIL, &["Priya"], &["LinkedIn"]),
    q("any mail from LinkedIn", MAIL, &["LinkedIn"], &["Priya"]),
    q("was there an email about my profile", MAIL, &["LinkedIn"], &["Uber"]),
    q("did Sam ask about lunch", MAIL, &["Lunch"], &["Priya"]),
    q("what's happening tomorrow", DAY, &["Dentist"], &["Design review"]),
    q("what's my first meeting tomorrow", DAY, &["Priya"], &[]),
    q("when do I see Sam", DAY, &["Climbing"], &[]),
    q("do I have plans on Saturday", DAY, &["Climbing"], &[]),
    q("is anything overdue", TODO, &[], &["Dentist"]),
    q("what should I get done today", TODO, &["insurance"], &[]),
    q("do I need to call anyone", TODO, &["Call mum"], &[]),
    q("what's left on my to-do list", TODO, &["Buy milk"], &[]),
    q("what's a good way to organise email", NONE, &[], &["Priya"]),
    q("who invented email", NONE, &[], &["Priya"]),
    q("what does RSVP mean", NONE, &[], &["Dentist"]),
    q("how long should meetings be", NONE, &[], &["Dentist"]),
    q("tell me about Lisbon", NONE, &[], &["Ana", "Priya"]),
    q("good morning", NONE, &[], &["Priya", "Dentist"]),
    // A second untuned set, written after the first was fixed.
    q("has Priya sent anything", MAIL, &["Priya"], &["LinkedIn"]),
    q("any emails from Uber", MAIL, &["Uber"], &["Priya"]),
    q("did GitHub email me", MAIL, &["GitHub"], &["Priya"]),
    q("what's Ana saying about the flights", MAIL, &["Lisbon"], &["Priya"]),
    q("do I owe anyone money", MAIL, &[], &["boss"]),
    q("did my Apple receipt come through", MAIL, &["receipt"], &["LinkedIn"]),
    q("any important emails", MAIL, &["Priya"], &["boss"]),
    q("how much was the acme invoice", MAIL, &["1,240"], &["boss"]),
    q("is my afternoon clear tomorrow", DAY, &["Dentist"], &[]),
    q("what time is my design review", DAY, &["23:00"], &[]),
    q("where is my dentist", DAY, &["Harley"], &[]),
    q("am I doing anything Saturday", DAY, &["Climbing"], &[]),
    q("what's on tomorrow", DAY, &["Dentist"], &[]),
    q("what meetings do I have", DAY, &["Design review"], &[]),
    q("when am I climbing", DAY, &["Climbing"], &[]),
    q("am I busy tonight", DAY, &["Design review"], &[]),
    q("what's on my plate today", TODO, &["insurance"], &[]),
    q("did I forget anything", TODO, &[], &["boss"]),
    q("anything I need to buy", TODO, &["milk"], &[]),
    q("is the insurance reminder still open", TODO, &["insurance"], &[]),
    q("tasks for tomorrow", TODO, &["Call mum"], &["insurance"]),
    q("what's the time in Tokyo", NONE, &[], &["Priya", "Dentist"]),
    q("how are you", NONE, &[], &["Priya", "Dentist"]),
    q("recommend a good book", NONE, &[], &["Priya", "Dentist"]),
    q("what do dentists do", NONE, &[], &["Harley"]),
    q("give me a tip for writing shorter emails", NONE, &[], &["Priya"]),
    q("is email secure", NONE, &[], &["Priya"]),
    q("who is Ana de Armas", NONE, &[], &["Lisbon"]),
    // A third untuned set: its first run is the score reported, whatever it is.
    q("has anyone emailed me about the contract", MAIL, &["Priya"], &["LinkedIn"]),
    q("anything from Sam", MAIL, &["Lunch"], &["Priya"]),
    q("what's the latest from Ana", MAIL, &["Lisbon"], &["Priya"]),
    q("did the Uber receipt arrive", MAIL, &["Uber"], &["Priya"]),
    q("when is the acme invoice due", MAIL, &["20 November"], &["LinkedIn"]),
    q("who sent me something yesterday", MAIL, &["Spotify"], &["Priya"]),
    q("do I have unread mail", MAIL, &["3 unread"], &["boss"]),
    q("any emails I haven't read", MAIL, &["Priya"], &["boss"]),
    q("show me emails from Duolingo", MAIL, &["Duolingo"], &["Priya"]),
    q("is there a message from my bank", MAIL, &["matches “bank”"], &["Priya"]),
    q("what's my schedule like on Friday", DAY, &["Dentist"], &[]),
    q("do I have anything on at 11 tomorrow", DAY, &["1:1"], &[]),
    q("where's my meeting tonight", DAY, &["Room 4"], &[]),
    q("what's next on my calendar", DAY, &["Design review"], &[]),
    q("when is climbing", DAY, &["Climbing"], &[]),
    q("am I free this afternoon", DAY, &["Design review"], &[]),
    q("anything tomorrow morning", DAY, &["Priya"], &[]),
    q("what's on my to do list", TODO, &["Call mum", "Buy milk"], &[]),
    q("what do I have to do tomorrow", TODO, &["Call mum"], &[]),
    q("anything due tomorrow", TODO, &["Call mum"], &["insurance"]),
    q("have I got any tasks", TODO, &["Buy milk"], &[]),
    q("what is a calendar invite", NONE, &[], &["Dentist"]),
    q("how many days are in a week", NONE, &["7"], &["Dentist"]),
    q("what should I cook tonight", NONE, &[], &["Dentist", "Design review"]),
    q("what's the difference between cc and bcc", NONE, &[], &["Priya"]),
    q("is it rude to reply all", NONE, &[], &["Priya"]),
    q("what time zone is London in", NONE, &[], &["Priya"]),
    q("who is my boss", NONE, &[], &["Priya"]),
    // A fourth untuned set: its first run is the score reported, whatever it is.
    q("has Sam replied", MAIL, &["Sam"], &["Priya"]),
    q("anything from legal", MAIL, &["Priya"], &["LinkedIn"]),
    q("what did Priya want", MAIL, &["signature"], &["LinkedIn"]),
    q("any emails about Lisbon", MAIL, &["Lisbon"], &["Priya"]),
    q("did I get a receipt from Uber", MAIL, &["Uber"], &["Priya"]),
    q("how much was my icloud charge", MAIL, &["2.99"], &["LinkedIn"]),
    q("did anyone message me yesterday", MAIL, &["Spotify"], &["Priya"]),
    q("show me unread mail", MAIL, &["3 unread"], &["boss"]),
    q("what's the deal with the contract", MAIL, &["Priya"], &["LinkedIn"]),
    q("anything from my landlord", MAIL, &["matches “landlord”"], &["Priya"]),
    q("what's on Saturday", DAY, &["Climbing"], &[]),
    q("do I have time for lunch tomorrow", DAY, &["Dentist"], &[]),
    q("when's my review", DAY, &["Design review"], &[]),
    q("is the dentist tomorrow", DAY, &["Dentist"], &[]),
    q("where am I going on Saturday", DAY, &["The Arch"], &[]),
    q("how's my week looking", DAY, &["Design review", "Dentist"], &[]),
    q("what have I got tomorrow", DAY, &["Dentist"], &[]),
    q("have I got any meetings today", DAY, &["Design review"], &[]),
    q("do I still need to call my mum", TODO, &["Call mum"], &[]),
    q("what's due this week", TODO, &["insurance", "Call mum"], &["Buy milk"]),
    q("is milk on my list", TODO, &["Buy milk"], &["Call mum"]),
    q("what haven't I done yet", TODO, &["Buy milk"], &[]),
    q("how far is the moon", NONE, &[], &["Priya", "Dentist"]),
    q("what's a good name for a dog", NONE, &[], &["Priya", "Dentist"]),
    q("should I learn Spanish or French", NONE, &[], &["Priya", "Dentist"]),
    q("how do you boil an egg", NONE, &[], &["Priya", "Dentist"]),
    q("who wrote Hamlet", NONE, &[], &["Priya", "Dentist"]),
    q("what day is Christmas this year", NONE, &[], &["Priya", "Dentist"]),
    q("what do you think about remote meetings", NONE, &[], &["Dentist", "Design review"]),
    q("I'm tired", NONE, &[], &["Priya", "Dentist"]),
    // The final untuned set, written before this run and not tuned against: its score is
    // the one to quote.
    q("did Priya send the contract", MAIL, &["signature"], &["LinkedIn"]),
    q("anything from Spotify", MAIL, &["New releases"], &["Priya"]),
    q("any word from Ana", MAIL, &["Lisbon"], &["Priya"]),
    q("what did Sam say about Friday", MAIL, &["ramen"], &["Priya"]),
    q("has Nest sent my report", MAIL, &["Nest"], &["Priya"]),
    q("how much did Uber charge me", MAIL, &["18.40"], &["Priya"]),
    q("is there anything about my streak", MAIL, &["Duolingo"], &["Priya"]),
    q("did anyone sign in to my github", MAIL, &["GitHub"], &["Priya"]),
    q("has my accountant written", MAIL, &["accountant"], &["Priya"]),
    q("what's in my inbox from today", MAIL, &["Priya"], &["boss"]),
    q("when's the design review", DAY, &["23:00"], &[]),
    q("what am I doing tomorrow", DAY, &["Dentist"], &[]),
    q("am I free tomorrow at 11", DAY, &["1:1"], &[]),
    q("what's on Friday", DAY, &["Dentist"], &[]),
    q("am I seeing Priya this week", DAY, &["1:1"], &[]),
    q("where's the climbing", DAY, &["The Arch"], &[]),
    q("do I have a dentist appointment", DAY, &["Dentist"], &[]),
    q("any meetings today", DAY, &["Design review"], &[]),
    q("what do I need to get", TODO, &[], &["Priya", "Dentist"]),
    q("did I remember the insurance", TODO, &["insurance"], &[]),
    q("what are my tasks", TODO, &["Buy milk"], &[]),
    q("is there a reminder to call mum", TODO, &["Call mum"], &["milk"]),
    q("how do I reset my password", NONE, &[], &["Priya", "Dentist"]),
    q("what's the population of Japan", NONE, &[], &["Priya", "Dentist"]),
    q("translate good night into Italian", NONE, &[], &["Priya", "Dentist"]),
    q("what's a healthy breakfast", NONE, &[], &["Priya", "Dentist"]),
    q("why is the sky blue", NONE, &[], &["Priya", "Dentist"]),
    q("sing me a song", NONE, &[], &["Priya", "Dentist"]),
    q("I just got back from the gym", NONE, &[], &["Priya", "Dentist"]),
    q("how are meetings usually run in Japan", NONE, &[], &["Priya", "Dentist"]),
    // Written after the rules above were last changed, to measure them, and not tuned on.
    q("did Duolingo write", MAIL, &["streak"], &["Priya"]),
    q("any mail from Acme", MAIL, &["4821"], &["Priya"]),
    q("did the Spotify email come", MAIL, &["releases"], &["Priya"]),
    q("has Ana answered about the flights", MAIL, &["Lisbon"], &["Priya"]),
    q("what's the news from Priya", MAIL, &["Contract"], &["LinkedIn"]),
    q("did my dad email", MAIL, &["matches “dad”"], &["Priya"]),
    q("was there anything about energy use", MAIL, &["Nest"], &["Priya"]),
    q("what's on tomorrow afternoon", DAY, &["Dentist"], &[]),
    q("am I doing anything tonight", DAY, &["Design review"], &[]),
    q("what's my day like tomorrow", DAY, &["Dentist"], &[]),
    q("any plans this Saturday", DAY, &["Climbing"], &[]),
    q("when am I seeing Priya", DAY, &["1:1"], &[]),
    q("what's at 11 tomorrow", DAY, &["1:1"], &[]),
    q("do I need to renew anything", TODO, &["insurance"], &[]),
    q("what's on my list", TODO, &["Buy milk"], &[]),
    q("what's due tomorrow", TODO, &["Call mum"], &["insurance"]),
    q("is buying milk on my to do list", TODO, &["Buy milk"], &["Call mum"]),
    q("what is the speed of light", NONE, &[], &["Priya", "Dentist"]),
    q("I need a holiday", NONE, &[], &["Priya", "Dentist"]),
    q("tell me a fun fact", NONE, &[], &["Priya", "Dentist"]),
    q("how should I prepare for an interview", NONE, &[], &["Priya", "Dentist"]),
    q("what should I name my cat", NONE, &[], &["Priya", "Dentist"]),
    q("explain how email works", NONE, &[], &["Priya"]),
    // Shopping, which went to Mail on a real Mac. It's about shops, never the user's apps.
    q("Nikon Z8 camera deals", NONE, &[], &["Priya", "Dentist", "matches"]),
    q("nikon z8 camera deals", NONE, &[], &["Priya", "Dentist", "matches"]),
    q("best price for airpods pro", NONE, &[], &["Priya", "Dentist", "matches"]),
    q("cheapest mechanical keyboard under $120", NONE, &[], &["Priya", "Dentist", "matches"]),
    q("sony wh-1000xm5 reviews", NONE, &[], &["Priya", "Dentist", "matches"]),
    q("where can I buy a nikon z8", NONE, &[], &["Priya", "Dentist", "matches"]),
    q("is the nikon z8 worth it", NONE, &[], &["Priya", "Dentist", "matches"]),
    q("black friday tv deals", NONE, &[], &["Priya", "Dentist", "matches"]),
    q("I asked you to search for nikon z8 deals", NONE, &[], &["Priya", "Dentist", "matches"]),
    q("are you sure nikon z8 for $1499?", NONE, &[], &["Priya", "Dentist", "matches"]),
    q("search for Nikon Z8 camera deals every hour", NONE, &[], &["Priya", "Dentist", "matches"]),
    q("search for Apple AirPods Pro 2 at Walmart every hour", NONE, &[], &["Priya", "Dentist", "matches"]),
    q("search for mechanical keyboards under $120 every hour", NONE, &[], &["Priya", "Dentist", "matches"]),
    // Carts go to the shop, never to the user's own apps; buying is never done for them.
    q("add AirPods 4 to my Target cart", NONE, &[], &["Priya", "Dentist", "matches"]),
    q("add this to my cart", NONE, &[], &["Priya", "Dentist", "matches"]),
    q("put https://www.keychron.com/products/keychron-q1-max-qmk-via-wireless-custom-mechanical-keyboard in my cart", NONE, &[], &["Priya", "Dentist", "matches"]),
    q("add a mechanical keyboard to my basket at Best Buy", NONE, &[], &["Priya", "Dentist", "matches"]),
    q("add a keyboard to my cart", NONE, &[], &["Priya", "Dentist", "matches"]),
    q("buy me AirPods 4 from Target", NONE, &[], &["Priya", "Dentist", "matches", "purchased", "I bought"]),
    q("add milk to my shopping list", NONE, &[], &["Priya", "Dentist", "matches"]),
];

fn world_inbox(today: &str, yesterday: &str) -> [(String, bool, &'static str, &'static str, &'static str); 12] {
    [
        (format!("{today} 07:10"), false, "Priya Shah <priya@northwind.example>", "Contract needs your signature by 5pm today", "Legal needs the signed contract by 5pm today or the deal slips to next quarter."),
        (format!("{today} 06:30"), false, "Apple <no_reply@email.apple.com>", "Your receipt from Apple", "Thanks for your purchase. iCloud+ with 200 GB, $2.99 a month."),
        (format!("{today} 06:10"), true, "The Morning Brief <news@morning.example>", "Markets open higher", "Stocks rose in early trading as investors weighed new data."),
        (format!("{today} 05:40"), true, "Duolingo <hello@duolingo.example>", "Your streak is about to end", "Don't lose your 12 day streak. It only takes five minutes."),
        (format!("{today} 05:10"), true, "GitHub <noreply@github.example>", "A new sign-in to your account", "We noticed a new sign-in from a Mac in London."),
        (format!("{today} 04:00"), true, "Uber Receipts <uber@uber.example>", "Your Tuesday evening trip", "Thanks for riding. Your total was $18.40."),
        (format!("{yesterday} 23:30"), true, "Spotify <no-reply@spotify.example>", "New releases for you", "Albums and singles picked for you this week."),
        (format!("{yesterday} 22:15"), true, "Nest <no-reply@nest.example>", "Your October home report", "You used 12 percent less energy than last month."),
        (format!("{yesterday} 21:05"), false, "Ana Ruiz <ana@example.com>", "Re: Lisbon flights", "Yes, the 14th works for me. I'll book tomorrow unless you object."),
        (format!("{yesterday} 16:40"), true, "Acme Billing <billing@acme.example>", "Invoice 4821 for October", "Your invoice of $1,240 is due on 20 November."),
        (format!("{yesterday} 09:15"), true, "LinkedIn <news@linkedin.example>", "5 people viewed your profile", "See who has been looking at your profile this week."),
        (format!("{yesterday} 08:00"), true, "Sam Lee <sam@example.com>", "Lunch on Friday?", "Are you around on Friday? The new ramen place opened."),
    ]
}

fn event(day: &str, start: &str, end: &str, title: &str, location: &str) -> Event {
    serde_json::from_value(json!({"start":format!("{day} {start}"),"end":format!("{day} {end}"),"allDay":false,"title":title,"location":location,"calendar":"Work"})).unwrap()
}

fn world_day(day: chrono::NaiveDate, today: chrono::NaiveDate) -> Agenda<Event> {
    use chrono::Datelike;
    let stamp = day.format("%Y-%m-%d").to_string();
    let items = if day == today {
        vec![event(&stamp, "23:00", "23:30", "Design review", "Room 4")]
    } else if day == today + Duration::days(1) {
        vec![event(&stamp, "11:00", "11:30", "1:1 with Priya", ""), event(&stamp, "15:00", "15:45", "Dentist", "Harley Street")]
    } else if day.weekday() == chrono::Weekday::Sat {
        vec![event(&stamp, "10:00", "12:00", "Climbing with Sam", "The Arch")]
    } else {
        vec![]
    };
    let total = items.len();
    Agenda { items, total }
}

fn world_reminders(today: chrono::NaiveDate) -> Agenda<Reminder> {
    let reminder = |title: &str, due: String| -> Reminder { serde_json::from_value(json!({"title":title,"due":due,"list":"Home"})).unwrap() };
    let items = vec![
        reminder("Renew car insurance", today.format("%Y-%m-%d").to_string()),
        reminder("Call mum", (today + Duration::days(1)).format("%Y-%m-%d").to_string()),
        reminder("Buy milk", String::new()),
    ];
    let total = items.len();
    Agenda { items, total }
}

#[tokio::test]
#[ignore = "uses the real Apple model over a made-up inbox, calendar and reminders; run with BUNTINI_PERSONAL_REPORT=<file>"]
async fn live_personal_answers() {
    let path = std::env::var("BUNTINI_PERSONAL_REPORT").expect("BUNTINI_PERSONAL_REPORT");
    let only = std::env::var("BUNTINI_PERSONAL_ONLY").ok();
    let mut out = std::fs::OpenOptions::new().create(true).append(true).open(path).unwrap();
    let today = chrono::Local::now().date_naive();
    let (day, before) = (today.format("%Y-%m-%d").to_string(), (today - Duration::days(1)).format("%Y-%m-%d").to_string());
    let inbox = world_inbox(&day, &before);
    let mut passed = 0;
    let mut asked = 0;
    for question in QUESTIONS {
        if only.as_deref().is_some_and(|only| !only.split('|').any(|one| question.asked == one)) {
            continue;
        }
        asked += 1;
        let dir = tempfile::tempdir().unwrap();
        let state = live_state(dir.path());
        let mut settings = state.store.lock().unwrap().settings().unwrap();
        settings.apps = vec!["mail".into(), "calendar".into(), "reminders".into()];
        state.store.lock().unwrap().save_settings(&settings).unwrap();
        let mut task: TaskRecord = serde_json::from_value(json!({
            "id":uuid::Uuid::new_v4().to_string(), "prompt":question.asked, "answer":"",
            "state":"running", "createdAt":now(), "sources":[], "files":[], "backend":"apple"
        }))
        .unwrap();
        state.store.lock().unwrap().save_task(&task).unwrap();
        let offered = chat_tools(&settings, state.models.locality(&settings), false, true);
        let messages = vec![json!({"role":"system","content":instructions(&state,&settings,true,"")}), json!({"role":"user","content":question.asked})];
        let mut flow = Workflow::new(&task, "auto", &settings, messages, None, &offered);
        let rows: Vec<(&str, bool, &str, &str, &str)> = inbox.iter().map(|row| (row.0.as_str(), row.1, row.2, row.3, row.4)).collect();
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(150),
            continue_workflow_using(&state, &mut task, &Channel::new(|_| Ok(())), Arc::new(AtomicBool::new(false)), &settings, &mut flow, |tool, args| {
                let found = match tool.as_str() {
                    "read_mail" => Ok(inbox_world(args.get("query").and_then(Value::as_str).unwrap_or(""), args.get("unread").and_then(Value::as_bool).unwrap_or(false), args.get("limit").and_then(Value::as_u64).unwrap_or(5) as usize, &rows)),
                    "read_calendar" => {
                        let asked = args.get("day").and_then(Value::as_str).unwrap_or_default();
                        let from = chrono::NaiveDate::parse_from_str(asked, "%Y-%m-%d").unwrap();
                        match args.get("until").and_then(Value::as_str) {
                            None => Ok(calendar_evidence(asked, &world_day(from, today))),
                            Some(until) => {
                                let to = chrono::NaiveDate::parse_from_str(until, "%Y-%m-%d").unwrap();
                                let items: Vec<Event> = (0..=(to - from).num_days()).flat_map(|ahead| world_day(from + Duration::days(ahead), today).items).collect();
                                let total = items.len();
                                Ok(calendar_range_evidence(asked, until, Agenda { items, total }))
                            }
                        }
                    }
                    "read_reminders" => Ok(reminder_evidence(&world_reminders(today))),
                    other => Err(format!("{other} isn't in this world")),
                };
                async move { found }
            }),
        )
        .await;
        let error = match result {
            Ok(Ok(())) => None,
            Ok(Err(error)) => Some(error),
            Err(_) => Some("timed out".to_string()),
        };
        let read: Vec<&str> = flow.steps.iter().filter(|step| step.state == "succeeded" && !step.effect).map(|step| step.tool.as_str()).collect();
        let needed = question.reads.iter().all(|tool| read.contains(tool)) && (!question.reads.is_empty() || read.is_empty());
        let records = crate::personal_answer::reads(question.asked, &flow.steps).is_some();
        let shown = error.clone().unwrap_or_else(|| task.answer.clone());
        // Reading it as a person would: a reply escapes what could become Markdown.
        let lower = shown.replace('\\', "").to_lowercase();
        let says = question.says.iter().all(|word| lower.contains(&word.to_lowercase()));
        let clean = !question.never.iter().any(|word| lower.contains(&word.to_lowercase()));
        let ok = error.is_none() && needed && says && clean && (question.reads.is_empty() || records);
        passed += usize::from(ok);
        let row = json!({
            "question": question.asked, "passed": ok, "needed": needed, "says": says, "clean": clean, "fromRecords": records,
            "read": read, "calls": flow.steps.iter().map(|step| json!({"tool":step.tool,"arguments":step.arguments,"state":step.state})).collect::<Vec<_>>(),
            "modelCalls": state.models.apple.model_calls(), "error": error, "answer": shown,
        });
        writeln!(out, "{row}").unwrap();
        out.flush().unwrap();
        eprintln!("PERSONAL {} {} (read {read:?})", if ok { "pass" } else { "FAIL" }, question.asked);
        state.models.shutdown().await;
    }
    eprintln!("PERSONAL {passed} of {asked} passed");
}

/// The same path over this Mac's real apps: what the test process may read, and how it
/// says so when it may not. Prints no contents: only what was read and whether the answer
/// holds nothing but what the reads returned.
#[tokio::test]
#[ignore = "reads this Mac's real Mail, Calendar and Reminders; run with --ignored live_real_apps"]
async fn live_real_apps() {
    let dir = tempfile::tempdir().unwrap();
    let state = live_state(dir.path());
    let mut settings = state.store.lock().unwrap().settings().unwrap();
    settings.apps = vec!["mail".into(), "calendar".into(), "reminders".into()];
    state.store.lock().unwrap().save_settings(&settings).unwrap();
    for question in ["what are my reminders", "whats due today", "how many reminders do I have", "any reminders about the car", "what do I need to do today", "whats on my mail", "did Ana write back?", "what's on my calendar today", "when's my dentist appointment", "hey buntini"] {
        let turn = super::apple_live_tests::run_live_prompt(&state, question).await;
        let steps = turn.workflow.as_ref().map(|flow| flow.steps.clone()).unwrap_or_default();
        let reads: Vec<String> = steps.iter().map(|step| format!("{}:{}", step.tool, step.state)).collect();
        let records: Vec<String> = steps
            .iter()
            .flat_map(|step| step.evidence.iter().flat_map(|evidence| evidence["records"].as_array()).flatten())
            .filter_map(|record| record["title"].as_str().or(record["subject"].as_str()).map(str::to_string))
            .collect();
        // Every list row names a record that was read.
        let answer = turn.task.answer.replace('\\', "");
        let rows: Vec<&str> = answer.lines().filter(|line| line.starts_with("- ")).collect();
        let grounded = rows.iter().all(|row| records.iter().any(|title| row.contains(title.split_whitespace().collect::<Vec<_>>().join(" ").as_str())));
        let message = if rows.is_empty() && records.is_empty() { answer.as_str() } else { "(records withheld)" };
        eprintln!("REAL {question} | reads {reads:?} | records {} | rows {} | every row is a record: {grounded} | model calls {} | {message}", records.len(), rows.len(), turn.apple_calls);
    }
    state.models.shutdown().await;
}
