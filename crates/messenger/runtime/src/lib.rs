// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The one object a host embeds.
//!
//! `MessengerRuntime::start(config, secret_store)` opens the store, the
//! identity service and the relay pool, and starts a session (ingress loop
//! and outbox pump) as soon as signing keys are available. The host only ever
//! talks to this type: the Tauri adapter today, a standalone app or the
//! `messenger-cli` tomorrow. Nothing here knows about Tauri.

pub mod media;
pub mod notify;

/// Region used when the stored one cannot be read.
pub(crate) const REGION_FALLBACK: &str = "default";
pub mod avatars;
pub mod bindings;
pub mod call_nodes;
pub mod calls;
pub mod cards;
pub mod groups;
pub mod link;
pub mod links;
pub mod net;
pub mod preview;
pub mod presence;
pub mod privacy;
pub mod push;
pub mod reactions;
pub mod receipts;
pub mod relays;
pub mod servers;
pub mod session;
pub mod shared;

use messenger_core::outbound::WireEvent;
use messenger_core::traits::{RelayState, SystemClock, UiEvent};
use messenger_contacts::{ContactService, MetaHandler, Nip05Service, ProfileService, ReqwestFetcher};
use messenger_core::{Clock, EventId, MessengerConfig, MessengerError, Outbound, PubKey, Result, Scope, SecretStore, SubId, Transport};
use messenger_dm::{Action, DmHandler, DmRoutesHandler, DmService, Prepared};
use messenger_identity::IdentityService;
use messenger_media::{MediaService, Recovered};
use messenger_ingress::{filters, Dispatcher, Fanout, Outbox};
use messenger_store::{settings, Store};
use nostr::key::Keys;
use nostr::prelude::*;
use serde::{Deserialize, Serialize};
use session::Session;
use std::sync::Arc;
use tokio::sync::{broadcast, Mutex};

pub use messenger_dm::{
    Action as DmAction, ChatView, MessageView, ReactionView, RelationView, UI_EVENT_CHAT_READ, UI_EVENT_CHAT_RECEIPT,
    UI_EVENT_CONTACT_PRIVATE_UPDATED, UI_EVENT_EMOJI_UPDATED, UI_EVENT_OWN_PRIVATE_UPDATED,
};
pub use media::{passive, PickedView, Poster, Recording};
pub use avatars::{AvatarPreview, UI_EVENT_AVATAR_READY};
pub use cards::{ContactPrivateView, OwnPrivateView};
pub use messenger_avatar::CropRect;
pub use messenger_contacts::{CardView, SocialLink, SocialPlatform, SocialView};
pub use messenger_richtext::{Color as BioColor, Span, Style as BioStyle};
pub use messenger_media::{MediaKind, MediaServerInput, MediaServerView, Paused, Progress as TransferProgress, TransferStage, TransferView};

pub use messenger_contacts::book::{parse_key, ContactPatch};
pub use messenger_contacts::{ContactView, ProfileInput, ProfileView};
pub use messenger_identity::{CreatedIdentity, Identity};
pub use messenger_groups::{GroupKind, GroupView, InviteView, KeyView as GroupKeyView, MemberView, OpBody as GroupOp, Role as GroupRole};
pub use links::LinkView;
pub use calls::{CallBackends, RegistrySource};
pub use call_nodes::{CallNodeClass, CallNodeHealth, CallNodeInfo, CallNodeSource, CallNodesView, CallTrust};
pub use calls::{
    CallDirection, CallEnded, CallIncoming, CallLimits, CallMedia, CallNodeInput, CallNodeView, CallOutcome, CallPhase,
    CallState, CallStats, CallVia, CallView, GroupCallAnnounced, GroupCallEnded, GroupCallLevel, GroupCallPhase,
    GroupCallState, GroupCallView, GroupParticipant, MediaEngine, ReconnectReason, RelayPolicy, UI_EVENT_CALL_ENDED,
    UI_EVENT_CALL_INCOMING, UI_EVENT_CALL_STATE, UI_EVENT_GROUP_CALL_ENDED, UI_EVENT_GROUP_CALL_LEVEL,
    UI_EVENT_GROUP_CALL_STARTED, UI_EVENT_GROUP_CALL_STATE,
};
pub use privacy::PrivacySettings;
pub use messenger_presence::PresenceView;
pub use messenger_preview::Preview as LinkPreview;
pub use relays::{ManifestCheck, ManifestInfo, RelayService, RelayView, ServersMode};
pub use servers::ManifestRemote;
pub use messenger_transport::{Manifest, ManifestFetcher};
pub use shared::{SharedCounts, SharedSection};

/// Facts for the host's status screen. Never contains secrets.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeStatus {
    /// Messenger crate version (not the host application version).
    pub version: String,
    pub data_dir: String,
    pub schema_version: i64,
    pub secrets_unlocked: bool,
    pub identity_present: bool,
    /// Whether a session (signer + ingress loop) is running.
    pub session_active: bool,
    pub relays_total: usize,
    pub relays_connected: usize,
    pub silent_mode: bool,
    /// What the app shows about its connection: a promise that is taken
    /// back only after a while without relays (`link`).
    pub link: link::LinkState,
    pub manifest_serial: Option<u64>,
    pub region: String,
    /// Whose servers are used; `None` until the user chooses.
    pub servers_mode: Option<ServersMode>,
    pub ingress: IngressCounters,
    pub outbox_pending: i64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct IngressCounters {
    pub received: u64,
    pub duplicates: u64,
    pub dispatched: u64,
    pub dm: u64,
    pub ignored: u64,
}

pub struct MessengerRuntime {
    config: MessengerConfig,
    store: Store,
    secrets: Arc<dyn SecretStore>,
    identity: IdentityService,
    relays: Arc<RelayService>,
    profiles: ProfileService,
    contacts: ContactService,
    nip05: Nip05Service,
    dm: DmService,
    media: MediaService,
    previews: preview::LinkPreviews,
    group_driver: groups::GroupsDriver,
    group_signals: tokio::task::JoinHandle<()>,
    outbox: Outbox,
    dispatcher: Arc<Dispatcher>,
    ui: broadcast::Sender<UiEvent>,
    session: Mutex<Option<Session>>,
    manifest_remote: std::sync::RwLock<ManifestRemote>,
    net: net::NetService,
    link: link::LinkWatch,
    /// Presence: whom to tell my key, whom to watch, when to beat.
    presence: Arc<presence::PresenceDriver>,
    /// My avatar and those of others (`crate::avatars`).
    avatars: avatars::Avatars,
    /// Photos made smaller at once (`media::PHOTO_SLOTS`): each takes a
    /// decoded picture in memory.
    photo_slots: Arc<tokio::sync::Semaphore>,
    /// Uploads in the preparing stage now (`media::Preparing`).
    preparing: media::Preparing,
    /// Calls: the service of the core and what serves it (`crate::calls`).
    calls: calls::CallsDriver,
    /// Group calls: the other service of the core, over the same engine.
    group_calls: calls::GroupCallsDriver,
}

impl MessengerRuntime {
    /// The runtime with the media engine of this build (`calls::default_engine`),
    /// or without one in a build that has none.
    pub async fn start(config: MessengerConfig, secrets: Arc<dyn SecretStore>) -> Result<Self> {
        Self::start_inner(config, secrets, CallBackends::of_build()).await
    }

    /// The runtime with the media engine the host gives: the CLI pushes
    /// its sound through one, the tests take the fake of the testkit.
    pub async fn start_with_engine(config: MessengerConfig, secrets: Arc<dyn SecretStore>, engine: Arc<dyn MediaEngine>) -> Result<Self> {
        Self::start_inner(config, secrets, CallBackends::with_engine(Some(engine))).await
    }

    /// The runtime with everything its calls are made with given by the
    /// host: the engine, the client of the nodes, the client of their
    /// rooms (the tests take the fakes of the testkit for all three).
    pub async fn start_with_backends(config: MessengerConfig, secrets: Arc<dyn SecretStore>, backends: CallBackends) -> Result<Self> {
        Self::start_inner(config, secrets, backends).await
    }

    async fn start_inner(config: MessengerConfig, secrets: Arc<dyn SecretStore>, backends: CallBackends) -> Result<Self> {
        messenger_transport::ensure_crypto_provider();
        let store = Store::open(&config).await?;
        let identity = IdentityService::new(store.clone(), secrets.clone());
        // Installs from before the choice existed keep the project's servers.
        if settings::get(&store, relays::KEY_MODE).await?.is_none() && identity.get().await?.is_some() {
            settings::set(&store, relays::KEY_MODE, "veydan").await?;
        }
        let signer = load_signer(&identity).await;
        // Before any connection is made: the way to the servers the app was
        // using when it last ran.
        let net = net::NetService::init(store.clone()).await?;
        let relays = Arc::new(RelayService::init(store.clone(), signer.clone()).await?);
        if let Err(e) = net.apply(&relays).await {
            eprintln!("messenger net: the way to the servers was not set: {e}");
        }
        let media = MediaService::new(store.clone(), secrets.clone(), config.data_dir())?;
        // Another process with this data folder (the CLI beside the app) may
        // be sending: every upload pauses but those it may run.
        let under_way = match media.recover().await? {
            Recovered::Alone(_) => Vec::new(),
            Recovered::Beside(under_way) => under_way,
        };
        messenger_store::messages::pause_uploading_except(&store, &under_way).await?;
        let outbox = Outbox::new(store.clone(), Arc::new(SystemClock));
        let profiles = ProfileService::new(store.clone());
        let contacts = ContactService::new(store.clone(), profiles.clone());
        let nip05 = Nip05Service::new(Arc::new(ReqwestFetcher::new()));
        let dm = DmService::new(store.clone(), contacts.clone(), profiles.clone(), Arc::new(SystemClock));
        let group_service =
            messenger_groups::GroupService::new(store.clone(), secrets.clone(), Arc::new(SystemClock), dm.clone());
        let (signals, signals_rx) = tokio::sync::mpsc::unbounded_channel();
        let (ui, _) = broadcast::channel(256);
        let CallBackends { engine, nodes, rooms, registry } = backends;
        let calls = calls::CallsDriver::new(
            store.clone(),
            dm.clone(),
            engine.clone(),
            nodes.clone(),
            outbox.clone(),
            ui.clone(),
            secrets.clone(),
            registry,
        );
        let group_calls = calls::GroupCallsDriver::new(
            store.clone(),
            dm.clone(),
            group_service.clone(),
            engine,
            calls.servers(),
            nodes,
            rooms,
            outbox.clone(),
            ui.clone(),
        );
        // A call between two that rings in during a group call is told so.
        calls.link_group_calls(group_calls.service.clone());
        // The chain of the DM handlers: GroupDmHandler → CallDmHandler → DmHandler.
        let dispatcher = Arc::new(
            Dispatcher::new()
                .with_dm(Arc::new(messenger_groups::GroupDmHandler::new(
                    group_service.clone(),
                    signals.clone(),
                    calls.handler(Arc::new(DmHandler::new(dm.clone()))),
                )))
                .with_group(Arc::new(messenger_groups::GroupHandler::new(group_service.clone(), signals)))
                .with_meta(Arc::new(Fanout::new(vec![
                    Arc::new(MetaHandler::new(profiles.clone(), contacts.clone())),
                    Arc::new(DmRoutesHandler::new(store.clone())),
                    Arc::new(messenger_presence::PresenceHandler::new(store.clone())),
                ]))),
        );
        let previews = preview::LinkPreviews::new(
            store.clone(),
            relays.clone(),
            messenger_preview::PreviewService::new(Arc::new(messenger_preview::ReqwestFetcher::new()?)),
            Arc::new(SystemClock),
        );
        let avatars = avatars::Avatars::new(
            store.clone(),
            media.clone(),
            profiles.clone(),
            outbox.clone(),
            ui.clone(),
            Arc::new(SystemClock),
            Arc::new(avatars::HttpAvatarNet::new()?),
            config.avatars_dir(),
        );
        let group_driver =
            groups::GroupsDriver::new(group_service, store.clone(), relays.clone(), outbox.clone(), ui.clone());
        let group_signals = tokio::spawn(group_driver.clone().run(signals_rx));
        let presence = Arc::new(presence::PresenceDriver::new(store.clone(), dm.clone(), contacts.clone(), outbox.clone()));
        let rt = Self {
            config,
            store,
            secrets,
            identity,
            relays,
            profiles,
            contacts,
            nip05,
            dm,
            media,
            previews,
            group_driver,
            group_signals,
            outbox,
            dispatcher,
            ui,
            session: Mutex::new(None),
            manifest_remote: std::sync::RwLock::new(ManifestRemote::project(Arc::new(
                messenger_transport::HttpManifestFetcher::new()?,
            ))),
            net,
            link: link::LinkWatch::default(),
            presence,
            avatars,
            photo_slots: Arc::new(tokio::sync::Semaphore::new(media::PHOTO_SLOTS)),
            preparing: media::Preparing::default(),
            calls,
            group_calls,
        };
        if let Err(e) = rt.seed_media_servers().await {
            eprintln!("messenger: media servers from the manifest not applied: {e}");
        }
        if let Err(e) = rt.seed_call_nodes().await {
            eprintln!("messenger: call nodes from the manifest not applied: {e}");
        }
        if let (Some(keys), true) = (signer, rt.servers_chosen().await?) {
            rt.start_session(keys).await?;
        }
        Ok(rt)
    }

    pub fn config(&self) -> &MessengerConfig {
        &self.config
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    pub fn secrets(&self) -> &Arc<dyn SecretStore> {
        &self.secrets
    }

    pub fn identity(&self) -> &IdentityService {
        &self.identity
    }

    pub fn relays(&self) -> &RelayService {
        &self.relays
    }

    pub fn outbox(&self) -> &Outbox {
        &self.outbox
    }

    pub fn profiles(&self) -> &ProfileService {
        &self.profiles
    }

    pub fn contacts(&self) -> &ContactService {
        &self.contacts
    }

    pub fn nip05(&self) -> &Nip05Service {
        &self.nip05
    }

    /// Subscribe to UI events (`inbound.dm`, `inbound.meta`, `ignored`,
    /// `error`, `notify`). Lagging receivers drop old events.
    pub fn ui_events(&self) -> broadcast::Receiver<UiEvent> {
        self.ui.subscribe()
    }

    /// Public key of the running session, if any.
    pub async fn session_pubkey(&self) -> Option<PubKey> {
        self.session
            .lock()
            .await
            .as_ref()
            .and_then(|s| PubKey::parse(&s.keys.public_key().to_hex()))
    }

    async fn start_session(&self, keys: Keys) -> Result<()> {
        // A start is shown as connected, and what waited while the app was
        // off gets a try before its deadline is looked at.
        self.link.reset();
        self.outbox.hold_expiry(SystemClock.now().secs() + link::HOLD_EXPIRY_SECS);
        let pool = self.relays.pool().await;
        let keys_for_dm = keys.clone();
        self.presence.session_started().await;
        let session = Session::start(
            self.store.clone(),
            pool,
            keys,
            self.outbox.clone(),
            self.ui.clone(),
            self.dispatcher.clone(),
            self.dm.clone(),
            self.group_driver.groups.clone(),
            self.presence.clone(),
            self.avatars.clone(),
        )
        .await?;
        self.group_driver.groups.set_signer(Some(keys_for_dm.clone()));
        self.calls.service.set_signer(Some(keys_for_dm.clone()));
        self.group_calls.service.set_signer(Some(keys_for_dm.clone()));
        self.dm.set_signer(Some(keys_for_dm));
        *self.session.lock().await = Some(session);
        self.resubscribe_meta().await?;
        self.group_driver.session_started().await;
        if let Err(e) = self.publish_dm_relays(false).await {
            eprintln!("messenger: inbox relay list not published: {e}");
        }
        // The keys are here: what the closing of the app interrupted goes on.
        if self.config.resume_transfers {
            match self.resume_interrupted().await {
                Ok(0) => {}
                Ok(n) => eprintln!("messenger media: {n} interrupted transfers go on"),
                Err(e) => eprintln!("messenger media: interrupted transfers not taken up: {e}"),
            }
        }
        Ok(())
    }

    /// Tell the world where we read DMs (kind 10050): our enabled write
    /// relays. Published only when the set changed, unless `force`.
    pub async fn publish_dm_relays(&self, force: bool) -> Result<()> {
        const KEY: &str = "dm.relays_published";
        let keys = self.session_keys().await?;
        let mut urls: Vec<String> = self
            .relays
            .list()
            .await?
            .into_iter()
            .filter(|r| r.enabled && r.read)
            .map(|r| r.url)
            .collect();
        urls.sort();
        let fingerprint = format!("{}|{}", keys.public_key().to_hex(), urls.join(","));
        if urls.is_empty() || (!force && settings::get(&self.store, KEY).await?.as_deref() == Some(fingerprint.as_str())) {
            return Ok(());
        }
        let mut builder = EventBuilder::new(Kind::InboxRelays, "");
        for u in &urls {
            builder = builder.tag(Tag::parse(["relay", u.as_str()]).map_err(|e| MessengerError::Invalid(e.to_string()))?);
        }
        let event = builder.finalize(&keys).map_err(|e| MessengerError::Crypto(e.to_string()))?;
        let event = WireEvent {
            id: EventId::parse(&event.id.to_hex()).expect("event id is hex"),
            json: serde_json::to_value(&event)?,
        };
        self.outbox.enqueue(Outbound::PublishOwn { event }).await?;
        settings::set(&self.store, KEY, &fingerprint).await?;
        Ok(())
    }

    /// Profiles of everyone we care about (contacts + me) and my follow list.
    async fn resubscribe_meta(&self) -> Result<()> {
        let Some(me) = self.session_pubkey().await else { return Ok(()) };
        let mut authors: Vec<PubKey> = self
            .contacts
            .list()
            .await?
            .into_iter()
            .filter_map(|c| PubKey::parse(&c.pubkey))
            .collect();
        authors.push(me.clone());
        let pool = self.relays.pool().await;
        pool.send(Outbound::Subscribe {
            id: SubId(filters::SUB_PROFILES.into()),
            filter: filters::profiles(&authors),
            scope: Scope::Own,
        })
        .await?;
        pool.send(Outbound::Subscribe {
            id: SubId(filters::SUB_MY_FOLLOWS.into()),
            filter: filters::my_follows(&me),
            scope: Scope::Own,
        })
        .await?;
        // Where contacts and chat peers want their DMs delivered.
        let mut peers = authors;
        for c in self.dm.list_chats(true).await? {
            if let Some(pk) = c.peer_pubkey.as_deref().and_then(PubKey::parse) {
                if !peers.contains(&pk) {
                    peers.push(pk);
                }
            }
        }
        pool.send(Outbound::Subscribe {
            id: SubId(filters::SUB_DM_RELAYS.into()),
            filter: filters::dm_relays(&peers),
            scope: Scope::Own,
        })
        .await?;
        Ok(())
    }

    // ─── Chats / DM ─────────────────────────────────────────────────────────

    pub fn dm(&self) -> &DmService {
        &self.dm
    }

    /// Open (creating if needed) the chat with `peer` (hex or npub) and
    /// start following the peer's profile and inbox relays.
    pub async fn chat_open(&self, peer: &str) -> Result<ChatView> {
        let pk = messenger_contacts::book::parse_key(peer)?;
        let existed = self.dm.chat(&messenger_store::chats::dm_chat_id(pk.as_hex())).await?.is_some();
        let view = self.dm.open_chat(&pk).await?;
        if !existed && self.session.lock().await.is_some() {
            let _ = self.request_profile(&pk).await;
            let _ = self.resubscribe_meta().await;
        }
        Ok(view)
    }

    async fn publish_prepared(&self, p: Prepared) -> Result<MessageView> {
        let peer = p.message.chat_id.strip_prefix("dm:").and_then(PubKey::parse);
        // A new message is tried for an hour, both copies; an edit or a
        // deletion until it leaves.
        let local_id = self.enqueue_copy(p.to_peer, p.expiring).await?;
        self.dm.attach_outbox(&p.tracking_id, &local_id).await?;
        if let Some(own) = p.to_self {
            self.enqueue_copy(own, p.expiring).await?;
        }
        // A request: the accept goes out after the text, and the peer
        // joins the address book.
        for out in p.followups {
            self.outbox.enqueue(out).await?;
        }
        if p.became_contact {
            if let (Some(me), Some(peer)) = (self.session_pubkey().await, &peer) {
                let _ = self.contacts.add(&me, peer.as_hex(), None).await;
                let _ = self.resubscribe_meta().await;
            }
        }
        for ev in p.events {
            let _ = self.ui.send(ev);
        }
        // Published in the background: the caller gets the stored message at
        // once and the status follows as an event.
        self.outbox.kick();
        for ev in self.dm.sync_statuses().await? {
            let _ = self.ui.send(ev);
        }
        Ok(self.dm.message(&p.message.id).await?.unwrap_or(p.message))
    }

    /// Send a text to `to`: a person (hex or npub) or a group (`group:<id>`).
    pub async fn dm_send_text(&self, to: &str, text: &str, reply_to: Option<&str>) -> Result<MessageView> {
        let keys = self.session_keys().await?;
        if let Some(group) = to.strip_prefix("group:") {
            return self.group_send_text(group, text, reply_to).await;
        }
        let peer = messenger_contacts::book::parse_key(to)
            .map_err(|_| MessengerError::Invalid("recipient must be an npub or 64-hex public key".into()))?;
        let first = self.dm.chat(&messenger_store::chats::dm_chat_id(peer.as_hex())).await?.is_none();
        let prepared = self.dm.prepare_text(&keys, &peer, text, reply_to).await?;
        if first {
            let _ = self.request_profile(&peer).await;
            let _ = self.resubscribe_meta().await;
        }
        self.publish_prepared(prepared).await
    }

    pub async fn dm_edit(&self, message_id: &str, text: &str) -> Result<MessageView> {
        let keys = self.session_keys().await?;
        if self.is_group_message(message_id).await? {
            return self.group_edit(message_id, text).await;
        }
        let prepared = self.dm.prepare_edit(&keys, message_id, text).await?;
        self.publish_prepared(prepared).await
    }

    /// `for_everyone` retracts our own message at the peer too; otherwise
    /// the message is only hidden on this device.
    pub async fn dm_delete(&self, message_id: &str, for_everyone: bool) -> Result<()> {
        if for_everyone && self.is_group_message(message_id).await? {
            return self.group_delete(message_id).await;
        }
        if for_everyone {
            let keys = self.session_keys().await?;
            let prepared = self.dm.prepare_delete(&keys, message_id).await?;
            self.publish_prepared(prepared).await?;
        } else {
            let note = self.dm.delete_local(message_id).await?;
            self.tell_own_devices(&note).await;
        }
        // Its file goes no further, in either direction.
        self.cancel_transfers_of_message(message_id).await;
        Ok(())
    }

    /// The chat was read on this device; my other devices are told.
    pub async fn chat_mark_read(&self, chat_id: &str) -> Result<()> {
        if let Some(note) = self.dm.mark_read(chat_id).await? {
            self.tell_own_devices(&note).await;
            // The peer hears of it now, not at the next tick of the session.
            if let Err(e) = self.flush_receipts().await {
                eprintln!("messenger receipts: {e}");
            }
        }
        Ok(())
    }

    /// A note for my other devices (`messenger_dm::own`). What is done here
    /// stays done when the note cannot leave: locked, or a broken outbox.
    async fn tell_own_devices(&self, note: &messenger_core::Envelope) {
        let Ok(keys) = self.session_keys().await else { return };
        let sent = async {
            let event = messenger_dm::wrap::wrap_own(&keys, &note.encode(), SystemClock.now().secs())?;
            self.enqueue_and_pump(Outbound::PublishOwn { event }).await
        };
        if let Err(e) = sent.await {
            let _ = self.ui.send(UiEvent {
                name: "error".into(),
                payload: serde_json::json!({ "family": "own", "error": e.to_string() }),
            });
        }
    }

    pub async fn dm_relation(&self, peer: &str) -> Result<RelationView> {
        let pk = messenger_contacts::book::parse_key(peer)?;
        self.dm.relation(&pk).await
    }

    /// Relationship action: request, accept, decline, block, unblock,
    /// remove. Publishes the signals and keeps the address book in step.
    pub async fn dm_act(&self, peer: &str, action: Action) -> Result<RelationView> {
        let keys = self.session_keys().await?;
        let pk = messenger_contacts::book::parse_key(peer)?;
        let result = self.dm.act(&keys, &pk, action).await?;
        for out in result.outbounds {
            self.outbox.enqueue(out).await?;
        }
        let me = PubKey::parse(&keys.public_key().to_hex()).expect("valid pubkey");
        let is_contact = self.contacts.is_contact(&pk).await?;
        match action {
            Action::Accept | Action::Request => {
                if !is_contact {
                    let _ = self.contacts.add(&me, pk.as_hex(), None).await;
                    let _ = self.request_profile(&pk).await;
                    let _ = self.resubscribe_meta().await;
                }
            }
            Action::Remove if is_contact => self.contacts.remove(&pk).await?,
            _ => {}
        }
        // Whoever was told my presence key may no longer watch it, and the
        // page no longer shows the peer (`presence_list` leaves it out).
        if matches!(action, Action::Remove | Action::Block) {
            if let Err(e) = self.presence_rotate().await {
                eprintln!("messenger presence: key not moved: {e}");
            }
            let _ = self.ui.send(UiEvent {
                name: presence::UI_EVENT_PRESENCE_KEYS_CHANGED.into(),
                payload: serde_json::json!({ "peer": pk.as_hex() }),
            });
        }
        for ev in result.events {
            let _ = self.ui.send(ev);
        }
        let _ = self.ui.send(UiEvent { name: "chats.updated".into(), payload: serde_json::json!({}) });
        self.outbox.kick();
        Ok(result.relation)
    }

    pub async fn dm_blocked(&self) -> Result<Vec<String>> {
        self.dm.blocked_peers().await
    }

    /// Put a failed message back in front of the queue.
    pub async fn dm_retry(&self, message_id: &str) -> Result<()> {
        let local_id = self.dm.outbox_id_for_retry(message_id).await?;
        self.outbox.retry_now(&local_id).await?;
        self.outbox.kick();
        for ev in self.dm.sync_statuses().await? {
            let _ = self.ui.send(ev);
        }
        Ok(())
    }

    // ─── Profiles / contacts ────────────────────────────────────────────────

    /// Ask relays for one profile (answer arrives through ingress as
    /// `profile.updated`). Cheap, idempotent per pubkey.
    pub async fn request_profile(&self, pubkey: &PubKey) -> Result<()> {
        let pool = self.relays.pool().await;
        pool.send(Outbound::Subscribe {
            id: SubId(format!("profile-{}", pubkey.short())),
            filter: filters::profile_of(pubkey),
            scope: Scope::Own,
        })
        .await?;
        Ok(())
    }

    pub async fn my_profile(&self) -> Result<Option<ProfileView>> {
        match self.session_pubkey().await {
            Some(me) => self.profiles.get(&me).await,
            None => Ok(None),
        }
    }

    /// Sign and publish our kind 0; the cache is updated immediately.
    /// `picture` stays what the last kind 0 said: only the avatar's own
    /// commands and its keeper (`crate::avatars`) change it, so a profile
    /// saved on a device that has not yet taken on an avatar set elsewhere
    /// does not take it away.
    pub async fn publish_own_profile(&self, input: &ProfileInput) -> Result<ProfileView> {
        let keys = self.session_keys().await?;
        let event = self.profiles.build_own(&keys, input, messenger_contacts::Picture::Keep).await?;
        self.enqueue_and_pump(Outbound::PublishOwn { event }).await?;
        let me = PubKey::parse(&keys.public_key().to_hex()).expect("valid pubkey");
        self.profiles.get(&me).await?.ok_or_else(|| MessengerError::Storage("own profile missing after publish".into()))
    }

    /// Sign and publish our kind 3 from `msg_follows`.
    pub async fn publish_follow_list(&self) -> Result<()> {
        let keys = self.session_keys().await?;
        let event = self.contacts.build_follow_list(&keys).await?;
        self.enqueue_and_pump(Outbound::PublishOwn { event }).await
    }

    /// Add a contact (hex/npub or NIP-05), fetch its profile, resubscribe.
    pub async fn contact_add(&self, key_or_nip05: &str, nickname: Option<&str>) -> Result<ContactView> {
        let me = self.session_pubkey().await.ok_or(MessengerError::NotLoggedIn)?;
        let input = key_or_nip05.trim();
        let key = if input.contains('@') || (!input.starts_with("npub1") && input.len() != 64 && input.contains('.')) {
            self.nip05.resolve(input).await?.as_hex().to_string()
        } else {
            input.to_string()
        };
        let view = self.contacts.add(&me, &key, nickname).await?;
        if let Some(pk) = PubKey::parse(&view.pubkey) {
            // Silent unless there is a past to mend (see the matrix).
            let _ = self.dm_act(pk.as_hex(), Action::Request).await;
        }
        if let Some(pk) = PubKey::parse(&view.pubkey) {
            let _ = self.request_profile(&pk).await;
        }
        let _ = self.resubscribe_meta().await;
        Ok(view)
    }

    pub async fn contact_update(&self, pubkey: &PubKey, patch: &ContactPatch) -> Result<ContactView> {
        self.contacts.update(pubkey, patch).await
    }

    pub async fn contact_remove(&self, pubkey: &PubKey) -> Result<()> {
        // The signal is derived from the state before the removal. Without
        // it (no session) the presence key still moves on.
        if self.dm_act(pubkey.as_hex(), Action::Remove).await.is_err() {
            if let Err(e) = self.presence_rotate().await {
                eprintln!("messenger presence: key not moved: {e}");
            }
        }
        self.contacts.remove(pubkey).await
    }

    /// Follow/unfollow and republish kind 3.
    pub async fn contact_set_followed(&self, pubkey: &PubKey, followed: bool) -> Result<()> {
        self.contacts.set_followed(pubkey, followed).await?;
        self.publish_follow_list().await
    }

    /// Check the profile's NIP-05 claim and record the result.
    pub async fn verify_nip05(&self, pubkey: &PubKey) -> Result<bool> {
        let Some(profile) = self.profiles.get(pubkey).await? else {
            return Err(MessengerError::Invalid("profile unknown".into()));
        };
        let Some(nip05) = profile.nip05 else {
            return Err(MessengerError::Invalid("profile has no NIP-05".into()));
        };
        let ok = self.nip05.verify(&nip05, pubkey).await?;
        let now = messenger_core::traits::SystemClock.now().secs();
        self.profiles.set_nip05_verified(pubkey, if ok { Some(now) } else { None }).await?;
        Ok(ok)
    }

    async fn session_keys(&self) -> Result<Keys> {
        self.session.lock().await.as_ref().map(|s| s.keys.clone()).ok_or(MessengerError::NotLoggedIn)
    }

    async fn enqueue_copy(&self, out: Outbound, expiring: bool) -> Result<String> {
        if expiring {
            self.outbox.enqueue_message(out).await
        } else {
            self.outbox.enqueue(out).await
        }
    }

    async fn enqueue_and_pump(&self, out: Outbound) -> Result<()> {
        self.outbox.enqueue(out).await?;
        self.outbox.kick();
        Ok(())
    }

    async fn stop_session(&self) {
        // The emoji counted since the last map: queued now, sent by the next
        // session if not by this one.
        let keys = self.session.lock().await.as_ref().map(|s| s.keys.clone());
        if let Some(keys) = &keys {
            if let Err(e) = reactions::send_emoji_snapshot(&self.dm, &self.outbox, keys, SystemClock.now().secs(), true).await {
                eprintln!("messenger emoji: {e}");
            }
        }
        // The pump of the session goes before the call is ended: the end
        // is sent below by one pump of our own, not by one stopped halfway.
        let session = self.session.lock().await.take();
        if let Some(s) = &session {
            s.stop();
        }
        // A call under way ends as a hang-up while the keys are still here:
        // the peer hears of it now, not after a timeout of its connection.
        // The room of a group call is left the same way.
        let goodbye = keys.is_some() && (self.end_call_before_leaving().await | self.leave_group_call_before_leaving().await);
        self.dm.set_signer(None);
        self.group_driver.groups.set_signer(None);
        self.calls.service.set_signer(None);
        self.group_calls.service.set_signer(None);
        if goodbye {
            let pool = self.relays.pool().await;
            if tokio::time::timeout(calls::GOODBYE_WAIT, self.outbox.pump(pool.as_ref())).await.is_err() {
                eprintln!("messenger calls: the end of the call waits for the next session");
            }
        }
    }

    /// Re-check which key should sign and (re)start the session when that
    /// changed. Call after identity create/import/delete and after the host
    /// unlocks secrets. Idempotent and cheap when nothing changed.
    pub async fn refresh_signer(&self) -> Result<bool> {
        // No session until the user has chosen whose servers to use: it
        // would publish our inbox relays.
        let signer = match self.servers_chosen().await? {
            true => load_signer(&self.identity).await,
            false => None,
        };
        let wanted = signer.as_ref().map(|k| k.public_key().to_hex());
        let current = self.session.lock().await.as_ref().map(|s| s.keys.public_key().to_hex());
        if wanted == current {
            return Ok(false);
        }
        self.stop_session().await;
        // The pool binds its authenticator at construction: rebuild it.
        self.relays.set_signer(signer.clone()).await?;
        if let Some(keys) = signer {
            self.start_session(keys).await?;
        }
        Ok(true)
    }

    async fn servers_chosen(&self) -> Result<bool> {
        Ok(self.relays.servers_mode().await?.is_some())
    }

    /// Relay set changed (add/remove/toggle): the pool object is the same,
    /// nostr-sdk re-applies subscriptions to new relays, nothing to restart.
    /// Kept as an explicit hook for hosts.
    pub async fn relays_changed(&self) -> Result<()> {
        Ok(())
    }

    pub async fn status(&self) -> Result<RuntimeStatus> {
        let relays = self.relays.list().await?;
        let (session_active, ingress) = match self.session.lock().await.as_ref() {
            Some(s) => {
                let (received, duplicates, dispatched, dm, ignored) = s.stats();
                (true, IngressCounters { received, duplicates, dispatched, dm, ignored })
            }
            None => (false, IngressCounters::default()),
        };
        Ok(RuntimeStatus {
            version: messenger_core::VERSION.to_string(),
            data_dir: self.config.data_dir().to_string_lossy().into_owned(),
            schema_version: self.store.schema_version().await?,
            secrets_unlocked: self.secrets.is_unlocked().await,
            identity_present: self.identity.get().await?.is_some(),
            session_active,
            relays_total: relays.iter().filter(|r| r.enabled).count(),
            relays_connected: relays.iter().filter(|r| r.state == RelayState::Connected).count(),
            silent_mode: self.relays.is_silent().await?,
            link: self.link.state(),
            manifest_serial: self.relays.manifest_serial().await?,
            region: self.relays.region().await?,
            servers_mode: self.relays.servers_mode().await?,
            ingress,
            outbox_pending: self.outbox.pending().await?,
        })
    }

    /// Stop background work, close the door of the bridges and the
    /// database. Idempotent.
    pub async fn shutdown(&self) {
        // Transfers here pause, and the data folder is let go: a runtime
        // started again in this process recovers as after a restart.
        self.media.release();
        self.stop_session().await;
        self.group_signals.abort();
        self.calls.stop();
        self.group_calls.stop();
        self.relays.shutdown().await;
        self.net.shutdown();
        self.store.close().await;
    }
}

/// Keys for the pool signer, or `None` when there is no identity or the
/// secrets are locked. Locked is not an error here: the pool works without
/// a signer until the host unlocks and `refresh_signer` runs.
async fn load_signer(identity: &IdentityService) -> Option<Keys> {
    match identity.load_keys().await {
        Ok(k) => Some(k),
        Err(MessengerError::NotLoggedIn) | Err(MessengerError::SecretsLocked) => None,
        Err(e) => {
            eprintln!("messenger: signer unavailable: {e}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use messenger_testkit::MemorySecretStore;

    #[tokio::test]
    async fn starts_reports_status_and_shuts_down() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = MessengerConfig::new(dir.path().join("messenger"));
        let secrets = Arc::new(MemorySecretStore::unlocked());
        let rt = MessengerRuntime::start(cfg.clone(), secrets).await.unwrap();
        servers::use_veydan_offline(&rt).await;

        let st = rt.status().await.unwrap();
        assert_eq!(st.version, messenger_core::VERSION);
        assert!(st.schema_version >= 4);
        assert!(st.secrets_unlocked);
        assert!(!st.identity_present);
        assert!(!st.session_active);
        assert!(st.manifest_serial.is_some());
        assert!(st.relays_total >= 1);
        assert_eq!(st.outbox_pending, 0);
        assert!(cfg.db_path().exists());

        rt.shutdown().await;
    }

    #[tokio::test]
    async fn media_servers_follow_the_manifest_and_keep_the_users_own() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = MessengerConfig::new(dir.path().join("messenger"));
        let secrets = Arc::new(MemorySecretStore::unlocked());
        let rt = MessengerRuntime::start(cfg.clone(), secrets.clone()).await.unwrap();
        servers::use_veydan_offline(&rt).await;
        // What an earlier manifest brought, and what the user added.
        let old = MediaServerInput {
            id: Some("veydan-node-1-s3".into()),
            kind: "s3".into(),
            url: "https://node-1.veydan.net:9000".into(),
            bucket: Some("veydan-media".into()),
            source: Some("manifest".into()),
            ..Default::default()
        };
        rt.media_server_put(old).await.unwrap();
        rt.media_server_put(MediaServerInput { kind: "blossom".into(), url: "https://mine.example".into(), ..Default::default() })
            .await
            .unwrap();
        rt.shutdown().await;

        let rt = MessengerRuntime::start(cfg, secrets).await.unwrap();
        let ids: Vec<String> = rt.media_servers().await.unwrap().into_iter().map(|s| s.id).collect();
        assert!(!ids.contains(&"veydan-node-1-s3".to_string()), "gone from the manifest, gone from the app");
        let manifest = messenger_transport::Manifest::parse_content(messenger_transport::EMBEDDED_MANIFEST_JSON).unwrap();
        let of_manifest = &manifest.media_for_region("default")[0].id;
        assert!(ids.contains(of_manifest), "{of_manifest} from the manifest: {ids:?}");
        assert!(ids.contains(&"blossom-mine-example".to_string()), "the user's own server stays: {ids:?}");
        rt.shutdown().await;
    }

    #[tokio::test]
    async fn locked_secret_store_is_reported_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = MessengerConfig::new(dir.path().join("messenger"));
        let rt = MessengerRuntime::start(cfg, Arc::new(MemorySecretStore::locked()))
            .await
            .unwrap();
        assert!(!rt.status().await.unwrap().secrets_unlocked);
        rt.shutdown().await;
    }

    #[tokio::test]
    async fn session_follows_identity_and_lock_state() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = MessengerConfig::new(dir.path().join("messenger"));
        let secrets = Arc::new(MemorySecretStore::unlocked());
        let rt = MessengerRuntime::start(cfg, secrets.clone()).await.unwrap();
        // Keep the test offline: sends fail fast instead of waiting for relays.
        rt.relays().set_silent(true).await.unwrap();
        servers::use_veydan_offline(&rt).await;
        assert!(!rt.refresh_signer().await.unwrap(), "nothing to do without identity");
        assert!(matches!(rt.dm_send_text(&"ab".repeat(32), "x", None).await, Err(MessengerError::NotLoggedIn)));

        let created = rt.identity().create("pw").await.unwrap();
        assert!(rt.refresh_signer().await.unwrap(), "session started with the new key");
        assert!(rt.status().await.unwrap().session_active);
        assert_eq!(rt.session_pubkey().await.unwrap(), created.identity.pubkey);
        assert!(!rt.refresh_signer().await.unwrap(), "idempotent");

        // Sending queues the wrap; relays may be unreachable in tests, the
        // outbox keeps it either way.
        let peer = Keys::generate().public_key().to_hex();
        let sent = rt.dm_send_text(&peer, "hello", None).await.unwrap();
        let id = sent.id.clone();
        assert_eq!(rt.dm().list_chats(false).await.unwrap().len(), 1);
        assert_eq!(rt.dm_edit(&id, "hello!").await.unwrap().text.as_deref(), Some("hello!"));
        rt.dm_delete(&id, true).await.unwrap();
        assert!(rt.dm().message(&id).await.unwrap().unwrap().deleted);
        assert!(!id.is_empty());
        assert!(rt.dm_send_text("not a key", "x", None).await.is_err());

        // Contacts and own profile round-trip through the runtime.
        let bob = Keys::generate();
        let c = rt.contact_add(&bob.public_key().to_hex(), Some("Bob")).await.unwrap();
        assert_eq!(c.nickname.as_deref(), Some("Bob"));
        assert!(rt.contact_add(created.identity.pubkey.as_hex(), None).await.is_err(), "no self-contact");
        let me = rt.publish_own_profile(&ProfileInput { name: Some("alice".into()), ..Default::default() }).await.unwrap();
        assert_eq!(me.name.as_deref(), Some("alice"));
        assert_eq!(rt.my_profile().await.unwrap().unwrap().pubkey, created.identity.pubkey.as_hex());
        rt.contact_set_followed(&PubKey::parse(&bob.public_key().to_hex()).unwrap(), true).await.unwrap();
        assert!(rt.contacts().list().await.unwrap()[0].followed);
        assert!(rt.verify_nip05(&PubKey::parse(&bob.public_key().to_hex()).unwrap()).await.is_err(), "no profile yet");

        secrets.set_unlocked(false);
        assert!(rt.refresh_signer().await.unwrap(), "locked secrets stop the session");
        assert!(!rt.status().await.unwrap().session_active);

        secrets.set_unlocked(true);
        assert!(rt.refresh_signer().await.unwrap());
        rt.identity().delete().await.unwrap();
        assert!(rt.refresh_signer().await.unwrap());
        assert!(!rt.status().await.unwrap().session_active);
        rt.shutdown().await;
    }
}
