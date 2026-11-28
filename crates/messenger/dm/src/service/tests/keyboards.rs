// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The buttons of a bot in a direct chat (platform-spec §9.7,
//! the rules of the buttons (sdk/docs/bridge.md, «Buttons under a message»)): every rule of the security of the
//! plan has its test here, named after the rule.

use super::*;
use crate::keyboard::{KbState, PreparedPress, UI_EVENT_DM_PRESS};
use messenger_core::keyboard::{
    Button, ButtonStyle, Keyboard, PRESS_DEDUP_SECS, PRESS_REPEAT_SECS,
};

fn kb() -> Keyboard {
    Keyboard {
        rows: vec![
            vec![Button::data("Allow", "a:1").styled(ButtonStyle::Primary)],
            vec![
                Button::data("Always", "s:1"),
                Button::data("Deny", "d:1").styled(ButtonStyle::Danger),
            ],
        ],
    }
}

/// `whose` kind 0 as `who` has it: a bot or a person (a person's dated
/// before a bot's, so that the bot's replaces it).
async fn profile(who: &Party, whose: &Party, bot: bool) {
    let (content, at) = if bot {
        (r#"{"name":"Bot","bot":true}"#, 11)
    } else {
        (r#"{"name":"Ann"}"#, 10)
    };
    assert!(who
        .dm
        .profiles
        .apply_event(&whose.pk(), Timestamp(at), content)
        .await
        .unwrap());
}

/// A bot and a person who knows it is one, and a text with buttons from
/// the bot that reached the person.
async fn bot_and_person() -> (Party, Party, Prepared) {
    let bot = Party::new().await;
    let person = Party::new().await;
    profile(&person, &bot, true).await;
    let p = bot
        .dm
        .prepare_text_with(
            &bot.keys,
            &person.pk(),
            "Open wttr.in?",
            TextFormat::Plain,
            None,
            Some(&kb()),
        )
        .await
        .unwrap();
    person.receive(&peer_event(&p)).await;
    (bot, person, p)
}

fn press_event_of(p: &PreparedPress) -> WireEvent {
    match &p.to_peer {
        Outbound::PublishToInbox { event, .. } => event.clone(),
        other => panic!("unexpected {other:?}"),
    }
}

fn presses(fx: &[Effect]) -> Vec<serde_json::Value> {
    fx.iter()
        .filter_map(|e| match e {
            Effect::Emit(u) if u.name == UI_EVENT_DM_PRESS => Some(u.payload.clone()),
            _ => None,
        })
        .collect()
}

async fn row(p: &Party, id: &str) -> MessageRow {
    repo::get(&p.dm.store, id).await.unwrap().unwrap()
}

#[tokio::test]
async fn a_text_with_buttons_of_a_bot_reaches_the_person_with_its_keyboard() {
    let (bot, person, p) = bot_and_person().await;
    assert_eq!(p.message.kb, Some(kb()), "the bot sees its own buttons");
    assert_eq!(p.message.media, None, "the buttons are no file");
    let seen = person.dm.message(&p.message.id).await.unwrap().unwrap();
    assert_eq!(
        (seen.text.as_deref(), seen.kb.clone(), seen.pressed.clone()),
        (Some("Open wttr.in?"), Some(kb()), None)
    );
    assert_eq!(seen.media, None);
    // The view as the UI reads it: the wire's shape, labels cleaned.
    let json = serde_json::to_value(&seen).unwrap();
    assert_eq!(
        json["kb"]["rows"][0][0],
        serde_json::json!({"t":"Allow","d":"a:1","style":"primary"})
    );
    assert_eq!(
        json["kb"]["rows"][1][0],
        serde_json::json!({"t":"Always","d":"s:1"})
    );
    assert!(json.get("pressed").is_none());
    drop(bot);
}

#[tokio::test]
async fn kb_from_a_person_is_dropped_before_storage() {
    let ann = Party::new().await;
    let bob = Party::new().await;
    profile(&bob, &ann, false).await;
    let p = ann
        .dm
        .prepare_text_with(
            &ann.keys,
            &bob.pk(),
            "pick one",
            TextFormat::Plain,
            None,
            Some(&kb()),
        )
        .await
        .unwrap();
    bob.receive(&peer_event(&p)).await;
    let r = row(&bob, &p.message.id).await;
    assert_eq!(r.media_json, None, "nothing of the keyboard is kept");
    let seen = bob.dm.message(&p.message.id).await.unwrap().unwrap();
    assert_eq!(
        (seen.text.as_deref(), seen.kb),
        (Some("pick one"), None),
        "the text stays"
    );
    // A profile saying «bot» later brings nothing back: it was never kept.
    profile(&bob, &ann, true).await;
    assert_eq!(
        bob.dm.message(&p.message.id).await.unwrap().unwrap().kb,
        None
    );
}

#[tokio::test]
async fn kb_of_a_key_not_known_yet_is_kept_and_shown_once_its_profile_says_bot() {
    let bot = Party::new().await;
    let person = Party::new().await;
    let p = bot
        .dm
        .prepare_text_with(
            &bot.keys,
            &person.pk(),
            "hi",
            TextFormat::Plain,
            None,
            Some(&kb()),
        )
        .await
        .unwrap();
    person.receive(&peer_event(&p)).await;
    assert!(
        row(&person, &p.message.id).await.media_json.is_some(),
        "kept in the row"
    );
    assert_eq!(
        person.dm.message(&p.message.id).await.unwrap().unwrap().kb,
        None,
        "not drawn until the profile comes"
    );
    assert_eq!(
        reason(
            person
                .dm
                .prepare_press(&person.keys, &p.message.id, "a:1")
                .await
                .unwrap_err()
        ),
        "press_not_bot"
    );
    profile(&person, &bot, true).await;
    assert_eq!(
        person.dm.message(&p.message.id).await.unwrap().unwrap().kb,
        Some(kb())
    );
    // A profile that says «person» hides them again.
    assert!(person
        .dm
        .profiles
        .apply_event(&bot.pk(), Timestamp(20), r#"{"name":"Bot"}"#)
        .await
        .unwrap());
    assert_eq!(
        person.dm.message(&p.message.id).await.unwrap().unwrap().kb,
        None
    );
}

#[tokio::test]
async fn a_keyboard_over_a_limit_is_refused_at_the_sender_and_nothing_is_stored() {
    let bot = Party::new().await;
    let person = Party::new().await;
    let nine = Keyboard {
        rows: vec![vec![Button::data("x", "1")]; 9],
    };
    let five = Keyboard {
        rows: vec![vec![Button::data("x", "1"); 5]],
    };
    let long = Keyboard {
        rows: vec![vec![Button::data(&"я".repeat(65), "1")]],
    };
    for (k, code) in [
        (nine, "kb_rows"),
        (five, "kb_row_buttons"),
        (long, "kb_label_long"),
    ] {
        let e = bot
            .dm
            .prepare_text_with(
                &bot.keys,
                &person.pk(),
                "x",
                TextFormat::Plain,
                None,
                Some(&k),
            )
            .await
            .unwrap_err();
        assert_eq!(reason(e), code);
    }
    assert!(
        bot.dm
            .chat(&chats::dm_chat_id(person.pk().as_hex(), None))
            .await
            .unwrap()
            .is_none(),
        "no chat, no row"
    );
    let p = bot
        .dm
        .prepare_text(&bot.keys, &person.pk(), "x", None)
        .await
        .unwrap();
    let bad = Keyboard { rows: vec![vec![]] };
    assert_eq!(
        reason(
            bot.dm
                .prepare_edit_with(&bot.keys, &p.message.id, "y", TextFormat::Plain, Some(&bad))
                .await
                .unwrap_err()
        ),
        "kb_empty_row"
    );
    assert_eq!(
        bot.dm
            .message(&p.message.id)
            .await
            .unwrap()
            .unwrap()
            .text
            .as_deref(),
        Some("x"),
        "the edit did not happen"
    );
}

#[tokio::test]
async fn a_broken_kb_keeps_the_text() {
    let bot = Party::new().await;
    let person = Party::new().await;
    profile(&person, &bot, true).await;
    for (i, broken) in [
        r#"{"rows":[[{"t":"x","d":"1","url":"javascript:alert(1)"}]]}"#,
        r#"{"rows":[[{"t":"\u202e\u202c","d":"1"}]]}"#,
        r#"{"rows":"nope"}"#,
        r#"{"rows":[[{"t":"x"}]]}"#,
    ]
    .into_iter()
    .enumerate()
    {
        let content = format!(r#"{{"v":1,"t":"text","text":"words {i}","kb":{broken}}}"#);
        let w = wrap(&bot.keys, person.pk(), &content, 1_000_000 + i as i64, None).unwrap();
        person.receive(&w.to_peer).await;
        let id = w.rumor_id.as_hex().to_string();
        let seen = person.dm.message(&id).await.unwrap().unwrap();
        assert_eq!(
            (seen.text, seen.kb),
            (Some(format!("words {i}")), None),
            "{broken}"
        );
        assert_eq!(
            row(&person, &id).await.media_json,
            None,
            "{broken}: nothing kept"
        );
    }
}

#[tokio::test]
async fn rtlo_and_bidi_in_a_label_are_stripped_before_it_is_kept() {
    let bot = Party::new().await;
    let person = Party::new().await;
    profile(&person, &bot, true).await;
    let content = r#"{"v":1,"t":"text","text":"?","kb":{"rows":[[{"t":"\u202eьтишерзаР\u202c\u2066!\u2069","d":"1"}]]}}"#;
    let w = wrap(&bot.keys, person.pk(), content, 1_000_000, None).unwrap();
    person.receive(&w.to_peer).await;
    let seen = person
        .dm
        .message(w.rumor_id.as_hex())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(seen.kb.unwrap().rows[0][0].t, "ьтишерзаР!");
}

#[tokio::test]
async fn a_press_reaches_the_bot_and_only_the_bot_hears_it() {
    let (bot, person, p) = bot_and_person().await;
    let id = p.message.id.clone();
    let pressed = person
        .dm
        .prepare_press(&person.keys, &id, "a:1")
        .await
        .unwrap();
    assert_eq!(pressed.chat_id, chats::dm_chat_id(bot.pk().as_hex(), None));
    assert_eq!(
        pressed.message.pressed,
        Some(crate::keyboard::Pressed {
            d: "a:1".into(),
            at: 1_000_000
        })
    );
    assert_eq!(
        pressed.message.kb,
        Some(kb()),
        "the buttons stay until the bot edits"
    );
    // A note to the bot: no copy for my devices, quiet for the push server.
    let ev = press_event_of(&pressed);
    let outer: Event = serde_json::from_value(ev.json.clone()).unwrap();
    assert!(
        outer
            .tags
            .iter()
            .any(|t| t.kind() == crate::pushtags::SILENT),
        "no push"
    );
    let u = UnwrappedGift::from_gift_wrap(&bot.keys, &outer).unwrap();
    assert_eq!(u.rumor.kind.as_u16(), KIND_PEER_NOTE_RUMOR);
    assert_eq!(
        Envelope::parse(&u.rumor.content).unwrap(),
        Envelope::press(&id, "a:1")
    );

    let fx = bot.receive(&ev).await;
    assert_eq!(
        names(&fx),
        vec![UI_EVENT_DM_PRESS],
        "one event, no notification, nothing for the timeline"
    );
    assert_eq!(
        presses(&fx),
        vec![serde_json::json!({
            "chat_id": chats::dm_chat_id(person.pk().as_hex(), None),
            "id": id,
            "from": person.pk().as_hex(),
            "d": "a:1",
            "created_at": 1_000_000,
            "historical": false,
        })]
    );
}

#[tokio::test]
async fn a_press_is_hidden_from_the_timeline_and_from_unread_counts() {
    let (bot, person, p) = bot_and_person().await;
    let chat = chats::dm_chat_id(person.pk().as_hex(), None);
    let before = bot.dm.messages(&chat, None, 50).await.unwrap().len();
    let unread = bot.dm.total_unread().await.unwrap();
    let pressed = person
        .dm
        .prepare_press(&person.keys, &p.message.id, "d:1")
        .await
        .unwrap();
    bot.receive(&press_event_of(&pressed)).await;
    assert_eq!(
        bot.dm.messages(&chat, None, 50).await.unwrap().len(),
        before,
        "no line of it"
    );
    assert_eq!(bot.dm.total_unread().await.unwrap(), unread);
    let c = bot.dm.chat(&chat).await.unwrap().unwrap();
    assert_eq!(
        c.last_preview.as_deref(),
        Some("Open wttr.in?"),
        "the list says what it said"
    );
    assert_eq!(
        repo::count_in_chat(&bot.dm.store, &chat).await.unwrap(),
        before as i64,
        "not kept even hidden"
    );
    // The presser's side: no line either.
    assert_eq!(
        person
            .dm
            .messages(&pressed.chat_id, None, 50)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn a_double_press_is_refused_until_30_s_or_an_edit() {
    let (bot, person, p) = bot_and_person().await;
    let id = p.message.id.clone();
    person
        .dm
        .prepare_press(&person.keys, &id, "a:1")
        .await
        .unwrap();
    for d in ["a:1", "d:1"] {
        assert_eq!(
            reason(
                person
                    .dm
                    .prepare_press(&person.keys, &id, d)
                    .await
                    .unwrap_err()
            ),
            "press_pending",
            "{d}"
        );
    }
    person
        .clock
        .0
        .fetch_add(PRESS_REPEAT_SECS - 1, Ordering::SeqCst);
    assert_eq!(
        reason(
            person
                .dm
                .prepare_press(&person.keys, &id, "a:1")
                .await
                .unwrap_err()
        ),
        "press_pending"
    );
    person.clock.0.fetch_add(1, Ordering::SeqCst);
    let again = person
        .dm
        .prepare_press(&person.keys, &id, "d:1")
        .await
        .unwrap();
    assert_eq!(
        again.message.pressed.unwrap().d,
        "d:1",
        "30 s on, the buttons are alive again"
    );
    drop(bot);
}

#[tokio::test]
async fn the_same_press_within_60_s_is_told_once() {
    let (bot, person, p) = bot_and_person().await;
    let id = p.message.id.clone();
    let first = person
        .dm
        .prepare_press(&person.keys, &id, "a:1")
        .await
        .unwrap();
    assert_eq!(
        presses(&bot.receive(&press_event_of(&first)).await).len(),
        1
    );
    // The same wrap again (a second relay), and a second press of the same
    // button 30 s later: told once.
    assert!(bot.receive(&press_event_of(&first)).await.is_empty());
    person
        .clock
        .0
        .fetch_add(PRESS_REPEAT_SECS, Ordering::SeqCst);
    bot.clock.0.fetch_add(PRESS_REPEAT_SECS, Ordering::SeqCst);
    let second = person
        .dm
        .prepare_press(&person.keys, &id, "a:1")
        .await
        .unwrap();
    assert!(
        bot.receive(&press_event_of(&second)).await.is_empty(),
        "within 60 s of the first"
    );
    // Another button is another press; and a minute after, the same one counts again.
    person
        .clock
        .0
        .fetch_add(PRESS_REPEAT_SECS, Ordering::SeqCst);
    let other = person
        .dm
        .prepare_press(&person.keys, &id, "d:1")
        .await
        .unwrap();
    assert_eq!(
        presses(&bot.receive(&press_event_of(&other)).await).len(),
        1
    );
    bot.clock.0.fetch_add(PRESS_DEDUP_SECS, Ordering::SeqCst);
    assert_eq!(
        presses(&bot.receive(&press_event_of(&second)).await).len(),
        1
    );
}

#[tokio::test]
async fn a_forged_press_is_dropped() {
    let (bot, person, p) = bot_and_person().await;
    let id = p.message.id.clone();
    let at = 1_000_100;
    let note = |from: &Keys, to: &Party, of: &str, d: &str| {
        crate::wrap::wrap_note(
            from,
            to.pk(),
            &Envelope::press(of, d).encode(),
            at,
            false,
            None,
        )
        .unwrap()
        .to_peer
    };
    // Of a message that is not here.
    assert!(
        bot.receive(&note(&person.keys, &bot, &"00".repeat(32), "a:1"))
            .await
            .is_empty(),
        "wrong of"
    );
    // Of a button the message does not have.
    assert!(
        bot.receive(&note(&person.keys, &bot, &id, "x:1"))
            .await
            .is_empty(),
        "wrong d"
    );
    // Malformed.
    let w = crate::wrap::wrap_note(
        &person.keys,
        bot.pk(),
        r#"{"v":1,"t":"press","of":"zz","d":"a:1"}"#,
        at,
        false,
        None,
    )
    .unwrap();
    assert!(bot.receive(&w.to_peer).await.is_empty(), "no id");
    // From a key that is not of the chat: the message is the bot's chat
    // with the person, not with Carol.
    let carol = Party::new().await;
    assert!(
        bot.receive(&note(&carol.keys, &bot, &id, "a:1"))
            .await
            .is_empty(),
        "from a non-participant"
    );
    // Of a message of the presser, not of the receiver's.
    let mine = person
        .dm
        .prepare_text(&person.keys, &bot.pk(), "my own", None)
        .await
        .unwrap();
    bot.receive(&peer_event(&mine)).await;
    assert!(
        bot.receive(&note(&person.keys, &bot, &mine.message.id, "a:1"))
            .await
            .is_empty(),
        "not authored by the receiver"
    );
    // Of a message of the bot in another chat.
    profile(&carol, &bot, true).await;
    let to_carol = bot
        .dm
        .prepare_text_with(
            &bot.keys,
            &carol.pk(),
            "yours",
            TextFormat::Plain,
            None,
            Some(&kb()),
        )
        .await
        .unwrap();
    assert!(
        bot.receive(&note(&person.keys, &bot, &to_carol.message.id, "a:1"))
            .await
            .is_empty(),
        "wrong chat"
    );
    // The real one is heard.
    assert_eq!(
        presses(&bot.receive(&note(&person.keys, &bot, &id, "a:1")).await).len(),
        1
    );
}

#[tokio::test]
async fn a_press_is_refused_where_it_makes_no_sense() {
    let (bot, person, p) = bot_and_person().await;
    // A button that is not there.
    assert_eq!(
        reason(
            person
                .dm
                .prepare_press(&person.keys, &p.message.id, "x")
                .await
                .unwrap_err()
        ),
        "press_no_button"
    );
    // My own message, a message without buttons, one that is not here.
    let mine = person
        .dm
        .prepare_text(&person.keys, &bot.pk(), "mine", None)
        .await
        .unwrap();
    assert_eq!(
        reason(
            person
                .dm
                .prepare_press(&person.keys, &mine.message.id, "a:1")
                .await
                .unwrap_err()
        ),
        "press_unknown"
    );
    let plain = bot
        .dm
        .prepare_text(&bot.keys, &person.pk(), "plain", None)
        .await
        .unwrap();
    person.receive(&peer_event(&plain)).await;
    assert_eq!(
        reason(
            person
                .dm
                .prepare_press(&person.keys, &plain.message.id, "a:1")
                .await
                .unwrap_err()
        ),
        "press_no_button"
    );
    assert_eq!(
        reason(
            person
                .dm
                .prepare_press(&person.keys, &"00".repeat(32), "a:1")
                .await
                .unwrap_err()
        ),
        "press_unknown"
    );
    // The bot cannot press its own.
    assert_eq!(
        reason(
            bot.dm
                .prepare_press(&bot.keys, &p.message.id, "a:1")
                .await
                .unwrap_err()
        ),
        "press_unknown"
    );
    // A deleted one.
    let d = bot
        .dm
        .prepare_delete(&bot.keys, &p.message.id)
        .await
        .unwrap();
    person.receive(&peer_event(&d)).await;
    assert_eq!(
        person.dm.message(&p.message.id).await.unwrap().unwrap().kb,
        None,
        "a deleted message has no buttons"
    );
    assert_eq!(
        reason(
            person
                .dm
                .prepare_press(&person.keys, &p.message.id, "a:1")
                .await
                .unwrap_err()
        ),
        "press_unknown"
    );
}

#[tokio::test]
async fn an_edit_clears_pressed_and_replaces_or_removes_the_keyboard() {
    let (bot, person, p) = bot_and_person().await;
    let id = p.message.id.clone();
    let pressed = person
        .dm
        .prepare_press(&person.keys, &id, "a:1")
        .await
        .unwrap();
    assert!(pressed.message.pressed.is_some());
    let other = Keyboard {
        rows: vec![vec![Button::data("Undo", "u:1")]],
    };
    bot.clock.0.fetch_add(1, Ordering::SeqCst);
    let e = bot
        .dm
        .prepare_edit_with(&bot.keys, &id, "Allowed.", TextFormat::Plain, Some(&other))
        .await
        .unwrap();
    assert_eq!(e.message.kb, Some(other.clone()));
    person.receive(&peer_event(&e)).await;
    let seen = person.dm.message(&id).await.unwrap().unwrap();
    assert_eq!(
        (seen.text.as_deref(), seen.kb.clone(), seen.pressed.clone()),
        (Some("Allowed."), Some(other), None)
    );
    // At once pressable again: the edit is the bot's answer.
    person
        .dm
        .prepare_press(&person.keys, &id, "u:1")
        .await
        .unwrap();
    bot.clock.0.fetch_add(1, Ordering::SeqCst);
    let e = bot.dm.prepare_edit(&bot.keys, &id, "Done.").await.unwrap();
    assert_eq!(
        e.message.kb, None,
        "an edit without kb takes the keyboard away"
    );
    person.receive(&peer_event(&e)).await;
    let seen = person.dm.message(&id).await.unwrap().unwrap();
    assert_eq!(
        (seen.text.as_deref(), seen.kb, seen.pressed),
        (Some("Done."), None, None)
    );
    assert_eq!(
        KbState::stored(row(&person, &id).await.media_json.as_deref()),
        KbState::default()
    );
    // A press after the buttons went: refused here, dropped there.
    assert_eq!(
        reason(
            person
                .dm
                .prepare_press(&person.keys, &id, "u:1")
                .await
                .unwrap_err()
        ),
        "press_no_button"
    );
    let late = crate::wrap::wrap_note(
        &person.keys,
        bot.pk(),
        &Envelope::press(&id, "a:1").encode(),
        1_000_200,
        false,
        None,
    )
    .unwrap();
    assert!(bot.receive(&late.to_peer).await.is_empty());
}

#[tokio::test]
async fn an_edit_by_another_key_cannot_change_the_buttons() {
    let (bot, person, p) = bot_and_person().await;
    let id = p.message.id.clone();
    let carol = Party::new().await;
    profile(&person, &carol, true).await;
    let fake = Keyboard {
        rows: vec![vec![Button::data("Pay", "pay")]],
    };
    for forger in [&carol.keys, &person.keys] {
        let content = Envelope::edit(&id, "hacked").with_kb(Some(&fake)).encode();
        let w = wrap(forger, person.pk(), &content, 1_000_100, None).unwrap();
        // Carol's lands in her own chat; my own copy speaks for me, not the bot.
        let _ = person.receive(&w.to_peer).await;
    }
    let seen = person.dm.message(&id).await.unwrap().unwrap();
    assert_eq!(
        (seen.text.as_deref(), seen.kb),
        (Some("Open wttr.in?"), Some(kb()))
    );
    // And I cannot edit the bot's message at all.
    assert!(person
        .dm
        .prepare_edit_with(&person.keys, &id, "hacked", TextFormat::Plain, Some(&fake))
        .await
        .is_err());
    drop(bot);
}

#[tokio::test]
async fn an_edit_with_buttons_that_overtakes_its_text_is_applied_on_arrival() {
    let bot = Party::new().await;
    let person = Party::new().await;
    profile(&person, &bot, true).await;
    let p = bot
        .dm
        .prepare_text(&bot.keys, &person.pk(), "first", None)
        .await
        .unwrap();
    bot.clock.0.fetch_add(1, Ordering::SeqCst);
    let e = bot
        .dm
        .prepare_edit_with(
            &bot.keys,
            &p.message.id,
            "second",
            TextFormat::Plain,
            Some(&kb()),
        )
        .await
        .unwrap();
    person.receive(&peer_event(&e)).await;
    person.receive(&peer_event(&p)).await;
    let seen = person.dm.message(&p.message.id).await.unwrap().unwrap();
    assert_eq!(
        (seen.text.as_deref(), seen.kb),
        (Some("second"), Some(kb()))
    );
}
