// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The buttons of a bot in a group (platform-spec §9.7,
//! the rules of the buttons (sdk/docs/bridge.md, «Buttons under a message»)): the press goes into the group,
//! sealed as a reaction is; the author hears it with the key of whoever
//! pressed, every other member drops it.

use super::*;
use messenger_core::keyboard::{Button, Keyboard};
use messenger_core::Envelope;
use messenger_store::messages as msgs;

fn kb() -> Keyboard {
    Keyboard {
        rows: vec![vec![Button::data("Yes", "y"), Button::data("No", "n")]],
    }
}

/// A group of a bot (device 0) with two members (1, 2) who know it is a
/// bot, and the bot's question with buttons, delivered to all.
async fn group_with_a_bot() -> (World, String, String) {
    let mut w = World::new();
    let bot = w.person().await;
    let bob = w.person().await;
    let carol = w.person().await;
    let g = w.create(bot, GroupKind::Private, "Polls", true).await;
    w.bring(bot, &g, bob).await;
    w.bring(bot, &g, carol).await;
    for who in [bob, carol] {
        let pk = w.pk(bot);
        w.devices[who]
            .profiles
            .apply_event(&pk, Timestamp(10), r#"{"name":"Bot","bot":true}"#)
            .await
            .unwrap();
    }
    w.tick();
    let d = &w.devices[bot];
    let (m, out) = d
        .svc
        .prepare_text_with(
            &d.keys,
            &g,
            "Pizza tonight?",
            messenger_core::TextFormat::Plain,
            None,
            Some(&kb()),
        )
        .await
        .unwrap();
    assert_eq!(m.kb, Some(kb()));
    w.run(
        bot,
        Outcome {
            publish: vec![out],
            ..Default::default()
        },
    )
    .await;
    (w, g, m.id)
}

/// `who` presses `d` under `id`: what goes to the relay.
async fn press(w: &World, who: usize, id: &str, d: &str) -> WireEvent {
    let dev = &w.devices[who];
    let (chat, out) = dev.svc.prepare_press(&dev.keys, id, d).await.unwrap();
    assert!(chat.starts_with("group:"));
    let Outbound::PublishScoped { event, .. } = out else {
        panic!("{out:?}")
    };
    assert!(
        event.json["tags"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t[0] == pushtags::SILENT),
        "a press wakes nobody"
    );
    event
}

fn presses(o: &Outcome) -> Vec<serde_json::Value> {
    o.events
        .iter()
        .filter(|e| e.name == UI_EVENT_GROUP_PRESS)
        .map(|e| e.payload.clone())
        .collect()
}

/// A press sealed with the group's key by `who`, whatever the group says of them.
async fn forged_press(w: &World, who: usize, group: &str, of: &str, d: &str) -> WireEvent {
    let svc = w.devices[0].svc.clone();
    let key_id = svc
        .need_log(group)
        .await
        .unwrap()
        .state()
        .current_key
        .clone()
        .unwrap();
    let key = svc.key(group, &key_id).await.unwrap().unwrap();
    let author = &w.devices[who].keys;
    let signed = crate::wire::sign_message(
        author,
        group,
        &Envelope::press(of, d).encode(),
        w.clock.now().secs(),
        None,
    )
    .unwrap();
    crate::wire::seal_note(group, &key, None, &signed, author).unwrap()
}

#[tokio::test]
async fn a_member_sees_the_buttons_of_a_bot_and_the_bot_alone_hears_the_press() {
    let (w, g, id) = group_with_a_bot().await;
    let (bot, bob, carol) = (0, 1, 2);
    for who in [bob, carol] {
        let seen = w.devices[who]
            .visible(&g)
            .await
            .into_iter()
            .find(|m| m.id == id)
            .unwrap();
        assert_eq!(seen.kb, Some(kb()), "device {who}");
    }
    let before = counts(&w, &g).await;
    let event = press(&w, bob, &id, "y").await;
    let shown = w.devices[bob]
        .visible(&g)
        .await
        .into_iter()
        .find(|m| m.id == id)
        .unwrap();
    assert_eq!(shown.pressed.map(|p| p.d), Some("y".into()));

    let heard = w.deliver(bot, &Wire::Group(event.clone()), false).await;
    let p = presses(&heard);
    assert_eq!(p.len(), 1, "{:?}", heard.events);
    assert_eq!(p[0]["group_id"], g.as_str());
    assert_eq!(p[0]["chat_id"], format!("group:{g}"));
    assert_eq!(p[0]["id"], id.as_str());
    assert_eq!(
        p[0]["from"],
        w.pk(bob).as_hex(),
        "the key of whoever pressed"
    );
    assert_eq!(p[0]["d"], "y");
    assert!(heard.notify.is_empty(), "no notification");
    // Another member, and the presser's own echo: nothing at all.
    for who in [carol, bob] {
        let o = w.deliver(who, &Wire::Group(event.clone()), false).await;
        assert!(
            presses(&o).is_empty() && o.notify.is_empty(),
            "device {who}"
        );
    }
    // Hidden from every timeline and every count.
    assert_eq!(counts(&w, &g).await, before, "no line of it anywhere");
    // The same press again: told once.
    assert!(presses(&w.deliver(bot, &Wire::Group(event), false).await).is_empty());
}

/// The messages of the group on each device, and the unread of each.
async fn counts(w: &World, g: &str) -> Vec<usize> {
    let mut out = Vec::new();
    for d in &w.devices {
        out.push(d.visible(g).await.len());
        out.push(
            d.dm.chat(&format!("group:{g}"))
                .await
                .unwrap()
                .map(|c| c.unread as usize)
                .unwrap_or(0),
        );
        out.push(
            msgs::count_in_chat(d.dm.store(), &format!("group:{g}"))
                .await
                .unwrap() as usize,
        );
    }
    out
}

#[tokio::test]
async fn a_forged_press_in_a_group_is_dropped() {
    let (mut w, g, id) = group_with_a_bot().await;
    let (bot, bob, carol) = (0, 1, 2);
    // A button the message does not have; a message that is not here.
    let nothing = "00".repeat(32);
    for (of, d) in [(id.as_str(), "maybe"), (nothing.as_str(), "y")] {
        let e = forged_press(&w, bob, &g, of, d).await;
        assert!(
            presses(&w.deliver(bot, &Wire::Group(e), false).await).is_empty(),
            "{of} {d}"
        );
    }
    // A message of Bob's: the bot is not its author.
    let mine = w.say(bob, &g, "mine").await;
    let e = forged_press(&w, carol, &g, &mine, "y").await;
    assert!(presses(&w.deliver(bot, &Wire::Group(e.clone()), false).await).is_empty());
    assert!(
        presses(&w.deliver(bob, &Wire::Group(e), false).await).is_empty(),
        "Bob's message has no buttons"
    );
    // From one who is not a member: held as anything an outsider says, never told.
    let dave = w.person().await;
    let e = forged_press(&w, dave, &g, &id, "y").await;
    assert!(
        presses(&w.deliver(bot, &Wire::Group(e), false).await).is_empty(),
        "from a non-participant"
    );
    // Of a message of the bot in another group of it.
    let g2 = w.create(bot, GroupKind::Private, "Other", true).await;
    w.bring(bot, &g2, carol).await;
    w.tick();
    let d = &w.devices[bot];
    let (other, out) = d
        .svc
        .prepare_text_with(
            &d.keys,
            &g2,
            "elsewhere",
            messenger_core::TextFormat::Plain,
            None,
            Some(&kb()),
        )
        .await
        .unwrap();
    w.run(
        bot,
        Outcome {
            publish: vec![out],
            ..Default::default()
        },
    )
    .await;
    let e = forged_press(&w, carol, &g, &other.id, "y").await;
    assert!(
        presses(&w.deliver(bot, &Wire::Group(e), false).await).is_empty(),
        "wrong chat"
    );
    // The real one is heard.
    let e = forged_press(&w, carol, &g, &id, "n").await;
    assert_eq!(
        presses(&w.deliver(bot, &Wire::Group(e), false).await).len(),
        1
    );
}

#[tokio::test]
async fn an_edit_in_a_group_clears_pressed_and_takes_the_buttons_away() {
    let (mut w, g, id) = group_with_a_bot().await;
    let (bot, bob) = (0, 1);
    let event = press(&w, bob, &id, "n").await;
    w.deliver(bot, &Wire::Group(event), false).await;
    let d = &w.devices[bob];
    assert_eq!(
        code(d.svc.prepare_press(&d.keys, &id, "y").await),
        "press_pending"
    );
    w.tick();
    let d = &w.devices[bot];
    let (_, _, out) = d
        .svc
        .prepare_edit_with(
            &d.keys,
            &id,
            "No pizza.",
            messenger_core::TextFormat::Plain,
            None,
        )
        .await
        .unwrap();
    w.run(
        bot,
        Outcome {
            publish: vec![out],
            ..Default::default()
        },
    )
    .await;
    for who in [bot, bob] {
        let m = w.devices[who]
            .visible(&g)
            .await
            .into_iter()
            .find(|m| m.id == id)
            .unwrap();
        assert_eq!(
            (m.text.as_deref(), m.kb, m.pressed),
            (Some("No pizza."), None, None),
            "device {who}"
        );
    }
    let d = &w.devices[bob];
    assert_eq!(
        code(d.svc.prepare_press(&d.keys, &id, "y").await),
        "press_no_button"
    );
}

#[tokio::test]
async fn kb_from_a_person_in_a_group_is_dropped_and_an_edit_by_another_key_changes_nothing() {
    let (mut w, g, id) = group_with_a_bot().await;
    let (bob, carol) = (1, 2);
    let pk = w.pk(bob);
    w.devices[carol]
        .profiles
        .apply_event(&pk, Timestamp(10), r#"{"name":"Bob"}"#)
        .await
        .unwrap();
    w.tick();
    let d = &w.devices[bob];
    let (m, out) = d
        .svc
        .prepare_text_with(
            &d.keys,
            &g,
            "vote",
            messenger_core::TextFormat::Plain,
            None,
            Some(&kb()),
        )
        .await
        .unwrap();
    w.run(
        bob,
        Outcome {
            publish: vec![out],
            ..Default::default()
        },
    )
    .await;
    let row = msgs::get(w.devices[carol].dm.store(), &m.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.media_json, None, "a person's buttons are kept nowhere");
    let d = &w.devices[carol];
    assert_eq!(
        code(d.svc.prepare_press(&d.keys, &m.id, "y").await),
        "press_no_button"
    );
    // Bob cannot put buttons under the bot's message.
    let d = &w.devices[bob];
    assert!(d
        .svc
        .prepare_edit_with(
            &d.keys,
            &id,
            "hacked",
            messenger_core::TextFormat::Plain,
            Some(&kb())
        )
        .await
        .is_err());
    let svc = w.devices[0].svc.clone();
    let key_id = svc
        .need_log(&g)
        .await
        .unwrap()
        .state()
        .current_key
        .clone()
        .unwrap();
    let key = svc.key(&g, &key_id).await.unwrap().unwrap();
    let fake = Keyboard {
        rows: vec![vec![Button::data("Pay", "pay")]],
    };
    let content = Envelope::edit(&id, "hacked").with_kb(Some(&fake)).encode();
    let signed = crate::wire::sign_message(
        &w.devices[bob].keys,
        &g,
        &content,
        w.clock.now().secs(),
        None,
    )
    .unwrap();
    let sealed = crate::wire::seal_message(&g, &key, None, &signed, &w.devices[bob].keys).unwrap();
    w.deliver(carol, &Wire::Group(sealed), false).await;
    let seen = w.devices[carol]
        .visible(&g)
        .await
        .into_iter()
        .find(|m| m.id == id)
        .unwrap();
    assert_eq!(
        (seen.text.as_deref(), seen.kb),
        (Some("Pizza tonight?"), Some(kb()))
    );
}
