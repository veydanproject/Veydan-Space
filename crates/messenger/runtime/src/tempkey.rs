// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The linking of devices on the relays of the project
//! (internal/cloud-recovery.md §6, internal/messenger-wire.md §3): what the
//! module `cloud` asks of the messenger through its seams, on pools of
//! their own that live for one step of the procedure.
//!
//! Why pools of their own: a new device has no identity and, before the
//! servers are chosen, no relay in the pool of the runtime; the key T is
//! not the session's, and what comes to it must not reach the ingress.
//! The relays are those of the project's manifest for the region, the
//! ones the notes go through; with the user's own servers the procedure
//! does not go to the project at all.

use std::collections::HashSet;
use std::time::Duration;

use messenger_core::envelope::{Envelope, KIND_LINK_CODE, KIND_OWN_RUMOR, KIND_PEER_NOTE_RUMOR};
use messenger_core::outbound::{Filter, WireEvent};
use messenger_core::traits::{Clock, SystemClock};
use messenger_core::{EventId, MessengerError, Outbound, PubKey, Result, Scope, SubId, Transport};
use messenger_dm::keynotes::{self, KeptWrap, KeyNote};
use messenger_transport::{RelayConfig, RelayPool};
use nostr::prelude::{Event, Keys};
use tokio::sync::mpsc;

use crate::relays::ServersMode;
use crate::MessengerRuntime;

/// How long a pool of a step waits for its first relay.
pub const CONNECT_PATIENCE: Duration = Duration::from_secs(8);
/// How long a lookup waits for the relays to answer.
pub const FETCH_PATIENCE: Duration = Duration::from_secs(6);
/// How far back a device waiting for a note of my own devices asks: the
/// note carries the real time; this covers clocks that disagree.
pub const OWN_NOTE_SKEW_SECS: i64 = 900;
/// The words of the error when the user keeps to their own servers.
pub const OWN_SERVERS: &str = "own_servers";
const LISTEN_QUEUE: usize = 16;
const SUB_KEY: &str = "key-listen";

fn now() -> i64 {
    SystemClock.now().secs()
}

/// A pool of `relays` with at least one connected, signing NIP-42 with
/// `signer`; or the transport error the screens show as "no connection to
/// the relays".
pub async fn open_pool(relays: &[RelayConfig], signer: Option<Keys>) -> Result<RelayPool> {
    if relays.is_empty() {
        return Err(MessengerError::Transport(
            "no relay of the project is known".into(),
        ));
    }
    let pool = RelayPool::new(signer);
    if let Err(e) = pool.set_relays(relays.to_vec()).await {
        pool.shutdown().await;
        return Err(e);
    }
    let deadline = tokio::time::Instant::now() + CONNECT_PATIENCE;
    while pool.connected().await == 0 {
        if tokio::time::Instant::now() >= deadline {
            pool.shutdown().await;
            return Err(MessengerError::Transport("no relay connected".into()));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Ok(pool)
}

/// `event` to `relays`: done when one relay took it.
pub async fn publish(relays: &[RelayConfig], signer: Option<Keys>, event: WireEvent) -> Result<()> {
    let pool = open_pool(relays, signer).await?;
    let ack = pool.send(Outbound::PublishOwn { event }).await;
    pool.shutdown().await;
    let ack = ack?;
    if ack.is_delivered() {
        Ok(())
    } else {
        Err(MessengerError::Transport(format!(
            "no relay took the event: {:?}",
            ack.rejected_by
        )))
    }
}

/// The gift wraps to the key of `keys` on `relays` (from `since` when
/// given), opened with it, each one once; what `accept` takes is handed
/// on. The pool closes when the receiver is dropped, and the keys go with
/// it.
pub async fn listen<F>(
    relays: &[RelayConfig],
    keys: Keys,
    since: Option<i64>,
    accept: F,
) -> Result<mpsc::Receiver<KeyNote>>
where
    F: Fn(&KeyNote) -> bool + Send + 'static,
{
    let pool = open_pool(relays, Some(keys.clone())).await?;
    let mut events = pool.events();
    let mut filter = serde_json::json!({ "kinds": [1059], "#p": [keys.public_key().to_hex()] });
    if let Some(since) = since {
        filter["since"] = since.max(0).into();
    }
    if let Err(e) = pool
        .send(Outbound::Subscribe {
            id: SubId(SUB_KEY.into()),
            filter: Filter(filter),
            scope: Scope::Own,
        })
        .await
    {
        pool.shutdown().await;
        return Err(e);
    }
    let (tx, rx) = mpsc::channel(LISTEN_QUEUE);
    tokio::spawn(async move {
        let mut seen = HashSet::new();
        loop {
            let raw = tokio::select! {
                _ = tx.closed() => break,
                raw = events.recv() => raw,
            };
            let Some(raw) = raw else { break };
            if raw.kind != 1059 || !seen.insert(raw.id.as_hex().to_string()) {
                continue;
            }
            let Ok(event) = serde_json::from_value::<Event>(raw.json) else {
                continue;
            };
            let Some(note) = keynotes::open_wrap(&keys, &event) else {
                continue;
            };
            if accept(&note) && tx.send(note).await.is_err() {
                break;
            }
        }
        pool.shutdown().await;
    });
    Ok(rx)
}

/// A live code of a link on the relays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundCode {
    /// The key T that signed it.
    pub pubkey: PubKey,
    /// Its NIP-40 `expiration`, unix seconds, when it names one.
    pub expiration: Option<u64>,
}

/// The key that published the latest live kind 30078 under `d` on
/// `relays`, with its `expiration`; several under one `d`: the latest by
/// `created_at` (the fingerprint catches a swap, and a T without the code
/// opens nothing).
pub async fn find_link_code(relays: &[RelayConfig], d: &str) -> Result<Option<FoundCode>> {
    let pool = open_pool(relays, None).await?;
    let filter = Filter(serde_json::json!({ "kinds": [KIND_LINK_CODE], "#d": [d] }));
    let found = pool.fetch(&filter, &messenger_core::Scope::Own, FETCH_PATIENCE).await;
    pool.shutdown().await;
    let at = now();
    Ok(found?
        .into_iter()
        .filter_map(|json| serde_json::from_value::<Event>(json).ok())
        .filter(|e| e.kind.as_u16() == KIND_LINK_CODE && e.tags.identifier().as_deref() == Some(d))
        .filter(|e| !e.is_expired_at(nostr::prelude::Timestamp::from_secs(at.max(0) as u64)))
        .max_by_key(|e| e.created_at)
        .and_then(|e| {
            Some(FoundCode {
                pubkey: PubKey::parse(&e.pubkey.to_hex())?,
                expiration: e.tags.expiration().map(|t| t.as_secs()),
            })
        }))
}

/// The picture of a link as a QR code: the generator of group links, so
/// that every code of the app looks the same.
pub fn link_qr_svg(link: &str) -> Result<String> {
    messenger_groups::qr::link_svg(link)
}

impl MessengerRuntime {
    /// The relays of the project's manifest for the region: where the
    /// notes of a link go. Refused in silent mode, and with the user's own
    /// servers (the procedure is the project's).
    pub async fn project_relays(&self) -> Result<Vec<RelayConfig>> {
        if self.relays().is_silent().await? {
            return Err(MessengerError::Transport("silent mode".into()));
        }
        if self.relays().servers_mode().await? == Some(ServersMode::Own) {
            return Err(MessengerError::Invalid(OWN_SERVERS.into()));
        }
        let (manifest, _) = self.relays().current_manifest().await?;
        let region = self.relays().region().await?;
        Ok(manifest
            .relays_for_region(&region)
            .into_iter()
            .filter_map(|r| {
                Some(RelayConfig {
                    url: messenger_core::RelayUrl::parse(&r.url)?,
                    read: r.read,
                    write: r.write,
                    api_key: r.auth.as_ref().and_then(|a| a.secret().map(String::from)),
                    server_id: None,
                })
            })
            .collect())
    }

    /// The notes from other keys (rumor kind 30079) that come to the key
    /// `t`, before or without an identity. Dropping the receiver ends it.
    pub async fn listen_as_key(&self, t: Keys) -> Result<mpsc::Receiver<KeyNote>> {
        let relays = self.project_relays().await?;
        listen(&relays, t, None, |note| {
            note.rumor_kind == KIND_PEER_NOTE_RUMOR
        })
        .await
    }

    /// A note to the key `to`, sealed by the identity, forgotten at
    /// `expiration`; the wrap and its outer key, for the deletion later.
    pub async fn send_note_to_key(
        &self,
        to: &PubKey,
        envelope: &Envelope,
        expiration: i64,
    ) -> Result<KeptWrap> {
        let relays = self.project_relays().await?;
        let keys = self.identity().load_keys().await?;
        let kept = keynotes::wrap_note_to_key(&keys, to, &envelope.encode(), now(), expiration)?;
        publish(&relays, Some(keys), kept.wrap.clone()).await?;
        Ok(kept)
    }

    /// A note to my own devices, stamped with the real time.
    pub async fn send_own_note_now(&self, envelope: &Envelope) -> Result<EventId> {
        let relays = self.project_relays().await?;
        let keys = self.identity().load_keys().await?;
        let wrap = keynotes::wrap_own_now(&keys, &envelope.encode(), now())?;
        let id = wrap.id.clone();
        publish(&relays, Some(keys), wrap).await?;
        Ok(id)
    }

    /// Waits until `until` (unix seconds) for a note of my own devices of
    /// type `t` whose field `field` is `value`: whether it came.
    pub async fn await_own_note(
        &self,
        t: &str,
        field: &str,
        value: &str,
        until: i64,
    ) -> Result<bool> {
        let relays = self.project_relays().await?;
        let keys = self.identity().load_keys().await?;
        let (t, field, value) = (t.to_string(), field.to_string(), value.to_string());
        let mut rx = listen(
            &relays,
            keys,
            Some(now() - OWN_NOTE_SKEW_SECS),
            move |note| {
                note.rumor_kind == KIND_OWN_RUMOR
                    && note.envelope.t == t
                    && note.envelope.str_field(&field) == Some(value.as_str())
            },
        )
        .await?;
        let wait = Duration::from_secs(until.saturating_sub(now()).max(0) as u64);
        Ok(matches!(
            tokio::time::timeout(wait, rx.recv()).await,
            Ok(Some(_))
        ))
    }

    /// The open event of a typed link code, signed by `t`: kind 30078 with
    /// `d`, forgotten at `expiration`. Its id, for the deletion later.
    pub async fn publish_link_code(&self, t: &Keys, d: &str, expiration: i64) -> Result<EventId> {
        let relays = self.project_relays().await?;
        let event = keynotes::link_code_event(t, d, now(), expiration)?;
        let id = event.id.clone();
        publish(&relays, Some(t.clone()), event).await?;
        Ok(id)
    }

    /// The key T behind a typed link code and its `expiration`, if a live
    /// one is on the relays.
    pub async fn find_link_code(&self, d: &str) -> Result<Option<FoundCode>> {
        let relays = self.project_relays().await?;
        find_link_code(&relays, d).await
    }

    /// NIP-09: `signer` asks the relays of the project to delete its events
    /// `ids` of `kind`.
    pub async fn delete_events(&self, signer: &Keys, ids: &[EventId], kind: u16) -> Result<()> {
        let relays = self.project_relays().await?;
        let event = keynotes::deletion(signer, ids, kind, now())?;
        publish(&relays, Some(signer.clone()), event).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use messenger_core::envelope::{T_DEVICE_LINK, T_DEVICE_LINK_DONE};
    use messenger_core::RelayUrl;
    use nostr_sdk::local_relay::LocalRelay;

    async fn local_relay() -> (LocalRelay, Vec<RelayConfig>) {
        let relay = LocalRelay::builder().build();
        relay.run().await.expect("local relay starts");
        let url = relay.url().await;
        let url = RelayUrl::parse(url.as_str_without_trailing_slash()).expect("url");
        (
            relay,
            vec![RelayConfig {
                url,
                read: true,
                write: true,
                api_key: None,
                server_id: None,
            }],
        )
    }

    fn pk(k: &Keys) -> PubKey {
        PubKey::parse(&k.public_key().to_hex()).expect("key")
    }

    async fn wraps_to(relays: &[RelayConfig], to: &Keys) -> usize {
        let pool = open_pool(relays, None).await.expect("pool");
        let filter =
            Filter(serde_json::json!({ "kinds": [1059], "#p": [to.public_key().to_hex()] }));
        let n = pool
            .fetch(&filter, &messenger_core::Scope::Own, FETCH_PATIENCE)
            .await
            .expect("fetch")
            .len();
        pool.shutdown().await;
        n
    }

    /// The whole exchange of a link on a relay: the code published by T and
    /// found by the old device, the note to T heard by the new one alone,
    /// the note of my own devices heard by the old one, the deletions.
    #[tokio::test]
    async fn a_link_goes_over_the_relay_and_leaves_nothing_behind() {
        let (_relay, relays) = local_relay().await;
        let (old, t, stranger) = (Keys::generate(), Keys::generate(), Keys::generate());
        let start = now();

        // New device: the code on the relay, then it listens as T.
        let d = "5e".repeat(32);
        let code = keynotes::link_code_event(&t, &d, start, start + 600).expect("code");
        let code_id = code.id.clone();
        publish(&relays, Some(t.clone()), code)
            .await
            .expect("published");
        let mut heard = listen(&relays, t.clone(), None, |n| {
            n.rumor_kind == KIND_PEER_NOTE_RUMOR
        })
        .await
        .expect("listen");

        // Old device: the key behind the code and its end, and nothing
        // behind another.
        assert_eq!(
            find_link_code(&relays, &d).await.expect("lookup"),
            Some(FoundCode {
                pubkey: pk(&t),
                expiration: Some((start + 600) as u64),
            })
        );
        assert_eq!(
            find_link_code(&relays, &"00".repeat(32))
                .await
                .expect("lookup"),
            None
        );

        // A stranger listening on its own key hears nothing of it.
        let mut strangers = listen(&relays, stranger.clone(), None, |_| true)
            .await
            .expect("listen");

        let note = Envelope::new(T_DEVICE_LINK).with("blob", "AQID");
        let kept = keynotes::wrap_note_to_key(&old, &pk(&t), &note.encode(), now(), now() + 600)
            .expect("wrap");
        publish(&relays, Some(old.clone()), kept.wrap.clone())
            .await
            .expect("sent");
        let got = tokio::time::timeout(Duration::from_secs(5), heard.recv())
            .await
            .expect("in time")
            .expect("a note");
        assert_eq!(got.from, pk(&old), "the sealer is the old device");
        assert_eq!(got.envelope.str_field("blob"), Some("AQID"));
        assert!(
            tokio::time::timeout(Duration::from_millis(500), strangers.recv())
                .await
                .is_err()
        );

        // The new device, now with my key, tells my devices it is in.
        let done = Envelope::new(T_DEVICE_LINK_DONE).with("t_pub", t.public_key().to_hex());
        let t_hex = t.public_key().to_hex();
        let mut waiting = listen(
            &relays,
            old.clone(),
            Some(now() - OWN_NOTE_SKEW_SECS),
            move |n| {
                n.rumor_kind == KIND_OWN_RUMOR
                    && n.envelope.t == T_DEVICE_LINK_DONE
                    && n.envelope.str_field("t_pub") == Some(t_hex.as_str())
            },
        )
        .await
        .expect("listen");
        let own = keynotes::wrap_own_now(&old, &done.encode(), now()).expect("own");
        publish(&relays, Some(old.clone()), own)
            .await
            .expect("sent");
        let came = tokio::time::timeout(Duration::from_secs(5), waiting.recv())
            .await
            .expect("in time")
            .expect("the note");
        assert_eq!(came.from, pk(&old));

        // The old device deletes its wrap, the new one its code.
        assert_eq!(wraps_to(&relays, &t).await, 1);
        let del = keynotes::deletion(
            &kept.wrapper,
            std::slice::from_ref(&kept.wrap.id),
            1059,
            now(),
        )
        .expect("deletion");
        publish(&relays, Some(kept.wrapper.clone()), del)
            .await
            .expect("deleted");
        assert_eq!(
            wraps_to(&relays, &t).await,
            0,
            "the wrap with the nsec is gone"
        );
        let del = keynotes::deletion(&t, &[code_id], KIND_LINK_CODE, now()).expect("deletion");
        publish(&relays, Some(t.clone()), del)
            .await
            .expect("deleted");
        assert_eq!(find_link_code(&relays, &d).await.expect("lookup"), None);
    }

    /// What has expired is neither taken nor found: a wrap or a code past
    /// its `expiration` (NIP-40).
    #[tokio::test]
    async fn an_expired_wrap_or_code_is_not_kept() {
        let (_relay, relays) = local_relay().await;
        let (old, t) = (Keys::generate(), Keys::generate());
        let past = now() - 10;
        let note = Envelope::new(T_DEVICE_LINK).with("blob", "AQID");
        let stale = keynotes::wrap_note_to_key(&old, &pk(&t), &note.encode(), past - 600, past)
            .expect("wrap");
        assert!(
            publish(&relays, Some(old), stale.wrap).await.is_err(),
            "the relay refuses what has expired"
        );
        assert_eq!(wraps_to(&relays, &t).await, 0);
        let d = "6f".repeat(32);
        let code = keynotes::link_code_event(&t, &d, past - 600, past).expect("code");
        assert!(publish(&relays, Some(t.clone()), code).await.is_err());
        assert_eq!(find_link_code(&relays, &d).await.expect("lookup"), None);
    }

    /// Of two codes under one `d`, the later is taken.
    #[tokio::test]
    async fn the_latest_code_under_a_d_wins() {
        let (_relay, relays) = local_relay().await;
        let (first, second) = (Keys::generate(), Keys::generate());
        let d = "7a".repeat(32);
        let at = now();
        for (k, created) in [(&first, at - 5), (&second, at)] {
            let code = keynotes::link_code_event(k, &d, created, at + 600).expect("code");
            publish(&relays, Some(k.clone()), code)
                .await
                .expect("published");
        }
        assert_eq!(
            find_link_code(&relays, &d)
                .await
                .expect("lookup")
                .map(|f| f.pubkey),
            Some(pk(&second))
        );
    }

    #[tokio::test]
    async fn no_relay_is_a_transport_error() {
        let err = open_pool(&[], None).await.err();
        assert!(matches!(err, Some(MessengerError::Transport(_))));
        let dead = vec![RelayConfig {
            url: RelayUrl::parse("ws://127.0.0.1:9").expect("url"),
            read: true,
            write: true,
            api_key: None,
            server_id: None,
        }];
        let started = tokio::time::Instant::now();
        assert!(matches!(
            open_pool(&dead, None).await.err(),
            Some(MessengerError::Transport(_))
        ));
        assert!(started.elapsed() < CONNECT_PATIENCE + Duration::from_secs(3));
        assert!(link_qr_svg("veydan://link/ab/abandon")
            .expect("svg")
            .contains("#000000"));
    }

    /// The manifest in use names the relay at `url` alone: the relays of
    /// the project, in a test.
    async fn on_the_relay(rt: &MessengerRuntime, url: &str) {
        let mut m: serde_json::Value =
            serde_json::from_str(messenger_transport::EMBEDDED_MANIFEST_JSON).expect("embedded");
        m["serial"] = (m["serial"].as_u64().expect("serial") + 1).into();
        m["relays"] = serde_json::json!([
            { "id": "test-relay", "url": url, "regions": ["*"], "read": true, "write": true }
        ]);
        let manifest =
            messenger_transport::Manifest::parse_content(&m.to_string()).expect("manifest");
        rt.relays()
            .adopt_for_test(&manifest, true)
            .await
            .expect("adopted");
    }

    async fn runtime(dir: &std::path::Path) -> MessengerRuntime {
        let cfg = messenger_core::MessengerConfig::new(dir.join("messenger"));
        let secrets = std::sync::Arc::new(messenger_testkit::MemorySecretStore::unlocked());
        MessengerRuntime::start(cfg, secrets).await.expect("start")
    }

    /// The path of the app on two real runtimes: A with an identity and its
    /// servers chosen, B fresh (no identity, servers not chosen yet) on the
    /// relays of the manifest; the code, the note to T sealed by the
    /// identity, the identity imported on B, `device.link.done` heard by A,
    /// both deletions.
    #[tokio::test]
    async fn two_runtimes_link_over_the_relays_of_the_manifest() {
        let (_relay, relays) = local_relay().await;
        let url = relays[0].url.as_str().to_string();
        let (dir_a, dir_b) = (
            tempfile::tempdir().expect("dir"),
            tempfile::tempdir().expect("dir"),
        );
        let a = runtime(dir_a.path()).await;
        crate::servers::use_veydan_offline(&a).await;
        on_the_relay(&a, &url).await;
        a.identity().create("backup words").await.expect("identity");
        a.refresh_signer().await.expect("signer");
        let b = runtime(dir_b.path()).await;
        on_the_relay(&b, &url).await;
        assert_eq!(b.relays().servers_mode().await.expect("mode"), None);
        assert_eq!(b.project_relays().await.expect("relays").len(), 1);

        // B: T and its code, then it listens as T.
        let t = Keys::generate();
        let d = "8b".repeat(32);
        let end = now() + 600;
        let code_id = b.publish_link_code(&t, &d, end).await.expect("code");
        let mut heard = b.listen_as_key(t.clone()).await.expect("listen");

        // A: T behind the code; the note goes sealed by the identity.
        let found = a.find_link_code(&d).await.expect("lookup").expect("found");
        assert_eq!(found.pubkey, pk(&t));
        assert_eq!(found.expiration, Some(end as u64));
        let note = Envelope::new(T_DEVICE_LINK).with("blob", "AQID");
        let kept = a
            .send_note_to_key(&found.pubkey, &note, now() + 600)
            .await
            .expect("sent");
        let got = tokio::time::timeout(Duration::from_secs(5), heard.recv())
            .await
            .expect("in time")
            .expect("a note");
        let a_keys = a.identity().load_keys().await.expect("keys");
        assert_eq!(got.from, pk(&a_keys), "the sealer is the identity");
        drop(heard);

        // B takes the identity over (no session before the servers are
        // chosen) and tells my devices; A hears it.
        let nsec = a_keys.secret_key().to_secret_hex();
        b.identity().import_nsec(&nsec).await.expect("import");
        assert!(!b.refresh_signer().await.expect("signer"), "no session yet");
        let me = b.identity().get().await.expect("get").expect("an identity");
        assert_eq!(me.pubkey, pk(&a_keys));
        let t_hex = t.public_key().to_hex();
        let done = Envelope::new(T_DEVICE_LINK_DONE).with("t_pub", t_hex.clone());
        b.send_own_note_now(&done).await.expect("done");
        assert!(a
            .await_own_note(T_DEVICE_LINK_DONE, "t_pub", &t_hex, now() + 5)
            .await
            .expect("waited"));
        assert!(
            !a.await_own_note(T_DEVICE_LINK_DONE, "t_pub", &"cd".repeat(32), now() + 1)
                .await
                .expect("waited"),
            "another T is not this link"
        );

        // A deletes its wrap, B its code.
        a.delete_events(&kept.wrapper, std::slice::from_ref(&kept.wrap.id), 1059)
            .await
            .expect("deleted");
        assert_eq!(
            wraps_to(&relays, &t).await,
            0,
            "the wrap with the nsec is gone"
        );
        b.delete_events(&t, &[code_id], KIND_LINK_CODE)
            .await
            .expect("deleted");
        assert_eq!(a.find_link_code(&d).await.expect("lookup"), None);
        a.shutdown().await;
        b.shutdown().await;
    }
}
