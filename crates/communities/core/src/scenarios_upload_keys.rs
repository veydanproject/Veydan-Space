// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! A channel on the owner's own server (internal/communities-wire.md §9.1):
//! a host of the kit with `MEDIA_UPLOAD_KEY=required` takes a write only with
//! the app key of its card or an upload key the keeper made for the staff
//! (замер 2026-10-23: without the key a channel only on its own server never
//! got its first epoch). The hosts of the tests check the keys as the kit's
//! `upload.js` does (`MemHosts::set_kit`).

use crate::db;
use crate::media::{NewRecording, RecordingPlace, Sink};
use crate::op::{History, JoinMode, OpBody};
use crate::roles::Role;
use crate::service::NewChannel;
use crate::state::SCOPE_EVERYONE;
use crate::testkit::{Node, World, H1, H2};
use crate::upload_keys::{self, NEEDS_KEY, RENEW_SECS};
use messenger_core::PubKey;
use std::sync::atomic::Ordering;

const APP: &str = "app-key-of-the-owners-server";

fn new_channel(storage: &[&str]) -> NewChannel {
    NewChannel {
        name: "Own".into(),
        about: String::new(),
        picture: String::new(),
        join: JoinMode::Open,
        history: History::Open,
        relays: vec!["wss://r1.example".into(), "wss://r2.example".into()],
        storage: storage.iter().map(|s| s.to_string()).collect(),
    }
}

/// The owner, with the card of the server of `H1` on his device, and a
/// channel on `storage`.
async fn own_channel(w: &World, storage: &[&str]) -> (Node, String) {
    w.hosts.set_kit(H1, APP, true);
    let owner = w.node().await;
    owner.svc.set_app_keys(&[(H1.to_string(), APP.to_string())]);
    let (cid, _) = owner.svc.create(&owner.keys, new_channel(storage)).await.unwrap_or_else(|e| panic!("create: {e}"));
    w.pump(&[&owner]).await;
    (owner, cid)
}

async fn join_by_link(w: &World, owner: &Node, reader: &Node, cid: &str, all: &[&Node]) {
    let link = owner.svc.share_link(cid).await.unwrap_or_else(|e| panic!("link: {e}"));
    w.pump(&[reader]).await;
    reader.svc.open_link(&reader.keys, &link, "").await.unwrap_or_else(|e| panic!("open link: {e}"));
    w.pump(all).await;
}

async fn newest_epoch(n: &Node, cid: &str) -> Option<u32> {
    n.svc.member(&n.keys).newest_key(cid, SCOPE_EVERYONE).await.unwrap_or_else(|e| panic!("keys: {e}")).map(|k| k.epoch)
}

fn pk(n: &Node) -> PubKey {
    n.me.clone()
}

/// A file of `n` bytes sent by `node` as a post: its message, or the error.
async fn send_file(node: &Node, cid: &str, dir: &std::path::Path, name: &str, n: usize) -> messenger_core::Result<String> {
    let path = dir.join(name);
    std::fs::write(&path, vec![7u8; n]).unwrap_or_else(|e| panic!("{e}"));
    let (row, _) = node.svc.media_placeholder(&node.keys, cid, None, None, &path, None).await?;
    Ok(node.svc.media_send(&node.keys, &row.id, None, &Sink(|_| {})).await?.0)
}

fn signers_on(w: &World, host: &str) -> Vec<String> {
    w.hosts.signers.lock().unwrap_or_else(|e| e.into_inner()).iter().filter(|(h, _)| h == host).map(|(_, p)| p.clone()).collect()
}

fn refused(w: &World) -> Vec<(String, String)> {
    w.hosts.key_refused.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// A channel only on the owner's own server: its keeper puts the first
/// epoch there with the app key of the card, and a reader gets the key.
/// Without the card on the device nothing is put there and no epoch goes
/// out (what the trial of 2026-10-23 saw, 401 on every put), and nothing is
/// even tried.
#[tokio::test]
async fn a_channel_only_on_an_own_server_gets_its_first_epoch_with_the_app_key() {
    let w = World::new();
    let (owner, cid) = own_channel(&w, &[H1]).await;
    assert_eq!(w.relay.epochs(), vec![1], "the first epoch is out");
    assert!(w.hosts.count(H1) > 0, "its blobs on the own server");
    assert!(refused(&w).is_empty(), "no write refused: {:?}", refused(&w));
    let reader = w.node().await;
    join_by_link(&w, &owner, &reader, &cid, &[&owner, &reader]).await;
    assert_eq!(newest_epoch(&reader, &cid).await, Some(1), "the reader holds the key");

    // The cover goes there too.
    let rgba: Vec<u8> = (0..64u32 * 64).flat_map(|i| [(i % 251) as u8, 30, 200, 255]).collect();
    let png = messenger_avatar::png_of_rgba(64, 64, &rgba).unwrap_or_else(|e| panic!("png: {e:?}"));
    let cover = owner.svc.upload_cover(&owner.keys, &cid, &png).await.unwrap_or_else(|e| panic!("cover: {e}"));
    assert_eq!(cover.copies, vec![H1.to_string()]);

    // Without the card: nothing put, nothing refused, no epoch.
    let w2 = World::new();
    w2.hosts.set_kit(H1, APP, true);
    let owner2 = w2.node().await;
    let (cid2, _) = owner2.svc.create(&owner2.keys, new_channel(&[H1])).await.unwrap_or_else(|e| panic!("create: {e}"));
    w2.pump(&[&owner2]).await;
    assert!(w2.relay.epochs().is_empty(), "no epoch without the key of the server");
    assert_eq!(w2.hosts.puts.load(Ordering::SeqCst), 0);
    assert!(refused(&w2).is_empty(), "a host that needs the key is not even asked");
    let status = owner2.svc.keeper_status(&cid2).unwrap_or_else(|| panic!("a keeper here"));
    assert!(status.storage_error.as_deref().is_some_and(|e| e.contains(NEEDS_KEY)), "the keeper says why: {:?}", status.storage_error);
}

/// An author who is not the owner gets an upload key of the own server from
/// the keeper, sealed to him, and his file goes there signed by him; a
/// reader never gets one. Removed from the staff, his key is revoked on the
/// server and stops working; made author again, he gets a new one.
#[tokio::test]
async fn an_author_writes_to_the_own_server_with_the_key_the_keeper_gave_him_until_it_is_revoked() {
    let w = World::new();
    let (owner, cid) = own_channel(&w, &[H1]).await;
    let author = w.node().await;
    let reader = w.node().await;
    let all = [&owner, &author, &reader];
    join_by_link(&w, &owner, &author, &cid, &all).await;
    join_by_link(&w, &owner, &reader, &cid, &all).await;
    owner.svc.act(&owner.keys, &cid, OpBody::SetRole { who: pk(&author), role: Role::Author }).await.unwrap_or_else(|e| panic!("role: {e}"));
    w.pump(&all).await;

    let given = db::upload_keys::given(author.svc.db()).await.unwrap_or_default();
    assert_eq!(given.len(), 1, "one key, for the one host of the kit");
    let key = given[0].key.clone();
    let p = upload_keys::parse(&key).unwrap_or_else(|| panic!("an upload key"));
    assert_eq!((given[0].host.as_str(), p.pubkey.as_str(), p.label), (H1, author.me.as_hex(), upload_keys::label_of(&cid)));
    assert!(db::upload_keys::given(reader.svc.db()).await.unwrap_or_default().is_empty(), "no key for a reader");
    assert!(db::upload_keys::given(owner.svc.db()).await.unwrap_or_default().is_empty(), "the owner has the card");
    let made = db::upload_keys::made(owner.svc.db(), &cid).await.unwrap_or_default();
    assert_eq!(made.iter().map(|r| r.who.as_str()).collect::<Vec<_>>(), [author.me.as_hex()]);

    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let before = signers_on(&w, H1).len();
    send_file(&author, &cid, dir.path(), "a.bin", 50_000).await.unwrap_or_else(|e| panic!("the author's file: {e}"));
    assert!(signers_on(&w, H1)[before..].iter().all(|s| *s == author.me.as_hex()), "his chunks, signed by him");
    assert!(refused(&w).is_empty(), "{:?}", refused(&w));
    // The key alone is his: the same key under another signature is refused.
    let blob = b"not his".to_vec();
    let sha = crate::blobs::sha256_hex(&blob);
    assert!(crate::blobs::BlobHosts::put_keyed(&*w.hosts, &reader.keys, Some(&key), H1, &sha, &blob).await.is_err(), "a reader with the author's key");

    // Not an author any more: the keeper revokes the key on the server, the
    // author's device drops it, and the key no longer opens the server.
    owner.svc.act(&owner.keys, &cid, OpBody::SetRole { who: pk(&author), role: Role::Member }).await.unwrap_or_else(|e| panic!("role: {e}"));
    w.pump(&all).await;
    owner.svc.tick().await.unwrap_or_else(|e| panic!("tick: {e}"));
    author.svc.tick().await.unwrap_or_else(|e| panic!("tick: {e}"));
    assert!(w.hosts.revoked.lock().unwrap_or_else(|e| e.into_inner()).contains_key(&(H1.to_string(), upload_keys::label_of(&cid), author.me.as_hex().to_string())), "revoked on the server");
    assert!(db::upload_keys::made(owner.svc.db(), &cid).await.unwrap_or_default().is_empty(), "the keeper's row goes once revoked");
    assert!(db::upload_keys::given(author.svc.db()).await.unwrap_or_default().is_empty(), "dropped on the author's device");
    let blob = b"after the role".to_vec();
    let sha = crate::blobs::sha256_hex(&blob);
    assert!(crate::blobs::BlobHosts::put_keyed(&*w.hosts, &author.keys, Some(&key), H1, &sha, &blob).await.is_err(), "the revoked key opens nothing");

    // Author again, a moment later: a new key, made after the revocation.
    w.clock.advance(5);
    owner.svc.act(&owner.keys, &cid, OpBody::SetRole { who: pk(&author), role: Role::Author }).await.unwrap_or_else(|e| panic!("role: {e}"));
    w.pump(&all).await;
    let again = db::upload_keys::given(author.svc.db()).await.unwrap_or_default();
    assert_eq!(again.len(), 1);
    assert_ne!(again[0].key, key);
    send_file(&author, &cid, dir.path(), "b.bin", 20_000).await.unwrap_or_else(|e| panic!("the author's file with the new key: {e}"));

    // Its life runs low: the keeper makes it again.
    w.clock.advance(upload_keys::TTL_SECS - RENEW_SECS + 60);
    w.pump(&all).await;
    owner.svc.tick().await.unwrap_or_else(|e| panic!("tick: {e}"));
    w.pump(&all).await;
    let renewed = db::upload_keys::given(author.svc.db()).await.unwrap_or_default();
    assert!(renewed.len() == 1 && renewed[0].expires_at > again[0].expires_at, "renewed: {renewed:?}");
}

/// A reader never writes to the owner's own server: his recorded comment on
/// a channel only there is refused before anything goes up, and on a channel
/// also on an open host it goes there alone.
#[tokio::test]
async fn a_readers_recording_never_goes_to_the_own_server() {
    use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
    for storage in [vec![H1], vec![H1, H2]] {
        let w = World::new();
        let (owner, cid) = own_channel(&w, &storage).await;
        let reader = w.node().await;
        let all = [&owner, &reader];
        join_by_link(&w, &owner, &reader, &cid, &all).await;
        owner.svc.send_post(&owner.keys, &cid, "post", messenger_core::TextFormat::Plain).await.unwrap_or_else(|e| panic!("post: {e}"));
        w.pump(&all).await;
        let post = db::content::posts(reader.svc.db(), &cid, None, 10).await.unwrap_or_default().into_iter().next().unwrap_or_else(|| panic!("the post")).id;
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let rec = NewRecording {
            kind: "voice".into(),
            mime: "audio/webm".into(),
            duration_ms: Some(1_000),
            waveform: None,
            data_base64: B64.encode(vec![3u8; 20_000]),
        };
        let place = RecordingPlace { post_id: Some(&post), ..RecordingPlace::default() };
        let (row, _) = reader.svc.media_placeholder_recording(&reader.keys, &cid, rec, place, dir.path()).await.unwrap_or_else(|e| panic!("placeholder: {e}"));
        let sent = reader.svc.media_send(&reader.keys, &row.id, None, &Sink(|_| {})).await;
        assert!(signers_on(&w, H1).iter().all(|s| *s != reader.me.as_hex()), "{storage:?}: nothing of the reader on the own server");
        assert!(refused(&w).is_empty(), "{storage:?}: nothing even tried: {:?}", refused(&w));
        if storage.len() == 1 {
            let e = sent.err().map(|e| e.to_string()).unwrap_or_default();
            assert!(e.contains(NEEDS_KEY), "{e}");
        } else {
            let (id, _) = sent.unwrap_or_else(|e| panic!("the comment on the open host: {e}"));
            w.pump(&all).await;
            let item = db::content::get(owner.svc.db(), &id).await.ok().flatten().unwrap_or_else(|| panic!("the comment at the owner's"));
            let d = messenger_media::MediaDescriptor::from_envelope(&messenger_core::Envelope::parse(item.media_json.as_deref().unwrap_or("")).unwrap_or_else(|e| panic!("{e}")))
                .unwrap_or_else(|e| panic!("{e}"));
            assert_eq!(d.servers, vec![H2.to_string()], "the open host alone");
        }
    }
}
