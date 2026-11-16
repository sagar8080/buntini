use super::*;
use crate::surface::cards::mail::MailCard;

fn inbox() -> Mailbox {
    Mailbox { account: "iCloud".into(), path: "INBOX".into() }
}

fn message(id: i64, sender: &str, subject: &str, at: i64) -> Message {
    Message {
        id,
        header: format!("m{id}@x"),
        at,
        read: false,
        flagged: false,
        junk: false,
        sender: sender.into(),
        subject: subject.into(),
        mailbox: inbox(),
    }
}

fn pool(now: i64) -> Vec<Message> {
    let mut read = message(2, "Bank Alerts <alerts@bank.com>", "Your statement", now - 3600);
    read.read = true;
    let mut flagged = message(3, "Boss <boss@work.com>", "Q4 plan — review", now - 3 * 86_400);
    flagged.flagged = true;
    flagged.read = true;
    vec![message(1, "Ana Ruiz <ana@example.org>", "Dinner on Tuesday?", now - 60), read, flagged]
}

#[test]
fn filters_show_unread_important_and_today_newest_first() {
    let now = chrono::Local::now();
    let pool = pool(now.timestamp());
    let ids = |filter, query: &str| filtered(&pool, filter, query, now).iter().map(|m| m.id).collect::<Vec<_>>();
    assert_eq!(ids(Filter::All, ""), [1, 2, 3]);
    assert_eq!(ids(Filter::Unread, ""), [1]);
    assert_eq!(ids(Filter::Important, ""), [3]);
    // An hour ago may be yesterday just after midnight.
    let today = ids(Filter::Today, "");
    assert!(today.starts_with(&[1]) && !today.contains(&3), "{today:?}");
}

#[test]
fn search_looks_at_sender_and_subject_and_every_word_must_match() {
    let now = chrono::Local::now();
    let pool = pool(now.timestamp());
    let ids = |query: &str| filtered(&pool, Filter::All, query, now).iter().map(|m| m.id).collect::<Vec<_>>();
    assert_eq!(ids("ana"), [1]);
    assert_eq!(ids("DINNER tuesday"), [1]);
    assert_eq!(ids("from:bank"), [2]);
    assert_eq!(ids("subject:bank"), Vec::<i64>::new());
    assert_eq!(ids("subject:q4 from:boss"), [3]);
    assert_eq!(ids("q4 plan review"), [3]);
    assert_eq!(ids("nothing"), Vec::<i64>::new());
}

#[test]
fn rows_name_the_sender_and_the_mailbox() {
    let row = Row::from(&message(9, "\"Ana Ruiz\" <Ana@Example.org>", "Hi", 0));
    assert_eq!((row.from.as_str(), row.address.as_str(), row.mailbox.as_str()), ("Ana Ruiz", "ana@example.org", "INBOX"));
    assert_eq!(who("<only@address.com>"), "only@address.com");
    assert_eq!(who("plain@address.com"), "plain@address.com");
    assert_eq!(reply_subject("Re: Lunch"), "Re: Lunch");
    assert_eq!(reply_subject("RE: Lunch"), "RE: Lunch");
    assert_eq!(reply_subject("Lunch"), "Re: Lunch");
}

#[test]
fn the_island_shows_nothing_until_mail_was_read_and_waits_while_a_change_does() {
    let mut hub = Hub { shown: PAGE, ..Default::default() };
    assert_eq!(contribution(&hub), Contribution::default());
    hub.read = true;
    hub.pool = pool(chrono::Utc::now().timestamp());
    let quiet = contribution(&hub);
    assert_eq!(quiet.pills.len(), 1);
    assert_eq!(quiet.pills[0].badge.as_deref(), Some("1"));
    assert_eq!(quiet.pills[0].state, PillState::Idle);
    assert!(quiet.offers.is_empty());
    hub.card_at = Some(10);
    let shown = contribution(&hub);
    assert_eq!(shown.offers[0].urgency, Urgency::Info);
    assert_eq!(shown.offers[0].ttl_ms, Some(CARD_MS));
    let target = hub.pool[0].target();
    hub.pending = Some(Pending {
        view: Waiting { id: "p".into(), title: "Flag".into(), lines: vec!["Ana Ruiz · Dinner on Tuesday?".into()] },
        steps: vec![(Step::Mark { target, what: Mark::Flagged, value: true }, Why::Manual)],
        made: Instant::now(),
    });
    let waiting = contribution(&hub);
    assert_eq!(waiting.pills[0].state, PillState::Waiting);
    assert_eq!(waiting.offers[0].urgency, Urgency::Waiting);
    assert_eq!(waiting.offers[0].ttl_ms, None);
    let CardBody::Mail(MailCard { pending, messages, unread, .. }) = &waiting.offers[0].card.body else { panic!() };
    assert_eq!(pending.as_ref().unwrap().title, "Flag");
    assert_eq!((messages.len(), *unread), (3, 1));
    // A closed Mail has no pill.
    hub.closed = true;
    hub.pending = None;
    hub.card_at = None;
    hub.pool.clear();
    assert!(contribution(&hub).pills.is_empty());
}

#[test]
fn sorting_lists_each_change_with_its_reason() {
    let settings = Settings {
        vips: vec!["ana@example.org".into()],
        banned: vec!["statement".into()],
        rules: vec![rules::Rule {
            id: "boss".into(),
            field: rules::Field::Sender,
            value: "boss@work.com".into(),
            action: rules::RuleAction::Move,
            to: Some(Mailbox { account: "iCloud".into(), path: "Work/Boss".into() }),
            enabled: true,
        }],
    };
    let steps = sort_steps(&settings, &pool(0));
    let lines: Vec<&str> = steps.iter().map(|(_, _, line)| line.as_str()).collect();
    assert_eq!(lines, ["Flag · Ana Ruiz · Dinner on Tuesday?", "Junk · Bank Alerts · Your statement", "To Boss · Boss · Q4 plan — review"]);
    assert_eq!(steps[0].1, Why::Vip);
    assert_eq!(steps[1].1, Why::Banned { word: "statement".into() });
    assert!(matches!(&steps[2].0, Step::Move { to, .. } if to.path == "Work/Boss"));
}

#[test]
fn a_merge_writes_each_persons_page_and_drafts_it_to_them() {
    let dir = tempfile::tempdir().unwrap();
    let pages = vec![
        merge::Page { text: "Payslip · Ana Ruiz".into(), pdf: b"%PDF-1 page one".to_vec() },
        merge::Page { text: "Payslip · Bo Ruiz".into(), pdf: b"%PDF-1 page two".to_vec() },
    ];
    let pdf = plan::Pdf { name: "payslips.pdf".into(), path: "/x/payslips.pdf".into(), pages: pages.iter().map(|p| p.text.clone()).collect() };
    let messages = pool(0);
    let mailboxes = vec![inbox()];
    let request = "Their payslips: Bo Ruiz bo@example.org, Ana Ruiz ana@example.org";
    let cx = plan::Context { request, messages: &messages, mailboxes: &mailboxes, pdf: Some(&pdf), by_words: false };
    let raw = json!({"steps":[
        {"do":"merge","subject":"Payslip for {name}","body":"Hi {name}"},
        {"do":"read","message":1}
    ]});
    let planned = Planned { plan: plan::validate(&raw, &cx).unwrap(), messages: messages.clone(), pages };
    let steps = plan_steps(&planned, dir.path()).unwrap();
    assert_eq!(steps.len(), 3);
    assert!(matches!(&steps[0].0, Step::Mark { target, what: Mark::Read, value: true } if target.id == 1));
    let Step::Draft { to, subject, body, files, shown } = &steps[1].0 else { panic!() };
    assert_eq!((to.as_str(), subject.as_str(), body.as_str(), *shown), ("bo@example.org", "Payslip for Bo Ruiz", "Hi Bo Ruiz", false));
    assert_eq!(files[0].file_name().unwrap(), "payslips - Bo Ruiz.pdf");
    assert_eq!(std::fs::read(&files[0]).unwrap(), b"%PDF-1 page two");
    assert!(steps.iter().all(|(_, why)| *why == Why::Plan));
}

#[test]
fn ops_read_from_the_app() {
    let op: Op = serde_json::from_value(json!({"op":"list","filter":"important","query":"ana"})).unwrap();
    assert!(matches!(op, Op::List { filter: Filter::Important, .. }));
    let op: Op = serde_json::from_value(json!({"op":"mark","id":"14","change":"unflag"})).unwrap();
    assert!(matches!(op, Op::Mark { change: MarkChange::Unflag, .. }));
    assert!(matches!(serde_json::from_value(json!({"op":"runPlan","id":"p"})).unwrap(), Op::RunPlan { .. }));
    assert!(serde_json::from_value::<Op>(json!({"op":"delete","id":"14"})).is_err());
    assert!(serde_json::from_value::<Op>(json!({"op":"junk","id":"14","extra":1})).is_err());
}

#[test]
fn a_dropped_file_becomes_a_shown_draft_and_folders_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("report.pdf");
    std::fs::write(&file, b"%PDF").unwrap();
    let waiting = propose_attach(&[file.to_string_lossy().into_owned()]).unwrap();
    assert_eq!((waiting.title.as_str(), waiting.lines.as_slice()), ("New draft", ["report.pdf".to_string()].as_slice()));
    let pending = hub().pending.take().unwrap();
    assert!(matches!(&pending.steps[0].0, Step::Draft { subject, shown: true, files, .. } if subject == "report" && files == &vec![file.clone()]));
    assert!(propose_attach(&[dir.path().to_string_lossy().into_owned()]).is_err());
    assert!(propose_attach(&["relative.pdf".into()]).is_err());
    assert!(propose_attach(&[]).is_err());
}

// ----- Live: the real Mail.app on this Mac -----
//
// These need Mail open with a local mailbox "Import/BuntiniTest" holding the six test emails
// (Message-IDs <ana1|bank1|spam1|news1|boss1|ana2.buntini-test@example.com>) and a local
// mailbox "Buntini Sorted". Mail on this Mac has no accounts, so there's no real inbox.
// Run: cargo test --lib mail::tests::live -- --ignored --test-threads=1 --nocapture

#[cfg(target_os = "macos")]
mod live {
    use super::*;

    fn test_box() -> Mailbox {
        Mailbox { account: String::new(), path: "Import/BuntiniTest".into() }
    }

    fn sorted_box() -> Mailbox {
        Mailbox { account: String::new(), path: "Buntini Sorted".into() }
    }

    async fn ready(dir: &Path) {
        {
            let mut hub = hub();
            hub.boxes = vec![test_box()];
            hub.dir = Some(dir.to_path_buf());
            hub.log = log::Log::default();
        }
        refresh(false).await.unwrap();
        assert!(!hub().closed, "open Mail first");
    }

    fn find(header: &str) -> Message {
        hub().pool.iter().find(|m| m.header == header).cloned().unwrap_or_else(|| panic!("{header} isn't in the test mailbox"))
    }

    async fn in_box(mailbox: &Mailbox, header: &str) -> Option<Message> {
        list(std::slice::from_ref(mailbox)).await.unwrap().into_iter().find(|m| m.header == header)
    }

    async fn allow(waiting: Waiting) -> Done {
        let done = approve(&waiting.id).await.unwrap();
        assert!(done.failed.is_empty(), "{:?}", done.failed);
        done
    }

    fn newest_entry() -> log::Entry {
        hub().log.recent(1)[0].clone()
    }

    #[tokio::test]
    #[ignore = "uses Mail.app on this Mac"]
    async fn live_mail_marks_read_back_and_undo() {
        let dir = tempfile::tempdir().unwrap();
        ready(dir.path()).await;
        let header = "ana1.buntini-test@example.com";
        let before = find(header);
        assert_eq!(before.subject, "Dinner on Tuesday?");

        for (what, value) in [(Mark::Flagged, !before.flagged), (Mark::Read, !before.read)] {
            let waiting = propose_mark(&find(header).id.to_string(), what, value).unwrap();
            assert_eq!(waiting.lines, ["Ana Ruiz · Dinner on Tuesday?"]);
            let done = allow(waiting).await;
            println!("{:?}", done.changed);
            let now = in_box(&test_box(), header).await.unwrap();
            assert_eq!(if what == Mark::Flagged { now.flagged } else { now.read }, value, "read back from Mail");
            let entry = newest_entry();
            assert!(entry.undoable());
            allow(propose_undo(&entry.id).unwrap()).await;
            let undone = in_box(&test_box(), header).await.unwrap();
            assert_eq!(if what == Mark::Flagged { undone.flagged } else { undone.read }, !value, "undo read back from Mail");
            assert!(hub().log.get(&entry.id).unwrap().undone);
            assert!(propose_undo(&entry.id).is_err(), "undone once");
            refresh(false).await.unwrap();
        }
        // The log was saved where the app keeps it.
        assert!(log::Log::load(&log_path(dir.path())).entries.len() >= 4);
    }

    #[tokio::test]
    #[ignore = "uses Mail.app on this Mac"]
    async fn live_mail_junk_and_move_undo() {
        let dir = tempfile::tempdir().unwrap();
        ready(dir.path()).await;
        // Junk: mail on this Mac has no junk mailbox, so it stays and only the mark changes.
        let header = "spam1.buntini-test@example.com";
        allow(propose_junk(&find(header).id.to_string()).unwrap()).await;
        assert!(in_box(&test_box(), header).await.unwrap().junk);
        allow(propose_undo(&newest_entry().id).unwrap()).await;
        assert!(!in_box(&test_box(), header).await.unwrap().junk);

        // Move out and back.
        let header = "news1.buntini-test@example.com";
        let id = find(header).id;
        allow(propose_move(&id.to_string(), sorted_box()).unwrap()).await;
        let moved = in_box(&sorted_box(), header).await.expect("moved into Buntini Sorted");
        assert_ne!(moved.id, id, "Mail renumbers a moved email");
        assert!(in_box(&test_box(), header).await.is_none());
        let entry = newest_entry();
        assert_eq!(entry.label(), "To Buntini Sorted · This week in tech");
        allow(propose_undo(&entry.id).unwrap()).await;
        assert!(in_box(&test_box(), header).await.is_some(), "back where it was");
        assert!(in_box(&sorted_box(), header).await.is_none());
    }

    #[tokio::test]
    #[ignore = "uses Mail.app on this Mac"]
    async fn live_mail_sorts_by_rules_and_undoes_every_move() {
        let dir = tempfile::tempdir().unwrap();
        ready(dir.path()).await;
        let settings = Settings {
            vips: vec!["boss@work.example.com".into()],
            banned: vec!["lottery".into()],
            rules: vec![rules::Rule {
                id: "news".into(),
                field: rules::Field::Domain,
                value: "news.example.com".into(),
                action: rules::RuleAction::Move,
                to: Some(sorted_box()),
                enabled: true,
            }],
        };
        let pool = hub().pool.clone();
        let steps = sort_steps(&settings, &pool);
        let lines: Vec<String> = steps.iter().map(|(_, _, line)| line.clone()).collect();
        println!("{lines:#?}");
        assert_eq!(steps.len(), 3, "{lines:?}");
        let waiting = propose(format!("Sort {}", steps.len()), lines, steps.into_iter().map(|(s, w, _)| (s, w)).collect());
        let done = allow(waiting).await;
        assert_eq!(done.changed.len(), 3);
        assert!(in_box(&test_box(), "boss1.buntini-test@example.com").await.unwrap().flagged);
        assert!(in_box(&test_box(), "spam1.buntini-test@example.com").await.unwrap().junk);
        assert!(in_box(&sorted_box(), "news1.buntini-test@example.com").await.is_some());
        let entries: Vec<log::Entry> = hub().log.recent(3).into_iter().cloned().collect();
        for entry in entries {
            allow(propose_undo(&entry.id).unwrap()).await;
        }
        assert!(!in_box(&test_box(), "boss1.buntini-test@example.com").await.unwrap().flagged);
        assert!(!in_box(&test_box(), "spam1.buntini-test@example.com").await.unwrap().junk);
        assert!(in_box(&test_box(), "news1.buntini-test@example.com").await.is_some());
    }

    #[tokio::test]
    #[ignore = "uses Mail.app on this Mac"]
    async fn live_mail_reads_a_message_and_lists_mailboxes() {
        let dir = tempfile::tempdir().unwrap();
        ready(dir.path()).await;
        let opened = open(&find("boss1.buntini-test@example.com").id.to_string()).await.unwrap();
        assert_eq!(opened.subject, "Q4 plan \"draft\" — review");
        assert!(opened.body.contains("Can you review the attached plan by Friday?\nThanks,"), "{:?}", opened.body);
        assert_eq!(opened.to, ["me@example.com"]);
        let boxes = mailboxes().await.unwrap();
        assert!(boxes.contains(&test_box()) && boxes.contains(&sorted_box()), "{boxes:?}");
    }

    /// The real on-device brain plans over the real test mailbox and a real split PDF.
    /// Requests are fresh wordings; read the printed answers, not only the pass.
    #[tokio::test]
    #[ignore = "uses Mail.app and Apple Intelligence on this Mac"]
    async fn live_mail_plan_on_the_apple_model() {
        let dir = tempfile::tempdir().unwrap();
        ready(dir.path()).await;
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf();
        let models = crate::model::ModelHub::new(root.join("worker/main.py"), root.join("native/buntini-fm"));
        let (available, reason) = models.probe_apple().await;
        assert!(available, "Apple Intelligence unavailable: {reason}");
        let settings = crate::types::Settings { model_backend: "apple".into(), ..Default::default() };
        let pdf_path = dir.path().join("payslips.pdf");
        std::fs::write(&pdf_path, merge::tests_pdf()).unwrap();
        let pages = merge::split(&root.join("native/buntini-fm"), &pdf_path).await.unwrap();
        let pdf = plan::Pdf { name: "payslips.pdf".into(), path: pdf_path.clone(), pages: pages.iter().map(|p| p.text.clone()).collect() };
        let messages: Vec<Message> = hub().pool.clone();
        let boxes = mailboxes().await.unwrap();
        let mut openings = HashMap::new();
        for message in messages.iter().take(plan::OPENED) {
            if let Ok(opened) = read(message).await {
                openings.insert(message.id, opened.body);
            }
        }
        // Tuned on: the first run scored 0 of 3, and the design changed after reading them.
        // Held out: written afterwards and never tuned on. Each lists the lines a right plan shows.
        let cases: [(&str, &str, bool, &[&str]); 16] = [
            ("tuned", "Put the tech newsletter in Buntini Sorted, flag what my boss sent, and mark the lottery one as junk.", false,
                &["To Buntini Sorted · Weekly News · This week in tech", "Flag · Boss Person · Q4 plan \"draft\" — review", "Junk · Prize Desk · You won the lottery!!!"]),
            ("tuned", "Answer Ana about dinner: Tuesday is good for me, 7pm.", false, &["Reply draft · Ana Ruiz · Dinner on Tuesday?"]),
            ("tuned", "Mail each person their own payslip from the PDF. Addresses: Ana Ruiz ana@example.org, Bo Ruiz bo@example.org, Chloe Martin chloe@example.org. Subject: Your November payslip.", true,
                &["3 drafts · Your November payslip · Ana Ruiz p1, Bo Ruiz p2, Chloe Martin p3"]),
            ("held out", "Flag the email about the Q4 plan and mark the bank statement as read.", false,
                &["Flag · Boss Person · Q4 plan \"draft\" — review", "Read · Bank Alerts · Your statement is ready"]),
            ("held out", "That prize email is spam, junk it.", false, &["Junk · Prize Desk · You won the lottery!!!"]),
            ("held out", "Move the weekly tech digest out of the way into Buntini Sorted.", false, &["To Buntini Sorted · Weekly News · This week in tech"]),
            ("held out", "Tell my boss I'll have the review done by Thursday.", false, &["Reply draft · Boss Person · Q4 plan \"draft\" — review"]),
            ("held out", "Reply to Ana's trip photos email to say thanks, they look great.", false, &["Reply draft · Ana Ruiz · Photos from the trip"]),
            ("held out", "Mark the statement from the bank as read and flag Ana's dinner question.", false,
                &["Flag · Ana Ruiz · Dinner on Tuesday?", "Read · Bank Alerts · Your statement is ready"]),
            ("held out", "Send Bo Ruiz (bo@example.org) and Chloe Martin (chloe@example.org) their pages from the PDF.", true,
                &["2 drafts · payslips · Bo Ruiz p2, Chloe Martin p3"]),
            ("held out", "Using the attached PDF, give Ana Ruiz <ana@example.org> her payslip, subject: Payslip for November", true,
                &["1 drafts · Payslip for November · Ana Ruiz p1"]),
            // Written after the fixes for the first held-out set, and never tuned on.
            ("held out 2", "Junk the email from Prize Desk and flag the one about photos.", false,
                &["Junk · Prize Desk · You won the lottery!!!", "Flag · Ana Ruiz · Photos from the trip"]),
            ("held out 2", "Put the bank email into Buntini Sorted.", false, &["To Buntini Sorted · Bank Alerts · Your statement is ready"]),
            ("held out 2", "Let Ana know I can't make dinner Tuesday, maybe Wednesday instead.", false, &["Reply draft · Ana Ruiz · Dinner on Tuesday?"]),
            ("held out 2", "Mark the newsletter as read.", false, &["Read · Weekly News · This week in tech"]),
            ("held out 2", "Each of these people should get their payslip page as a draft: Chloe Martin chloe@example.org, Ana Ruiz ana@example.org.", true,
                &["2 drafts · payslips · Chloe Martin p3, Ana Ruiz p1"]),
        ];
        let by_words = models.small(&settings);
        let mut scores: HashMap<&str, (usize, usize)> = HashMap::new();
        for (set, request, with_pdf, expected) in cases {
            let cx = plan::Context { request, messages: &messages, mailboxes: &boxes, pdf: with_pdf.then_some(&pdf), by_words };
            let payload = json!({"messages": plan::prompt(&cx, &openings, &Settings::default()), "tools": [], "max_tokens": 1800});
            let turn = models.generate(&settings, payload, Arc::new(AtomicBool::new(false)), |_| {}).await.unwrap();
            println!("\n=== [{set}] {request}\n--- answer:\n{}", turn.text);
            let mut got = match plan::parse(&turn.text, &cx) {
                Ok(plan) => {
                    println!("--- left out: {:?}", plan.problems);
                    plan::lines(&plan, &messages)
                }
                Err(error) => {
                    println!("--- refused: {error}");
                    vec![]
                }
            };
            got.sort();
            let mut want: Vec<String> = expected.iter().map(|line| line.to_string()).collect();
            want.sort();
            // Merge lines carry the brain's subject when it wrote one; compare people and pages.
            let same = got.len() == want.len()
                && got.iter().zip(&want).all(|(g, w)| g == w || (w.contains(" drafts · ") && g.ends_with(w.rsplit(" · ").next().unwrap()) && g.starts_with(w.split(" · ").next().unwrap())));
            println!("--- plan: {got:#?}\n--- {}", if same { "RIGHT" } else { "WRONG" });
            let score = scores.entry(set).or_default();
            score.1 += 1;
            if same {
                score.0 += 1;
            }
        }
        models.shutdown().await;
        println!("\n{scores:?}");
    }

    #[tokio::test]
    #[ignore = "uses Mail.app and native/buntini-fm on this Mac; leaves drafts in Mail’s Drafts"]
    async fn live_mail_runs_a_plan_with_a_merge() {
        let dir = tempfile::tempdir().unwrap();
        ready(dir.path()).await;
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf();
        let pdf_path = dir.path().join("payslips.pdf");
        std::fs::write(&pdf_path, merge::tests_pdf()).unwrap();
        let pages = merge::split(&root.join("native/buntini-fm"), &pdf_path).await.unwrap();
        let pdf = plan::Pdf { name: "payslips.pdf".into(), path: pdf_path, pages: pages.iter().map(|p| p.text.clone()).collect() };
        let messages = hub().pool.clone();
        let boxes = mailboxes().await.unwrap();
        let stamp = chrono::Local::now().format("%H%M%S").to_string();
        let request = format!("Payslips for Ana Ruiz ana@example.org and Bo Ruiz bo@example.org. Subject: Buntini merge test {stamp}");
        let cx = plan::Context { request: &request, messages: &messages, mailboxes: &boxes, pdf: Some(&pdf), by_words: true };
        let raw = json!({"summary":"Payslips","steps":[{"do":"merge","body":"Hi {name}, your payslip is attached."},{"do":"read","about":"lottery"}]});
        let plan = plan::validate(&raw, &cx).unwrap();
        println!("{:#?}", plan::lines(&plan, &messages));
        let id = plan.id.clone();
        hub().planned = Some(Planned { plan, messages: messages.clone(), pages });
        let done = run_plan(&id).await.unwrap();
        println!("{done:#?}");
        assert!(done.failed.is_empty(), "{:?}", done.failed);
        assert_eq!(done.changed.len(), 3);
        let drafts: Vec<log::Entry> = hub().log.recent(3).into_iter().cloned().collect();
        let attached: Vec<Vec<String>> = drafts
            .iter()
            .filter_map(|e| match &e.change {
                log::Change::Draft { attachments, to, .. } => Some([to.clone(), attachments.clone()].concat()),
                _ => None,
            })
            .collect();
        assert!(attached.contains(&vec!["ana@example.org".to_string(), "payslips - Ana Ruiz.pdf".to_string()]), "{attached:?}");
        assert!(attached.contains(&vec!["bo@example.org".to_string(), "payslips - Bo Ruiz.pdf".to_string()]), "{attached:?}");
        // The mark is undone; drafts stay.
        let read = drafts.iter().find(|e| matches!(e.change, log::Change::Mark { .. })).unwrap().clone();
        allow(propose_undo(&read.id).unwrap()).await;
        assert!(!in_box(&test_box(), "spam1.buntini-test@example.com").await.unwrap().read);
    }

    /// The actions the island's mail card sends (checked by clicking it in SurfaceLab), handled
    /// as the island hands them over, all the way into Mail and back with Undo.
    #[tokio::test]
    #[ignore = "uses Mail.app on this Mac"]
    async fn live_mail_island_actions_reach_mail_and_undo() {
        let dir = tempfile::tempdir().unwrap();
        ready(dir.path()).await;
        let island = |name: &str, value: Value| Action { source: Source::Mail, card: Some("mail".into()), name: name.into(), value: Some(value) };
        let header = "bank1.buntini-test@example.com";
        let id = find(header).id.to_string();
        act(island("filter", json!("unread"))).await.unwrap();
        act(island("open", json!(id))).await.unwrap();
        assert!(hub().open.as_ref().is_some_and(|(_, opened)| opened.body.contains("monthly statement")));
        act(island("flag", json!(id))).await.unwrap();
        let pending = hub().pending.as_ref().map(|p| p.view.clone()).unwrap();
        assert_eq!((pending.title.as_str(), pending.lines.as_slice()), ("Flag", ["Bank Alerts · Your statement is ready".to_string()].as_slice()));
        // Nothing changes before Allow.
        assert!(!in_box(&test_box(), header).await.unwrap().flagged);
        act(island("allow", json!(pending.id))).await.unwrap();
        assert!(in_box(&test_box(), header).await.unwrap().flagged);
        let card = mail_card(&hub());
        assert_eq!(card.log[0].label, "Flagged · Your statement is ready");
        act(island("undo", json!(card.log[0].id))).await.unwrap();
        let undo = hub().pending.as_ref().map(|p| p.view.id.clone()).unwrap();
        act(island("deny", json!(undo))).await.unwrap();
        assert!(hub().pending.is_none() && in_box(&test_box(), header).await.unwrap().flagged, "Deny leaves it");
        act(island("undo", json!(card.log[0].id))).await.unwrap();
        let undo = hub().pending.as_ref().map(|p| p.view.id.clone()).unwrap();
        act(island("allow", json!(undo))).await.unwrap();
        assert!(!in_box(&test_box(), header).await.unwrap().flagged);
        assert!(!mail_card(&hub()).log.iter().any(|row| row.id == card.log[0].id && row.undoable));
    }

    /// Run with Mail quit: no read, background or asked for, opens it.
    #[tokio::test]
    #[ignore = "needs Mail.app quit first"]
    async fn live_mail_closed_stays_closed() {
        let running = || std::process::Command::new("/usr/bin/pgrep").args(["-x", "Mail"]).status().unwrap().success();
        assert!(!running(), "quit Mail first");
        hub().boxes = vec![test_box()];
        let quiet = refresh(true).await;
        let asked = refresh(false).await;
        let listed = list(&[]).await;
        let open = mail_open().await;
        tokio::time::sleep(Duration::from_secs(3)).await;
        println!("{quiet:?} {asked:?} {listed:?} {open:?}");
        assert!(!running(), "Mail was opened");
        assert!(hub().closed);
        assert_eq!(listed.unwrap_err(), script::CLOSED_TEXT);
        assert_eq!(open.unwrap_err(), script::CLOSED_TEXT);
        assert!(contribution(&hub()).pills.is_empty());
    }

    #[tokio::test]
    #[ignore = "uses Mail.app on this Mac; leaves drafts in Mail’s Drafts"]
    async fn live_mail_saves_an_attachment_draft_and_a_reply_draft() {
        let dir = tempfile::tempdir().unwrap();
        ready(dir.path()).await;
        let file = dir.path().join("Buntini test attachment.pdf");
        std::fs::write(&file, merge::tests_pdf()).unwrap();
        let waiting = propose_attach(&[file.to_string_lossy().into_owned()]).unwrap();
        // Saved without showing it, so the test doesn't leave a window open.
        hub().pending.as_mut().unwrap().steps[0].0 = Step::Draft {
            to: "ana@example.org".into(),
            subject: "Buntini test attachment".into(),
            body: "Here it is.".into(),
            files: vec![file.clone()],
            shown: false,
        };
        let done = allow(waiting).await;
        println!("{:?}", done.changed);
        let entry = newest_entry();
        assert!(matches!(&entry.change, log::Change::Draft { to, attachments, .. } if to == &["ana@example.org"] && attachments.len() == 1));
        assert!(!entry.undoable());

        let reply = propose_reply(&find("ana2.buntini-test@example.com").id.to_string(), "Lovely, thanks!").unwrap();
        let done = approve(&reply.id).await.unwrap();
        println!("reply: {done:?}");
        assert!(done.failed.is_empty(), "{:?}", done.failed);
        assert!(matches!(newest_entry().change, log::Change::Reply { .. }));
    }
}
