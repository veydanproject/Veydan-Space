// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The widths of the panes between my devices (`own.ui`): the whole map,
//! the later wins.

use super::*;
use crate::ui_prefs::{UiPrefs, UI_PREFS_EVERY_SECS};
use messenger_core::envelope::T_OWN_UI;

fn widths(p: &UiPrefs) -> Vec<(String, u32)> {
    p.widths.iter().map(|(k, v)| (k.clone(), *v)).collect()
}

#[tokio::test]
async fn a_width_set_here_is_kept_and_owed_once() {
    let alice = Party::new().await;
    assert_eq!(alice.dm.ui_prefs().await.unwrap(), UiPrefs::default());
    assert_eq!(alice.dm.ui_prefs_due(1_000_000).await.unwrap(), None, "nothing set, nothing to tell");

    for (key, px) in [("Bad", Some(400)), ("", Some(400)), ("chats.side", Some(199)), ("chats.side", Some(2001))] {
        assert_eq!(reason(alice.dm.set_ui_pref(key, px).await.unwrap_err()), "ui_pref_invalid", "{key} {px:?}");
    }

    let kept = alice.dm.set_ui_pref("channels.side", Some(420)).await.unwrap();
    assert_eq!((widths(&kept), kept.at), (vec![("channels.side".into(), 420)], 1_000_000));
    // Two changes in one second: still later each time, and one note for both.
    let kept = alice.dm.set_ui_pref("chats.side", Some(360)).await.unwrap();
    assert_eq!(kept.at, 1_000_001);
    assert_eq!(alice.dm.ui_prefs().await.unwrap(), kept, "kept in the store");
    let note = alice.dm.ui_prefs_due(1_000_000).await.unwrap().expect("owed");
    assert_eq!(note, Envelope::own_ui(&widths(&kept), 1_000_001));
    alice.dm.ui_prefs_sent(1_000_000).await.unwrap();
    assert_eq!(alice.dm.ui_prefs_due(1_000_002).await.unwrap(), None, "sent");

    // The same width again changes nothing and owes nothing.
    assert_eq!(alice.dm.set_ui_pref("chats.side", Some(360)).await.unwrap(), kept);
    assert_eq!(alice.dm.ui_prefs_due(1_000_002).await.unwrap(), None);

    // Forgotten: the pane takes its default; that is a change too.
    let kept = alice.dm.set_ui_pref("chats.side", None).await.unwrap();
    assert_eq!(widths(&kept), vec![("channels.side".into(), 420)]);
    assert!(alice.dm.ui_prefs_due(1_000_003).await.unwrap().is_some());
    alice.dm.ui_prefs_sent(1_000_003).await.unwrap();

    // Told again every few days.
    assert_eq!(alice.dm.ui_prefs_due(1_000_003 + UI_PREFS_EVERY_SECS - 1).await.unwrap(), None);
    assert!(alice.dm.ui_prefs_due(1_000_003 + UI_PREFS_EVERY_SECS).await.unwrap().is_some());
}

#[tokio::test]
async fn the_later_map_wins_whole_on_my_other_device() {
    let alice = Party::new().await;
    let laptop = Party::with_keys(alice.keys.clone()).await;
    laptop.dm.set_ui_pref("chats.side", Some(500)).await.unwrap();

    alice.clock.0.store(1_000_100, Ordering::SeqCst);
    let kept = alice.dm.set_ui_pref("channels.side", Some(420)).await.unwrap();
    let note = alice.dm.ui_prefs_due(1_000_100).await.unwrap().unwrap();
    let effects = laptop.receive(&own_note(&alice.keys, &note, 1_000_100)).await;
    assert_eq!(names(&effects), vec!["ui_prefs.updated"]);
    let Effect::Emit(ev) = &effects[0] else { panic!() };
    assert_eq!(ev.payload, serde_json::json!({ "w": { "channels.side": 420 } }), "the whole map, not a merge");
    assert_eq!(laptop.dm.ui_prefs().await.unwrap(), kept);
    assert!(laptop.receive(&own_note(&alice.keys, &note, 1_000_101)).await.is_empty(), "the same again is no news");
    assert_eq!(laptop.dm.ui_prefs_due(1_000_101).await.unwrap(), None, "a map heard is not owed back, the change it beat neither");

    // An older map changes nothing.
    let older = Envelope::own_ui(&[("channels.side".into(), 300)], 1_000_099);
    assert!(laptop.receive(&own_note(&alice.keys, &older, 1_000_102)).await.is_empty());
    assert_eq!(laptop.dm.ui_prefs().await.unwrap(), kept);

    // A map without a time is dropped; what is out of bounds in one is skipped.
    let no_time = Envelope::new(T_OWN_UI).with("w", serde_json::json!({ "channels.side": 600 }));
    assert!(laptop.receive(&own_note(&alice.keys, &no_time, 1_000_103)).await.is_empty());
    let junk = Envelope::new(T_OWN_UI).with("w", serde_json::json!({ "channels.side": 9000, "X": 400, "chats.side": 380 })).with("at", 1_000_200);
    assert_eq!(names(&laptop.receive(&own_note(&alice.keys, &junk, 1_000_104)).await), vec!["ui_prefs.updated"]);
    assert_eq!(widths(&laptop.dm.ui_prefs().await.unwrap()), vec![("chats.side".into(), 380)]);

    // Changed on the laptop with a clock behind: still the later.
    laptop.clock.0.store(900_000, Ordering::SeqCst);
    let back = laptop.dm.set_ui_pref("channels.side", Some(640)).await.unwrap();
    assert_eq!(back.at, 1_000_201);
    let note = laptop.dm.ui_prefs_due(900_000).await.unwrap().unwrap();
    assert_eq!(names(&alice.receive(&own_note(&alice.keys, &note, 1_000_300)).await), vec!["ui_prefs.updated"]);
    assert_eq!(alice.dm.ui_prefs().await.unwrap(), back);
}

#[tokio::test]
async fn two_maps_of_the_same_second_end_alike() {
    let alice = Party::new().await;
    let laptop = Party::with_keys(alice.keys.clone()).await;
    let a = alice.dm.set_ui_pref("chats.side", Some(400)).await.unwrap();
    let b = laptop.dm.set_ui_pref("chats.side", Some(500)).await.unwrap();
    assert_eq!(a.at, b.at);
    let (na, nb) = (alice.dm.ui_prefs_due(a.at).await.unwrap().unwrap(), laptop.dm.ui_prefs_due(b.at).await.unwrap().unwrap());
    alice.receive(&own_note(&alice.keys, &nb, b.at)).await;
    laptop.receive(&own_note(&alice.keys, &na, a.at)).await;
    assert_eq!(alice.dm.ui_prefs().await.unwrap(), laptop.dm.ui_prefs().await.unwrap());
}
