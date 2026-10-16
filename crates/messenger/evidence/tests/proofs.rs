// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Proofs made of what the app really sends, and every forgery we could
//! think of: a report is only worth something if none of them passes.
//!
//! Alice is the accused, Bob reports, Carol is a third person, Mallory a
//! modified client.

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use messenger_core::envelope::KIND_PEER_NOTE_RUMOR;
use messenger_core::{Envelope, PubKey};
use messenger_evidence::nip44::{message_keys, Payload};
use messenger_evidence::{
    dm_proof, group_proof, verify, Attached, Context, Error, MessageKeys, Outcome, Proof, Reason,
    Report, Target, Verified,
};
use nostr::nips::nip44::v2::ConversationKey;
use nostr::nips::nip59::GiftWrapSealBuilder;
use nostr::prelude::*;

const GROUP: &str = "c0ffee00c0ffee00c0ffee00c0ffee00";

fn now() -> i64 {
    Timestamp::now().as_secs() as i64
}

fn pk(keys: &Keys) -> PubKey {
    PubKey::parse(&keys.public_key().to_hex()).unwrap()
}

/// A message as the app sends it: both wraps, the peer's and my own copy.
fn send(from: &Keys, to: &Keys, envelope: &Envelope, at: i64) -> (Event, Event, String) {
    let w = messenger_dm::wrap::wrap(from, &pk(to), &envelope.encode(), at, None).unwrap();
    let peer: Event = serde_json::from_value(w.to_peer.json).unwrap();
    let mine: Event = serde_json::from_value(w.to_self.expect("a copy for me").json).unwrap();
    (peer, mine, w.rumor_id.as_hex().to_string())
}

fn text(t: &str) -> Envelope {
    Envelope::text(t)
}

fn report(reporter: &Keys, accused: &Keys, context: Context, proofs: Vec<Proof>) -> Report {
    Report {
        target: Target::Person,
        group_name: None,
        group_admins: Vec::new(),
        message_id: None,
        v: 1,
        reporter: reporter.public_key().to_hex(),
        accused: accused.public_key().to_hex(),
        reason: Reason::Harassment,
        sub_reason: Some("threats".into()),
        comment: Some("He keeps writing at night.".into()),
        context,
        messages: proofs
            .into_iter()
            .map(|p| Attached {
                proof: Some(p),
                note: None,
            })
            .collect(),
        created_at: now(),
    }
}

fn one(reporter: &Keys, accused: &Keys, proof: Proof) -> Outcome {
    let v = verify(&report(reporter, accused, Context::Dm, vec![proof]), now());
    assert!(v.accepted(), "{v:?}");
    v.messages.into_iter().next().unwrap()
}

fn unverified(outcome: &Outcome) -> &str {
    match outcome {
        Outcome::Unverified(why) => why,
        Outcome::Verified(m) => panic!("a forgery verified: {m:?}"),
    }
}

/// A proof Alice → Bob that Bob made from the wrap he received.
fn alice_to_bob(alice: &Keys, bob: &Keys, t: &str) -> (Proof, String) {
    let (to_bob, _, id) = send(alice, bob, &text(t), now());
    (dm_proof(bob, &to_bob).unwrap(), id)
}

/// The keys of a seal's payload under the conversation of `secret` and
/// `public`: what a forger who holds one side of a conversation can make.
fn keys_of(seal: &Event, secret: &Keys, public: &PublicKey) -> MessageKeys {
    let payload = Payload::parse(&seal.content).unwrap();
    let conversation = ConversationKey::derive(secret.secret_key(), public).unwrap();
    message_keys(conversation.as_bytes().try_into().unwrap(), &payload.nonce)
}

/// A seal `author` signs of any rumor whatever, to `to`.
fn seal(author: &Keys, to: &PublicKey, rumor: UnsignedEvent) -> Event {
    GiftWrapSealBuilder::new(rumor, *to)
        .finalize(author)
        .unwrap()
}

fn rumor(author: &PublicKey, to: &PublicKey, content: &str, at: i64) -> UnsignedEvent {
    EventBuilder::new(Kind::PrivateDirectMessage, content)
        .tag(Tag::public_key(*to))
        .custom_created_at(Timestamp::from_secs(at as u64))
        .finalize_unsigned(*author)
}

// ─── What verifies ──────────────────────────────────────────────────────────

#[test]
fn a_dm_of_the_accused_verifies() {
    let (alice, bob) = (Keys::generate(), Keys::generate());
    let envelope = text("you will regret this");
    let (to_bob, _, id) = send(&alice, &bob, &envelope, now());
    let proof = dm_proof(&bob, &to_bob).unwrap();
    match one(&bob, &alice, proof) {
        Outcome::Verified(m) => {
            assert_eq!(m.author, alice.public_key().to_hex());
            assert_eq!(m.content, envelope.encode());
            assert_eq!(m.kind, 14);
            assert_eq!(m.id, id, "the id is the rumor's, as the app keeps it");
            assert!(m
                .tags
                .iter()
                .any(|t| t == &vec!["p".to_string(), bob.public_key().to_hex()]));
            assert!(m.media_json.is_empty());
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn my_own_message_verifies_as_mine() {
    let (alice, bob) = (Keys::generate(), Keys::generate());
    // Bob's answer, from Bob's own copy: context for the moderator.
    let (_, bobs_copy, _) = send(&bob, &alice, &text("leave me alone"), now());
    let proof = dm_proof(&bob, &bobs_copy).unwrap();
    match one(&bob, &alice, proof) {
        Outcome::Verified(m) => assert_eq!(m.author, bob.public_key().to_hex()),
        other => panic!("{other:?}"),
    }
}

#[test]
fn an_attachment_shows_its_descriptor() {
    let (alice, bob) = (Keys::generate(), Keys::generate());
    let media = Envelope::new("media")
        .with("kind", "image")
        .with("name", "x.jpg")
        .with("mime", "image/jpeg")
        .with("size", 10)
        .with("sha256", "ab".repeat(32))
        .with(
            "chunks",
            serde_json::json!([{"sha256": "cd".repeat(32), "size": 26}]),
        )
        .with("key", B64.encode([1u8; 32]))
        .with("iv", B64.encode([2u8; 12]))
        .with("servers", serde_json::json!(["https://media.example"]))
        .with("local_path", "/home/alice/x.jpg");
    let (to_bob, _, _) = send(&alice, &bob, &media, now());
    match one(&bob, &alice, dm_proof(&bob, &to_bob).unwrap()) {
        Outcome::Verified(m) => {
            assert_eq!(m.media_json.len(), 1);
            let d = &m.media_json[0];
            assert_eq!(d["sha256"], "ab".repeat(32));
            assert_eq!(d["servers"][0], "https://media.example");
            assert!(
                d.get("local_path").is_none(),
                "a sender's local field is not shown"
            );
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_group_message_verifies() {
    let (alice, bob) = (Keys::generate(), Keys::generate());
    let inner = messenger_groups::wire::sign_message(
        &alice,
        GROUP,
        &text("spam spam").encode(),
        now(),
        None,
    )
    .unwrap();
    let proof = group_proof(&serde_json::to_value(&inner).unwrap()).unwrap();
    let v = verify(
        &report(
            &bob,
            &alice,
            Context::Group {
                group_id: GROUP.into(),
            },
            vec![proof],
        ),
        now(),
    );
    match &v.messages[0] {
        Outcome::Verified(m) => {
            assert_eq!(m.author, alice.public_key().to_hex());
            assert_eq!(m.kind, 9);
            assert_eq!(m.id, inner.id.to_hex());
            assert_eq!(m.content, text("spam spam").encode());
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_report_survives_the_wire() {
    let (alice, bob) = (Keys::generate(), Keys::generate());
    let (dm, _) = alice_to_bob(&alice, &bob, "hello");
    let inner =
        messenger_groups::wire::sign_message(&alice, GROUP, &text("hi all").encode(), now(), None)
            .unwrap();
    let group = group_proof(&serde_json::to_value(&inner).unwrap()).unwrap();
    let mut r = report(
        &bob,
        &alice,
        Context::Group {
            group_id: GROUP.into(),
        },
        vec![dm, group],
    );
    r.messages.push(Attached {
        proof: None,
        note: Some("wrap_gone".into()),
    });

    let json = serde_json::to_string(&r).unwrap();
    let back: Report = serde_json::from_str(&json).unwrap();
    assert_eq!(back, r);
    let wire: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(wire["context"]["t"], "group");
    assert_eq!(wire["reason"], "harassment");
    assert_eq!(wire["messages"][0]["proof"]["t"], "dm");
    assert_eq!(
        wire["messages"][0]["proof"]["keys"]["chacha_nonce"]
            .as_str()
            .unwrap()
            .len(),
        24
    );
    assert_eq!(wire["messages"][1]["proof"]["t"], "group");

    let v = verify(&back, now());
    assert!(v.accepted());
    assert_eq!(v.proved().count(), 2);
    assert_eq!(v.messages[2], Outcome::Unverified("no_proof".into()));
}

// ─── Forgeries ──────────────────────────────────────────────────────────────

/// (a) One byte of the ciphertext changed: the signature no longer covers it.
#[test]
fn forged_ciphertext() {
    let (alice, bob) = (Keys::generate(), Keys::generate());
    let (proof, _) = alice_to_bob(&alice, &bob, "I owe you 10 euros");
    let Proof::Dm { mut seal, keys } = proof else {
        unreachable!()
    };
    let mut raw = B64.decode(&seal.content).unwrap();
    raw[40] ^= 0x01;
    seal.content = B64.encode(&raw);
    assert_eq!(
        unverified(&one(&bob, &alice, Proof::Dm { seal, keys })),
        "seal_signature"
    );
}

/// (b) Bob signs a seal himself and puts words in Alice's mouth: once with
/// a rumor he wrote in her name, once with her real ciphertext.
#[test]
fn seal_resigned_by_the_reporter() {
    let (alice, bob) = (Keys::generate(), Keys::generate());
    let fake = rumor(
        &alice.public_key(),
        &bob.public_key(),
        &text("I will kill you").encode(),
        now(),
    );
    let by_bob = seal(&bob, &bob.public_key(), fake);
    let keys = keys_of(&by_bob, &bob, &bob.public_key());
    assert_eq!(
        unverified(&one(&bob, &alice, Proof::Dm { seal: by_bob, keys })),
        "rumor_author"
    );

    let (real, _) = alice_to_bob(&alice, &bob, "see you");
    let Proof::Dm {
        seal: real_seal,
        keys,
    } = real
    else {
        unreachable!()
    };
    let copied = EventBuilder::new(Kind::Seal, real_seal.content.clone())
        .finalize(&bob)
        .unwrap();
    assert_eq!(
        unverified(&one(&bob, &alice, Proof::Dm { seal: copied, keys })),
        "rumor_author"
    );
}

/// (c) A real message of Alice to Carol, handed by Carol to Bob: Alice did
/// write it, but not to Bob.
#[test]
fn a_message_to_somebody_else() {
    let (alice, bob, carol) = (Keys::generate(), Keys::generate(), Keys::generate());
    let (to_carol, _, _) = send(&alice, &carol, &text("for Carol only"), now());
    let proof = dm_proof(&carol, &to_carol).unwrap();
    assert_eq!(unverified(&one(&bob, &alice, proof)), "rumor_recipient");
}

/// (d) Another ChaCha20 key: the MAC holds, the plaintext is noise.
#[test]
fn wrong_chacha_key() {
    let (alice, bob) = (Keys::generate(), Keys::generate());
    for byte in 0..8 {
        let (proof, _) = alice_to_bob(&alice, &bob, "hello there");
        let Proof::Dm { seal, mut keys } = proof else {
            unreachable!()
        };
        keys.chacha_key[byte] ^= 0x80;
        let why = unverified(&one(
            &bob,
            &alice,
            Proof::Dm {
                seal: seal.clone(),
                keys,
            },
        ))
        .to_string();
        assert!(
            ["payload_padding", "rumor_utf8", "rumor_json"].contains(&why.as_str()),
            "{why}"
        );
        // The nonce of the cipher is as much a part of the key.
        let (proof, _) = alice_to_bob(&alice, &bob, "hello there");
        let Proof::Dm { seal, mut keys } = proof else {
            unreachable!()
        };
        keys.chacha_nonce[byte] ^= 0x80;
        assert!(matches!(
            one(&bob, &alice, Proof::Dm { seal, keys }),
            Outcome::Unverified(_)
        ));
    }
}

/// (e) Another HMAC key: the MAC fails before anything is decrypted.
#[test]
fn wrong_hmac_key() {
    let (alice, bob) = (Keys::generate(), Keys::generate());
    let (proof, _) = alice_to_bob(&alice, &bob, "hello");
    let Proof::Dm { seal, mut keys } = proof else {
        unreachable!()
    };
    keys.hmac_key[31] ^= 1;
    assert_eq!(
        unverified(&one(&bob, &alice, Proof::Dm { seal, keys })),
        "payload_mac"
    );
}

/// (f) Mallory seals a rumor in Alice's name and sends it to Bob.
#[test]
fn rumor_in_somebody_elses_name() {
    let (alice, bob, mallory) = (Keys::generate(), Keys::generate(), Keys::generate());
    let fake = rumor(
        &alice.public_key(),
        &bob.public_key(),
        &text("Alice said so").encode(),
        now(),
    );
    let by_mallory = seal(&mallory, &bob.public_key(), fake);
    let keys = keys_of(&by_mallory, &bob, &mallory.public_key());
    // Reported as Alice's: the seal is not hers.
    assert_eq!(
        unverified(&one(
            &bob,
            &alice,
            Proof::Dm {
                seal: by_mallory.clone(),
                keys: keys.clone()
            }
        )),
        "seal_author"
    );
    // Reported as Mallory's: the rumor is not his.
    assert_eq!(
        unverified(&one(
            &bob,
            &mallory,
            Proof::Dm {
                seal: by_mallory.clone(),
                keys
            }
        )),
        "rumor_author"
    );

    // The app's own wrap of it does not even make a proof.
    let wrap = GiftWrapBuilder::new(
        bob.public_key(),
        rumor(&alice.public_key(), &bob.public_key(), "x", now()),
    )
    .finalize(&mallory)
    .unwrap();
    assert!(matches!(dm_proof(&bob, &wrap), Err(Error::Rumor(_))));
}

/// (g) A group message of another group.
#[test]
fn group_message_of_another_group() {
    let (alice, bob) = (Keys::generate(), Keys::generate());
    let inner = messenger_groups::wire::sign_message(
        &alice,
        "another-group",
        &text("hi").encode(),
        now(),
        None,
    )
    .unwrap();
    let proof = group_proof(&serde_json::to_value(&inner).unwrap()).unwrap();
    let v = verify(
        &report(
            &bob,
            &alice,
            Context::Group {
                group_id: GROUP.into(),
            },
            vec![proof.clone()],
        ),
        now(),
    );
    assert_eq!(unverified(&v.messages[0]), "inner_group");
    // And no group message counts outside a group.
    assert_eq!(unverified(&one(&bob, &alice, proof)), "group_context");
}

/// (h) A group message signed by somebody else than the two.
#[test]
fn group_message_of_a_third() {
    let (alice, bob, carol) = (Keys::generate(), Keys::generate(), Keys::generate());
    let inner =
        messenger_groups::wire::sign_message(&carol, GROUP, &text("hi").encode(), now(), None)
            .unwrap();
    let proof = group_proof(&serde_json::to_value(&inner).unwrap()).unwrap();
    let v = verify(
        &report(
            &bob,
            &alice,
            Context::Group {
                group_id: GROUP.into(),
            },
            vec![proof],
        ),
        now(),
    );
    assert_eq!(unverified(&v.messages[0]), "inner_author");

    // Nor one whose content was changed after Alice signed it.
    let mut changed =
        messenger_groups::wire::sign_message(&alice, GROUP, &text("hi").encode(), now(), None)
            .unwrap();
    changed.content = text("something worse").encode();
    let v = verify(
        &report(
            &bob,
            &alice,
            Context::Group {
                group_id: GROUP.into(),
            },
            vec![Proof::Group {
                inner: changed.clone(),
            }],
        ),
        now(),
    );
    assert_eq!(unverified(&v.messages[0]), "inner_signature");
    assert!(matches!(
        group_proof(&serde_json::to_value(&changed).unwrap()),
        Err(Error::Group(_))
    ));
}

/// A report on a whole group: any member's signed message counts, still
/// under its own author; a DM is no evidence about a group; the claims of
/// the report (name, admins, the message it was opened on) are checked for
/// their form only.
#[test]
fn a_report_on_a_group() {
    let (owner, bob, carol) = (Keys::generate(), Keys::generate(), Keys::generate());
    let group = Context::Group {
        group_id: GROUP.into(),
    };
    let by_carol = messenger_groups::wire::sign_message(
        &carol,
        GROUP,
        &text("buy here").encode(),
        now(),
        None,
    )
    .unwrap();
    let carols = group_proof(&serde_json::to_value(&by_carol).unwrap()).unwrap();

    // On a person (the owner), Carol's message is not theirs.
    let on_person = report(&bob, &owner, group.clone(), vec![carols.clone()]);
    assert_eq!(
        unverified(&verify(&on_person, now()).messages[0]),
        "inner_author"
    );

    // On the group, it is evidence, and says whose it is.
    let mut on_group = report(&bob, &owner, group.clone(), vec![carols.clone()]);
    on_group.target = Target::Group;
    on_group.group_name = Some("Market".into());
    on_group.group_admins = vec![owner.public_key().to_hex(), carol.public_key().to_hex()];
    on_group.message_id = Some(by_carol.id.to_hex());
    let v = verify(&on_group, now());
    assert!(v.accepted(), "{v:?}");
    match &v.messages[0] {
        Outcome::Verified(m) => assert_eq!(m.author, carol.public_key().to_hex()),
        other => panic!("{other:?}"),
    }
    let back: Report = serde_json::from_str(&serde_json::to_string(&on_group).unwrap()).unwrap();
    assert_eq!(back, on_group);

    // A report on a person reads as it did before: no `target` on the wire.
    let json = serde_json::to_value(&on_person).unwrap();
    assert!(json.get("target").is_none() && json.get("group_admins").is_none());

    // A DM attached to a report on a group proves nothing about the group.
    let (dm, _) = alice_to_bob(&owner, &bob, "join us");
    let mut with_dm = on_group.clone();
    with_dm.messages = vec![Attached {
        proof: Some(dm),
        note: None,
    }];
    assert_eq!(
        unverified(&verify(&with_dm, now()).messages[0]),
        "dm_in_group_report"
    );

    // The claims, by their form.
    let refused = |r: &Report| verify(r, now()).refused;
    let mut not_in_group = on_group.clone();
    not_in_group.context = Context::Dm;
    assert_eq!(refused(&not_in_group).as_deref(), Some("target_context"));
    let mut bad_admin = on_group.clone();
    bad_admin.group_admins = vec!["XYZ".into()];
    assert_eq!(refused(&bad_admin).as_deref(), Some("group_admins"));
    let mut long_name = on_group.clone();
    long_name.group_name = Some("x".repeat(129));
    assert_eq!(refused(&long_name).as_deref(), Some("group_name_too_long"));
    let mut bad_id = on_group.clone();
    bad_id.message_id = Some("ABC".into());
    assert_eq!(refused(&bad_id).as_deref(), Some("message_id"));
}

/// (i) Eleven messages.
#[test]
fn too_many_messages() {
    let (alice, bob) = (Keys::generate(), Keys::generate());
    let proofs: Vec<Proof> = (0..10)
        .map(|i| alice_to_bob(&alice, &bob, &format!("m{i}")).0)
        .collect();
    let mut r = report(&bob, &alice, Context::Dm, proofs);
    let v = verify(&r, now());
    assert!(v.accepted());
    assert_eq!(v.proved().count(), 10);
    r.messages.push(Attached {
        proof: None,
        note: None,
    });
    assert_eq!(
        verify(&r, now()).refused.as_deref(),
        Some("too_many_messages")
    );
}

/// (j) Reporting oneself, also under another spelling of one's key.
#[test]
fn reporter_is_the_accused() {
    let (alice, bob) = (Keys::generate(), Keys::generate());
    let (proof, _) = alice_to_bob(&alice, &bob, "hi");
    assert_eq!(
        verify(&report(&bob, &bob, Context::Dm, vec![proof.clone()]), now())
            .refused
            .as_deref(),
        Some("self_report")
    );
    let mut r = report(&bob, &alice, Context::Dm, vec![proof]);
    r.accused = bob.public_key().to_hex().to_uppercase();
    assert_eq!(verify(&r, now()).refused.as_deref(), Some("accused_key"));
}

/// (k) Garbage of every sort: refused, never a panic.
#[test]
fn garbage_does_not_panic() {
    let (alice, bob) = (Keys::generate(), Keys::generate());
    for junk in [
        "",
        "{}",
        "null",
        "[]",
        "{\"v\":1}",
        "{\"t\":\"dm\"}",
        "\u{0}",
        "{\"v\":1,\"messages\":[{\"proof\":{\"t\":\"dm\"}}]}",
    ] {
        assert!(serde_json::from_str::<Report>(junk).is_err(), "{junk:?}");
    }

    // A seal signed by Alice for real, whose content is anything but a payload.
    let huge = "A".repeat(1 << 20);
    let long_but_fine = B64.encode(vec![2u8; 4096]);
    for content in [
        "",
        "#",
        "AA==",
        "not base64 at all",
        "Ag==",
        long_but_fine.as_str(),
        huge.as_str(),
    ] {
        let signed = EventBuilder::new(Kind::Seal, content)
            .finalize(&alice)
            .unwrap();
        let keys = MessageKeys {
            chacha_key: [0; 32],
            chacha_nonce: [0; 12],
            hmac_key: [0; 32],
        };
        let why = unverified(&one(&bob, &alice, Proof::Dm { seal: signed, keys })).to_string();
        assert!(why.starts_with("payload_") || why == "too_large", "{why}");
    }

    // A group message of 1 MiB.
    let big = EventBuilder::new(Kind::from(9u16), "x".repeat(1 << 20))
        .tag(Tag::parse(["h", GROUP]).unwrap())
        .finalize(&alice)
        .unwrap();
    let v = verify(
        &report(
            &bob,
            &alice,
            Context::Group {
                group_id: GROUP.into(),
            },
            vec![Proof::Group { inner: big }],
        ),
        now(),
    );
    assert_eq!(unverified(&v.messages[0]), "too_large");

    // The opener on random bytes of every length.
    let mut seed = 0x9e37_79b9_7f4a_7c15u64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for len in 0..600 {
        let mut raw: Vec<u8> = (0..len).map(|_| next() as u8).collect();
        if let Some(first) = raw.first_mut() {
            *first = 2;
        }
        let keys = MessageKeys {
            chacha_key: [3; 32],
            chacha_nonce: [4; 12],
            hmac_key: [5; 32],
        };
        if let Ok(p) = Payload::from_bytes(&raw) {
            assert!(p.open(&keys).is_err());
        }
    }
}

/// Every one-character change of a report on the wire: it parses or not,
/// and what verifies is still exactly what Alice wrote.
#[test]
fn mutated_reports_never_verify_something_else() {
    let (alice, bob) = (Keys::generate(), Keys::generate());
    let (proof, _) = alice_to_bob(&alice, &bob, "the real words");
    let original = match one(&bob, &alice, proof.clone()) {
        Outcome::Verified(m) => m,
        other => panic!("{other:?}"),
    };
    let json = serde_json::to_string(&report(&bob, &alice, Context::Dm, vec![proof])).unwrap();
    let bytes = json.as_bytes();
    let mut parsed = 0;
    for i in 0..bytes.len() {
        for replacement in *b"0f\"A" {
            if bytes[i] == replacement {
                continue;
            }
            let mut changed = bytes.to_vec();
            changed[i] = replacement;
            let Ok(text) = std::str::from_utf8(&changed) else {
                continue;
            };
            let Ok(r) = serde_json::from_str::<Report>(text) else {
                continue;
            };
            parsed += 1;
            let v: Verified = verify(&r, now());
            for m in v.proved() {
                assert_eq!(m.content, original.content, "byte {i}");
                assert_eq!(m.author, original.author, "byte {i}");
                assert_eq!(m.created_at, original.created_at, "byte {i}");
            }
        }
    }
    assert!(parsed > 100, "the mutations reached the checks ({parsed})");
}

// ─── The rest of the rules ──────────────────────────────────────────────────

#[test]
fn a_note_is_no_message() {
    let (alice, bob) = (Keys::generate(), Keys::generate());
    let w = messenger_dm::wrap::wrap_note(
        &alice,
        &pk(&bob),
        &Envelope::reaction(&"ab".repeat(32), "👎", true).encode(),
        now(),
        false,
        None,
    )
    .unwrap();
    let wrap: Event = serde_json::from_value(w.to_peer.json).unwrap();
    assert!(matches!(dm_proof(&bob, &wrap), Err(Error::Rumor(_))));

    let note = EventBuilder::new(Kind::from(KIND_PEER_NOTE_RUMOR), "{}")
        .tag(Tag::public_key(bob.public_key()))
        .finalize_unsigned(alice.public_key());
    let sealed = seal(&alice, &bob.public_key(), note);
    let keys = keys_of(&sealed, &bob, &alice.public_key());
    assert_eq!(
        unverified(&one(&bob, &alice, Proof::Dm { seal: sealed, keys })),
        "rumor_kind"
    );
}

#[test]
fn a_rumor_far_from_its_seal() {
    let (alice, bob) = (Keys::generate(), Keys::generate());
    let (to_bob, _, _) = send(&alice, &bob, &text("old"), now() - 10 * 86_400);
    assert_eq!(
        unverified(&one(&bob, &alice, dm_proof(&bob, &to_bob).unwrap())),
        "rumor_time"
    );
    // Within the two days a seal may be put back, it holds.
    let (to_bob, _, _) = send(&alice, &bob, &text("recent"), now() - 86_400);
    assert!(matches!(
        one(&bob, &alice, dm_proof(&bob, &to_bob).unwrap()),
        Outcome::Verified(_)
    ));
}

#[test]
fn the_same_message_twice_counts_once() {
    let (alice, bob) = (Keys::generate(), Keys::generate());
    let (to_bob, _, _) = send(&alice, &bob, &text("once"), now());
    let p = dm_proof(&bob, &to_bob).unwrap();
    let v = verify(
        &report(&bob, &alice, Context::Dm, vec![p.clone(), p]),
        now(),
    );
    assert!(matches!(v.messages[0], Outcome::Verified(_)));
    assert_eq!(unverified(&v.messages[1]), "duplicate");
}

#[test]
fn a_wrap_to_somebody_else_makes_no_proof() {
    let (alice, bob, carol) = (Keys::generate(), Keys::generate(), Keys::generate());
    let (to_carol, _, _) = send(&alice, &carol, &text("hi"), now());
    assert!(matches!(dm_proof(&bob, &to_carol), Err(Error::NotMine)));
    let mut tampered = to_carol.clone();
    tampered.content.push('A');
    assert!(matches!(dm_proof(&carol, &tampered), Err(Error::NotAWrap)));
}

#[test]
fn report_level_rules() {
    let (alice, bob) = (Keys::generate(), Keys::generate());
    let (proof, _) = alice_to_bob(&alice, &bob, "hi");
    let base = report(&bob, &alice, Context::Dm, vec![proof]);
    let refused = |r: &Report| verify(r, now()).refused;

    assert_eq!(refused(&base), None);
    assert_eq!(
        refused(&Report {
            v: 2,
            ..base.clone()
        })
        .as_deref(),
        Some("version")
    );
    assert_eq!(
        refused(&Report {
            reporter: "zz".into(),
            ..base.clone()
        })
        .as_deref(),
        Some("reporter_key")
    );
    // A report may have waited a week in the outbox of a device without a
    // connection; one older than that, or from the future, is refused.
    assert_eq!(
        refused(&Report {
            created_at: now() - 7 * 86_400,
            ..base.clone()
        }),
        None
    );
    assert_eq!(
        refused(&Report {
            created_at: now() - 9 * 86_400,
            ..base.clone()
        })
        .as_deref(),
        Some("created_at")
    );
    assert_eq!(
        refused(&Report {
            created_at: now() + 2 * 3_600,
            ..base.clone()
        })
        .as_deref(),
        Some("created_at")
    );
    assert_eq!(
        refused(&Report {
            created_at: i64::MIN,
            ..base.clone()
        })
        .as_deref(),
        Some("created_at")
    );
    assert_eq!(
        refused(&Report {
            comment: Some("я".repeat(2000)),
            ..base.clone()
        }),
        None,
        "characters, not bytes"
    );
    assert_eq!(
        refused(&Report {
            comment: Some("x".repeat(2001)),
            ..base.clone()
        })
        .as_deref(),
        Some("comment_too_long")
    );
    assert_eq!(
        refused(&Report {
            sub_reason: Some("x".repeat(65)),
            ..base.clone()
        })
        .as_deref(),
        Some("sub_reason_too_long")
    );
    assert_eq!(
        refused(&Report {
            context: Context::Group {
                group_id: String::new()
            },
            ..base.clone()
        })
        .as_deref(),
        Some("group_id")
    );
    let mut noted = base.clone();
    noted.messages.push(Attached {
        proof: None,
        note: Some("x".repeat(65)),
    });
    assert_eq!(refused(&noted).as_deref(), Some("note_too_long"));
    assert_eq!(
        verify(&base, i64::MAX).refused.as_deref(),
        Some("created_at")
    );
}
