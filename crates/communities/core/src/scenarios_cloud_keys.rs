// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The staff of a channel upload into the owner's Veydan Cloud
//! (internal/communities-wire.md §9.2, cloud-api.md §17): the keeper on the
//! owner's device registers a key of the channel for each author in the
//! owner's quota with the owner's account and hands it to him; his files go
//! up signed by it; «Своя квота», a lost role, revokes it and the author goes
//! on with his own key.

use crate::cloud_keys::{CloudDelegates, DelegatePut, PROOF_CONTENT, PROOF_KIND};
use crate::db;
use crate::media::Sink;
use crate::op::{History, JoinMode, OpBody};
use crate::roles::Role;
use crate::service::NewChannel;
use crate::testkit::{Node, World, H2};
use async_trait::async_trait;
use messenger_core::{MessengerError, PubKey, Result};
use std::sync::{Arc, Mutex};

/// The owner's Cloud as the tests keep it: who the account is, what was
/// registered and revoked, and a refusal to give when asked.
#[derive(Default)]
struct FakeCloud {
    account: Mutex<Option<String>>,
    puts: Mutex<Vec<DelegatePut>>,
    revokes: Mutex<Vec<String>>,
    refuse: Mutex<bool>,
}

#[async_trait]
impl CloudDelegates for FakeCloud {
    async fn account(&self) -> Option<String> {
        self.account.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
    async fn put(&self, d: &DelegatePut) -> Result<()> {
        if *self.refuse.lock().unwrap_or_else(|e| e.into_inner()) {
            return Err(MessengerError::Transport("409 delegate_taken".into()));
        }
        // What vcloud checks of the proof (cloud-api.md §17.1).
        let event: nostr::event::Event = serde_json::from_value(d.proof.clone()).map_err(|e| MessengerError::Invalid(e.to_string()))?;
        event.verify().map_err(|e| MessengerError::Invalid(e.to_string()))?;
        let owner = self.account().await.unwrap_or_default();
        let tags: Vec<Vec<String>> = event.tags.iter().map(|t| t.as_slice().to_vec()).collect();
        assert_eq!((event.kind.as_u16(), event.content.as_str(), event.pubkey.to_hex()), (PROOF_KIND, PROOF_CONTENT, d.pubkey.clone()));
        assert_eq!(tags, [vec!["p".to_string(), owner], vec!["d".to_string(), d.label.clone()]]);
        self.puts.lock().unwrap_or_else(|e| e.into_inner()).push(d.clone());
        Ok(())
    }
    async fn revoke(&self, pubkey: &str) -> Result<()> {
        self.revokes.lock().unwrap_or_else(|e| e.into_inner()).push(pubkey.to_string());
        Ok(())
    }
}

impl FakeCloud {
    fn puts(&self) -> Vec<DelegatePut> {
        self.puts.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
    fn revokes(&self) -> Vec<String> {
        self.revokes.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

fn new_channel() -> NewChannel {
    NewChannel {
        name: "Cloud".into(),
        about: String::new(),
        picture: String::new(),
        join: JoinMode::Open,
        history: History::Open,
        relays: vec!["wss://r1.example".into(), "wss://r2.example".into()],
        storage: vec![H2.to_string()],
    }
}

fn pk(n: &Node) -> PubKey {
    n.me.clone()
}

fn signers_on(w: &World, host: &str) -> Vec<String> {
    w.hosts.signers.lock().unwrap_or_else(|e| e.into_inner()).iter().filter(|(h, _)| h == host).map(|(_, p)| p.clone()).collect()
}

async fn send_file(node: &Node, cid: &str, dir: &std::path::Path, name: &str) -> Result<String> {
    let path = dir.join(name);
    std::fs::write(&path, vec![7u8; 30_000]).unwrap_or_else(|e| panic!("{e}"));
    let (row, _) = node.svc.media_placeholder(&node.keys, cid, None, None, &path, None).await?;
    Ok(node.svc.media_send(&node.keys, &row.id, None, &Sink(|_| {})).await?.0)
}

/// What the author's files are signed by after `steps`: the key of the
/// channel while he is in the owner's quota, his own once he is not.
#[tokio::test]
async fn the_staff_upload_into_the_owners_cloud_with_the_key_of_the_channel_until_the_owner_says_otherwise() {
    let w = World::new();
    let owner = w.node().await;
    let cloud = Arc::new(FakeCloud::default());
    *cloud.account.lock().unwrap_or_else(|e| e.into_inner()) = Some(owner.me.as_hex().to_string());
    owner.svc.set_cloud(Some(cloud.clone()));
    let (cid, _) = owner.svc.create(&owner.keys, new_channel()).await.unwrap_or_else(|e| panic!("create: {e}"));
    w.pump(&[&owner]).await;
    let author = w.node().await;
    let all = [&owner, &author];
    let link = owner.svc.share_link(&cid).await.unwrap_or_else(|e| panic!("link: {e}"));
    w.pump(&[&author]).await;
    author.svc.open_link(&author.keys, &link, "").await.unwrap_or_else(|e| panic!("open link: {e}"));
    w.pump(&all).await;
    assert!(cloud.puts().is_empty(), "a reader gets no key");

    // An author, in the owner's quota as everyone starts.
    owner.svc.act(&owner.keys, &cid, OpBody::SetRole { who: pk(&author), role: Role::Author }).await.unwrap_or_else(|e| panic!("role: {e}"));
    w.pump(&all).await;
    owner.svc.tick().await.unwrap_or_else(|e| panic!("tick: {e}"));
    w.pump(&all).await;
    let puts = cloud.puts();
    assert_eq!(puts.len(), 1, "one key, for the one author");
    let key = puts[0].pubkey.clone();
    assert_ne!(key, author.me.as_hex(), "a key of the channel, not his own");
    assert_eq!(puts[0].label, crate::upload_keys::label_of(&cid));
    let given = db::cloud_keys::given(author.svc.db()).await.unwrap_or_default();
    assert_eq!(given.iter().map(|g| (g.pubkey.as_str(), g.owner.as_str())).collect::<Vec<_>>(), [(key.as_str(), owner.me.as_hex())]);
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let before = signers_on(&w, H2).len();
    send_file(&author, &cid, dir.path(), "a.bin").await.unwrap_or_else(|e| panic!("the author's file: {e}"));
    let signed = signers_on(&w, H2)[before..].to_vec();
    assert!(!signed.is_empty() && signed.iter().all(|s| *s == key), "his chunks under the key of the channel: {signed:?}");
    // Another turn registers nothing again.
    owner.svc.tick().await.unwrap_or_else(|e| panic!("tick: {e}"));
    assert_eq!(cloud.puts().len(), 1);

    // «Своя квота»: revoked in the Cloud, dropped on his device, his own key again.
    owner.svc.act(&owner.keys, &cid, OpBody::SetQuota { who: pk(&author), own: true }).await.unwrap_or_else(|e| panic!("quota: {e}"));
    w.pump(&all).await;
    owner.svc.tick().await.unwrap_or_else(|e| panic!("tick: {e}"));
    w.pump(&all).await;
    author.svc.tick().await.unwrap_or_else(|e| panic!("tick: {e}"));
    assert_eq!(cloud.revokes(), std::slice::from_ref(&key));
    assert!(db::cloud_keys::made(owner.svc.db(), &cid).await.unwrap_or_default().is_empty(), "the keeper's row goes once revoked");
    assert!(db::cloud_keys::given(author.svc.db()).await.unwrap_or_default().is_empty(), "dropped on his device");
    let before = signers_on(&w, H2).len();
    send_file(&author, &cid, dir.path(), "b.bin").await.unwrap_or_else(|e| panic!("the author's own file: {e}"));
    assert!(signers_on(&w, H2)[before..].iter().all(|s| *s == author.me.as_hex()), "his own key now");

    // Back in the owner's quota: the same key registered again.
    owner.svc.act(&owner.keys, &cid, OpBody::SetQuota { who: pk(&author), own: false }).await.unwrap_or_else(|e| panic!("quota: {e}"));
    w.pump(&all).await;
    owner.svc.tick().await.unwrap_or_else(|e| panic!("tick: {e}"));
    w.pump(&all).await;
    assert_eq!(cloud.puts().len(), 2);
    assert_eq!(cloud.puts()[1].pubkey, key, "made again from the keeper's, the same");
    assert_eq!(db::cloud_keys::given(author.svc.db()).await.unwrap_or_default().len(), 1);

    // No author any more: revoked again.
    owner.svc.act(&owner.keys, &cid, OpBody::SetRole { who: pk(&author), role: Role::Member }).await.unwrap_or_else(|e| panic!("role: {e}"));
    w.pump(&all).await;
    owner.svc.tick().await.unwrap_or_else(|e| panic!("tick: {e}"));
    w.pump(&all).await;
    author.svc.tick().await.unwrap_or_else(|e| panic!("tick: {e}"));
    assert_eq!(cloud.revokes(), [key.clone(), key]);
    assert!(db::cloud_keys::given(author.svc.db()).await.unwrap_or_default().is_empty());
}

/// A device without the owner's account (none, or another's) registers and
/// revokes nothing; a Cloud that refuses is not asked again on every turn.
#[tokio::test]
async fn nothing_is_registered_without_the_owners_account_and_a_refusal_waits() {
    let w = World::new();
    let owner = w.node().await;
    let cloud = Arc::new(FakeCloud::default());
    owner.svc.set_cloud(Some(cloud.clone()));
    let (cid, _) = owner.svc.create(&owner.keys, new_channel()).await.unwrap_or_else(|e| panic!("create: {e}"));
    w.pump(&[&owner]).await;
    let author = w.node().await;
    let all = [&owner, &author];
    let link = owner.svc.share_link(&cid).await.unwrap_or_else(|e| panic!("link: {e}"));
    w.pump(&[&author]).await;
    author.svc.open_link(&author.keys, &link, "").await.unwrap_or_else(|e| panic!("open link: {e}"));
    w.pump(&all).await;
    owner.svc.act(&owner.keys, &cid, OpBody::SetRole { who: pk(&author), role: Role::Author }).await.unwrap_or_else(|e| panic!("role: {e}"));
    w.pump(&all).await;
    owner.svc.tick().await.unwrap_or_else(|e| panic!("tick: {e}"));
    assert!(cloud.puts().is_empty(), "no account here");
    *cloud.account.lock().unwrap_or_else(|e| e.into_inner()) = Some("ab".repeat(32));
    owner.svc.tick().await.unwrap_or_else(|e| panic!("tick: {e}"));
    assert!(cloud.puts().is_empty(), "another person's account");
    // The owner's, refusing: one try, then the wait.
    *cloud.account.lock().unwrap_or_else(|e| e.into_inner()) = Some(owner.me.as_hex().to_string());
    *cloud.refuse.lock().unwrap_or_else(|e| e.into_inner()) = true;
    owner.svc.tick().await.unwrap_or_else(|e| panic!("tick: {e}"));
    owner.svc.tick().await.unwrap_or_else(|e| panic!("tick: {e}"));
    assert!(db::cloud_keys::made(owner.svc.db(), &cid).await.unwrap_or_default().is_empty());
    *cloud.refuse.lock().unwrap_or_else(|e| e.into_inner()) = false;
    owner.svc.tick().await.unwrap_or_else(|e| panic!("tick: {e}"));
    assert!(cloud.puts().is_empty(), "not before the wait");
    w.clock.advance(crate::keeper::CLOUD_RETRY_SECS + 1);
    owner.svc.tick().await.unwrap_or_else(|e| panic!("tick: {e}"));
    w.pump(&all).await;
    assert_eq!(cloud.puts().len(), 1, "asked again after the wait");
    assert_eq!(db::cloud_keys::given(author.svc.db()).await.unwrap_or_default().len(), 1);
}
