// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The redesign of the channels (2026-10-23): the look of a section and
//! its rename (`scope_set`), the hidden section whose meta only its
//! members read, and the edit of a file's caption with its date
//! (internal/communities-wire.md §4, §14.1).

use crate::levels::{scope_shown, ScopeChange};
use crate::op::{History, JoinMode, LevelCan, OpBody};
use crate::roles::Role;
use crate::service::NewChannel;
use crate::state::SCOPE_EVERYONE;
use crate::testkit::{Node, World, H1, H2};
use crate::transport::FeedItem;
use crate::wire::{ScopeLook, KIND_CONTENT};
use keydist::SchemeId;
use messenger_core::{Envelope, PubKey, TextFormat};

fn new_channel() -> NewChannel {
    NewChannel {
        name: "News".into(),
        about: String::new(),
        picture: String::new(),
        join: JoinMode::Open,
        history: History::Open,
        relays: vec!["wss://r1.example".into(), "wss://r2.example".into()],
        storage: vec![H1.into(), H2.into()],
    }
}

async fn tick(n: &Node) {
    n.svc.tick().await.unwrap_or_else(|e| panic!("tick: {e}"));
}

fn pk(n: &Node) -> PubKey {
    n.me.clone()
}

async fn join_by_link(w: &World, owner: &Node, reader: &Node, cid: &str, all: &[&Node]) {
    let link = owner.svc.share_link(cid).await.unwrap_or_else(|e| panic!("link: {e}"));
    w.pump(&[reader]).await;
    reader.svc.open_link(&reader.keys, &link, "").await.unwrap_or_else(|e| panic!("open link: {e}"));
    w.pump(all).await;
}

/// A channel of the owner, an admin and two readers, all in by the link;
/// a snapshot names the readers.
async fn channel(w: &World) -> (Node, Node, Node, Node, String) {
    let (owner, admin, a, b) = (w.node().await, w.node().await, w.node().await, w.node().await);
    let all = [&owner, &admin, &a, &b];
    let (cid, _) = owner.svc.create(&owner.keys, new_channel()).await.unwrap_or_else(|e| panic!("create: {e}"));
    w.pump(&[&owner]).await;
    for n in [&admin, &a, &b] {
        join_by_link(w, &owner, n, &cid, &all).await;
    }
    owner.svc.act(&owner.keys, &cid, OpBody::SetRole { who: pk(&admin), role: Role::Admin }).await.unwrap_or_else(|e| panic!("{e}"));
    w.clock.advance(600);
    tick(&owner).await;
    w.pump(&all).await;
    (owner, admin, a, b, cid)
}

async fn settle(w: &World, owner: &Node, all: &[&Node]) {
    for _ in 0..2 {
        w.pump(all).await;
        tick(owner).await;
    }
    w.pump(all).await;
}

async fn names(n: &Node, cid: &str) -> crate::levels::Names {
    let log = n.svc.log(cid).await.ok().flatten().unwrap_or_else(|| panic!("journal"));
    n.svc.names(&log).await.unwrap_or_else(|e| panic!("names: {e}"))
}

async fn shown(n: &Node, cid: &str, scope_id: &str) -> bool {
    let log = n.svc.log(cid).await.ok().flatten().unwrap_or_else(|| panic!("journal"));
    let names = n.svc.names(&log).await.unwrap_or_else(|e| panic!("names: {e}"));
    scope_shown(log.state(), &names, scope_id, Some(&n.me))
}

fn err_text<T>(r: messenger_core::Result<T>) -> String {
    r.err().map(|e| e.to_string()).unwrap_or_default()
}

/// A section renamed and given a look by the owner: every reader reads
/// the newest meta, the fields of before stay where a change leaves them
/// out; the order of the sections is the owner's; nobody else renames.
#[tokio::test]
async fn the_owner_renames_a_section_and_gives_it_a_look() {
    let w = World::new();
    let (owner, admin, a, b, cid) = channel(&w).await;
    let all = [&owner, &admin, &a, &b];
    let look = ScopeLook { icon: Some("star".into()), color: Some(3), note: Some("For patrons".into()), ..ScopeLook::default() };
    let (gold, _) = owner.svc.scope_add_with(&owner.keys, &cid, "Premium", look.clone(), SchemeId::SdV1, History::Open, false).await.unwrap_or_else(|e| panic!("scope_add: {e}"));
    let (news, _) = owner.svc.scope_add(&owner.keys, &cid, "News", SchemeId::SdV1, History::Open).await.unwrap_or_else(|e| panic!("scope_add: {e}"));
    settle(&w, &owner, &all).await;
    let seen = names(&a, &cid).await;
    assert_eq!((seen.scope(&gold), seen.look(&gold)), ("Premium".to_string(), look.clone()), "the look travels with the title");
    assert!(seen.look(&news).is_empty(), "a section of no look");
    // Renamed: the title and the colour change, the icon and the note stay.
    let change = ScopeChange { title: Some("Gold".into()), color: Some(Some(7)), ..ScopeChange::default() };
    owner.svc.scope_set(&owner.keys, &cid, &gold, &change).await.unwrap_or_else(|e| panic!("scope_set: {e}"));
    settle(&w, &owner, &all).await;
    for n in [&a, &b, &admin] {
        let seen = names(n, &cid).await;
        assert_eq!(seen.scope(&gold), "Gold");
        assert_eq!(seen.look(&gold), ScopeLook { color: Some(7), ..look.clone() });
    }
    // The icon and the note taken away, the colour too.
    let change = ScopeChange { icon: Some(String::new()), note: Some(String::new()), color: Some(None), ..ScopeChange::default() };
    owner.svc.scope_set(&owner.keys, &cid, &gold, &change).await.unwrap_or_else(|e| panic!("scope_set: {e}"));
    settle(&w, &owner, &all).await;
    assert!(names(&a, &cid).await.look(&gold).is_empty());
    // The order: everyone, then news, then gold, by one call.
    owner.svc.scopes_order(&owner.keys, &cid, &[SCOPE_EVERYONE.to_string(), news.clone(), gold.clone()]).await.unwrap_or_else(|e| panic!("order: {e}"));
    settle(&w, &owner, &all).await;
    let seen = names(&b, &cid).await;
    assert_eq!([SCOPE_EVERYONE, news.as_str(), gold.as_str()].map(|s| seen.look(s).order), [Some(0), Some(1), Some(2)]);
    assert_eq!(seen.scope(&gold), "Gold", "a change of the order keeps the title");
    assert_eq!(seen.scope(SCOPE_EVERYONE), "", "everyone stays without a title of its own");
    // Nobody but the owner and the admins (2026-10-25): a reader is refused before anything goes out, and by the journal.
    let ops = owner.svc.log(&cid).await.ok().flatten().map(|l| l.ordered().count()).unwrap_or(0);
    assert!(err_text(a.svc.scope_set(&a.keys, &cid, &gold, &ScopeChange { title: Some("Mine".into()), ..ScopeChange::default() }).await).contains("not permitted"));
    assert!(a.svc.act(&a.keys, &cid, OpBody::ScopeSet { scope_id: gold.clone(), hidden: None }).await.is_err(), "the journal refuses it");
    assert!(err_text(a.svc.scope_add_with(&a.keys, &cid, "Secret", ScopeLook::default(), SchemeId::SdV1, History::Open, true).await.map(|_| ())).contains("not permitted"));
    w.pump(&all).await;
    assert_eq!(owner.svc.log(&cid).await.ok().flatten().map(|l| l.ordered().count()).unwrap_or(0), ops, "nothing of theirs in the journal");
    // An admin renames the section, and everyone reads the new title.
    admin.svc.scope_set(&admin.keys, &cid, &gold, &ScopeChange { title: Some("Gold+".into()), ..ScopeChange::default() }).await.unwrap_or_else(|e| panic!("admin's scope_set: {e}"));
    settle(&w, &owner, &all).await;
    assert_eq!(names(&b, &cid).await.scope(&gold), "Gold+");
    assert_eq!(names(&b, &cid).await.look(&gold).order, Some(2), "the look stays as it was");
    // A look out of its bounds is refused.
    let bad = ScopeChange { icon: Some("Star!".into()), ..ScopeChange::default() };
    assert!(err_text(owner.svc.scope_set(&owner.keys, &cid, &gold, &bad).await).contains("scope icon"));
    assert!(err_text(owner.svc.scope_set(&owner.keys, &cid, &gold, &ScopeChange { color: Some(Some(10)), ..ScopeChange::default() }).await).contains("scope color"));
    assert!(err_text(owner.svc.scope_set(&owner.keys, &cid, SCOPE_EVERYONE, &ScopeChange { hidden: Some(true), ..ScopeChange::default() }).await).contains("scope everyone"));
    for n in all {
        assert!(n.errors().is_empty(), "{:?}", n.errors());
    }
}

/// A hidden section (owner's decision 2026-10-23): its title and look go
/// into its own feed, so a reader outside it reads nothing of it and is
/// not shown it at all; a reader whose level gives it reads it. A section
/// hidden later says its new title only inside.
#[tokio::test]
async fn a_hidden_section_is_shown_to_its_members_only() {
    let w = World::new();
    let (owner, admin, a, b, cid) = channel(&w).await;
    let all = [&owner, &admin, &a, &b];
    let look = ScopeLook { icon: Some("shield".into()), color: Some(1), ..ScopeLook::default() };
    let (secret, _) = owner.svc.scope_add_with(&owner.keys, &cid, "Backstage", look.clone(), SchemeId::SdV1, History::Open, true).await.unwrap_or_else(|e| panic!("scope_add: {e}"));
    let (lvl, _) = owner.svc.level_set(&owner.keys, &cid, None, "Crew", vec![secret.clone()], LevelCan::BASE).await.unwrap_or_else(|e| panic!("level: {e}"));
    w.pump(&all).await;
    owner.svc.set_levels(&owner.keys, &cid, &pk(&a), &[lvl]).await.unwrap_or_else(|e| panic!("set_levels: {e}"));
    settle(&w, &owner, &all).await;
    // No meta of it was sealed into everyone.
    let in_everyone = w
        .relay
        .of_kind(KIND_CONTENT)
        .iter()
        .filter(|e| e["tags"].as_array().is_some_and(|t| t.iter().any(|t| t[0] == "s" && t[1] == secret.as_str())))
        .count();
    assert!(in_everyone >= 1, "the meta went into the section's own feed");
    // The member reads it; the reader outside knows it exists (the journal
    // is open) and nothing more, and is not shown it.
    let seen = names(&a, &cid).await;
    assert_eq!((seen.scope(&secret), seen.look(&secret)), ("Backstage".to_string(), look.clone()));
    assert!(shown(&a, &cid, &secret).await);
    let outside = names(&b, &cid).await;
    assert!(!outside.knows(&secret) && outside.scope(&secret).is_empty(), "nothing of its meta: {outside:?}");
    assert!(b.svc.log(&cid).await.ok().flatten().is_some_and(|l| l.state().scopes.get(&secret).is_some_and(|d| d.hidden)));
    assert!(!shown(&b, &cid, &secret).await, "not shown to who is outside");
    assert!(shown(&admin, &cid, &secret).await, "staff belong to every section");
    // A section made in the open and hidden later, by an admin (2026-10-25): the new title goes inside only.
    let (open, _) = owner.svc.scope_add(&owner.keys, &cid, "Lounge", SchemeId::SdV1, History::Open).await.unwrap_or_else(|e| panic!("scope_add: {e}"));
    settle(&w, &owner, &all).await;
    assert!(shown(&b, &cid, &open).await && names(&b, &cid).await.scope(&open) == "Lounge", "an open section is shown to all");
    let hide = ScopeChange { title: Some("Green room".into()), hidden: Some(true), ..ScopeChange::default() };
    admin.svc.scope_set(&admin.keys, &cid, &open, &hide).await.unwrap_or_else(|e| panic!("scope_set: {e}"));
    settle(&w, &owner, &all).await;
    assert_eq!(names(&b, &cid).await.scope(&open), "Lounge", "the old title was seen, the new one is not");
    assert!(!shown(&b, &cid, &open).await);
    assert_eq!(names(&owner, &cid).await.scope(&open), "Green room", "the admin's meta is read inside");
    // Shown again: the next meta goes into everyone.
    admin.svc.scope_set(&admin.keys, &cid, &open, &ScopeChange { title: Some("Hall".into()), hidden: Some(false), ..ScopeChange::default() }).await.unwrap_or_else(|e| panic!("scope_set: {e}"));
    settle(&w, &owner, &all).await;
    assert!(shown(&b, &cid, &open).await && names(&b, &cid).await.scope(&open) == "Hall");
    // An admin deletes it; a reader may not.
    assert!(b.svc.scope_del(&b.keys, &cid, &open).await.is_err(), "a reader");
    admin.svc.scope_del(&admin.keys, &cid, &open).await.unwrap_or_else(|e| panic!("scope_del: {e}"));
    settle(&w, &owner, &all).await;
    assert!(!shown(&b, &cid, &open).await);
    assert!(b.svc.log(&cid).await.ok().flatten().is_some_and(|l| !l.state().scopes.contains_key(&open)), "gone from the journal");
    for n in all {
        assert!(n.errors().is_empty(), "{:?}", n.errors());
    }
}

/// The photo of a section: sealed on the hosts under a key of its meta;
/// a member sees it, the hosts hold no picture, a replaced photo goes.
#[tokio::test]
async fn the_photo_of_a_section_is_sealed_and_shown_to_who_reads_its_meta() {
    let w = World::new();
    let (owner, admin, a, b, cid) = channel(&w).await;
    let all = [&owner, &admin, &a, &b];
    let (gold, _) = owner.svc.scope_add(&owner.keys, &cid, "Premium", SchemeId::SdV1, History::Open).await.unwrap_or_else(|e| panic!("scope_add: {e}"));
    settle(&w, &owner, &all).await;
    let png = messenger_avatar::png_of_rgba(40, 30, &[200u8; 40 * 30 * 4]).unwrap_or_else(|e| panic!("png: {e:?}"));
    assert!(err_text(a.svc.upload_scope_photo(&a.keys, &cid, &png).await).contains("not permitted"), "a reader");
    admin.svc.upload_scope_photo(&admin.keys, &cid, &png).await.unwrap_or_else(|e| panic!("an admin's photo: {e}"));
    let up = owner.svc.upload_scope_photo(&owner.keys, &cid, &png).await.unwrap_or_else(|e| panic!("photo: {e}"));
    let stored = w.hosts.blobs.lock().unwrap_or_else(|e| e.into_inner()).get(H1).and_then(|b| b.get(&up.photo.sha256).cloned()).unwrap_or_else(|| panic!("on the host"));
    assert!(!stored.starts_with(&[0xff, 0xd8, 0xff]), "the hosts hold no JPEG");
    owner.svc.scope_set(&owner.keys, &cid, &gold, &ScopeChange { photo: Some(Some(up.photo.clone())), ..ScopeChange::default() }).await.unwrap_or_else(|e| panic!("scope_set: {e}"));
    settle(&w, &owner, &all).await;
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let url = b.svc.scope_photo_data_url(&cid, &gold, dir.path()).await.unwrap_or_else(|e| panic!("{e}")).unwrap_or_else(|| panic!("the photo"));
    assert!(url.starts_with("data:image/jpeg;base64,"));
    // The middle of the picture in the 3:2 of a tile (960×640), not a square as a cover.
    use base64::Engine as _;
    let jpeg = base64::engine::general_purpose::STANDARD.decode(&url["data:image/jpeg;base64,".len()..]).unwrap_or_else(|e| panic!("{e}"));
    let img = messenger_avatar::decode(&jpeg).unwrap_or_else(|e| panic!("decode: {e:?}"));
    assert_eq!((img.width(), img.height()), crate::media::SCOPE_PHOTO_SIZE);
    assert_eq!(b.svc.scope_photo_data_url(&cid, SCOPE_EVERYONE, dir.path()).await.unwrap_or_else(|e| panic!("{e}")), None, "no photo");
    // Replaced: the old blob is removed from the hosts.
    let next = owner.svc.upload_scope_photo(&owner.keys, &cid, &png).await.unwrap_or_else(|e| panic!("photo: {e}"));
    assert_ne!(next.photo.sha256, up.photo.sha256, "a key of its own each time");
    owner.svc.scope_set(&owner.keys, &cid, &gold, &ScopeChange { photo: Some(Some(next.photo.clone())), ..ScopeChange::default() }).await.unwrap_or_else(|e| panic!("scope_set: {e}"));
    let blobs = w.hosts.blobs.lock().unwrap_or_else(|e| e.into_inner()).get(H1).cloned().unwrap_or_default();
    assert!(!blobs.contains_key(&up.photo.sha256) && blobs.contains_key(&next.photo.sha256));
}

fn item(page: &crate::transport::FeedPage, id: &str) -> FeedItem {
    page.items.iter().find(|i| i.id == id).cloned().unwrap_or_else(|| panic!("the post {id}"))
}

/// The edit of a file's post is a new caption (wire §4, 2026-10-23): the
/// readers see the new words in its format and the date of the edit, the
/// attachment stays the same file; an empty caption takes it away.
#[tokio::test]
async fn an_edit_of_a_file_changes_its_caption_and_says_when() {
    let w = World::new();
    let (owner, admin, a, b, cid) = channel(&w).await;
    let all = [&owner, &admin, &a, &b];
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let path = dir.path().join("photo.jpg");
    std::fs::write(&path, vec![7u8; 20_000]).unwrap_or_else(|e| panic!("{e}"));
    // An album caption in Markdown (2026-10-23): `fmt` on the envelope of the file.
    let file = crate::media::NewFile { caption: Some("**first** words"), fmt: TextFormat::Markdown, ..crate::media::NewFile::at(&path) };
    let (row, _) = owner.svc.media_placeholder_file(&owner.keys, &cid, file).await.unwrap_or_else(|e| panic!("placeholder: {e}"));
    let (id, _) = owner.svc.media_send(&owner.keys, &row.id, None, &crate::media::Sink(|_| {})).await.unwrap_or_else(|e| panic!("send: {e}"));
    let (text_post, _) = owner.svc.send_post(&owner.keys, &cid, "words", TextFormat::Plain).await.unwrap_or_else(|e| panic!("post: {e}"));
    w.pump(&all).await;
    let page = a.svc.history(&a.keys, &cid, 10, None).await.unwrap_or_else(|e| panic!("{e}"));
    let before = item(&page, &id);
    assert_eq!((before.text.as_deref(), before.fmt.as_deref(), before.edited_at), (Some("**first** words"), Some("md"), None));
    assert!(before.rich.as_ref().is_some_and(|b| !b.is_empty()), "the caption shown as Markdown");
    assert_eq!(item(&page, &text_post).edited_at, None);
    let descriptor = |i: &FeedItem| messenger_media::MediaDescriptor::from_envelope(&Envelope::parse(i.media_json.as_deref().unwrap_or("")).unwrap_or_else(|e| panic!("{e}"))).unwrap_or_else(|e| panic!("{e}"));
    w.clock.advance(30);
    let edited_at = w.clock.now();
    owner.svc.send_edit(&owner.keys, &cid, &id, "**new** words", TextFormat::Markdown).await.unwrap_or_else(|e| panic!("edit: {e}"));
    w.pump(&all).await;
    for n in [&owner, &a, &b] {
        let page = n.svc.history(&n.keys, &cid, 10, None).await.unwrap_or_else(|e| panic!("{e}"));
        let after = item(&page, &id);
        assert_eq!(after.text.as_deref(), Some("**new** words"));
        assert_eq!((after.fmt.as_deref(), after.edited_at), (Some("md"), Some(edited_at)));
        let (d, was) = (descriptor(&after), descriptor(&before));
        assert_eq!(d.caption.as_deref(), Some("**new** words"), "the caption of the envelope too");
        assert_eq!((d.sha256, d.servers), (was.sha256, was.servers), "the same file");
    }
    // An edit in plain text turns the Markdown of the caption off.
    w.clock.advance(5);
    owner.svc.send_edit(&owner.keys, &cid, &id, "plain *now*", TextFormat::Plain).await.unwrap_or_else(|e| panic!("edit: {e}"));
    w.pump(&all).await;
    let page = b.svc.history(&b.keys, &cid, 10, None).await.unwrap_or_else(|e| panic!("{e}"));
    let after = item(&page, &id);
    assert_eq!((after.text.as_deref(), after.fmt.as_deref(), after.rich.is_none()), (Some("plain *now*"), None, true));
    let env = Envelope::parse(after.media_json.as_deref().unwrap_or("")).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!((env.format(), env.str_field("caption")), (TextFormat::Plain, Some("plain *now*")));
    // An empty caption takes it away.
    w.clock.advance(5);
    owner.svc.send_edit(&owner.keys, &cid, &id, "", TextFormat::Plain).await.unwrap_or_else(|e| panic!("edit: {e}"));
    w.pump(&all).await;
    let page = b.svc.history(&b.keys, &cid, 10, None).await.unwrap_or_else(|e| panic!("{e}"));
    let after = item(&page, &id);
    assert_eq!((after.text.as_deref(), after.fmt.as_deref(), descriptor(&after).caption), (None, None, None));
    // Somebody else's edit changes nothing.
    assert!(err_text(admin.svc.send_edit(&admin.keys, &cid, &id, "mine", TextFormat::Plain).await).contains("only the author edits"));
    for n in all {
        assert!(n.errors().is_empty(), "{:?}", n.errors());
    }
}

/// The channels of a reader in the order of the list of chats: by the
/// newest post `last_posts` gives, newest first; with no post, by when
/// the channel came here (as `ChannelView::last_at`).
async fn list_order(n: &Node) -> Vec<String> {
    let rows = crate::db::channels::list(n.svc.db()).await.unwrap_or_else(|e| panic!("list: {e}"));
    let lasts = n.svc.last_posts(Some(&n.me), &rows).await.unwrap_or_else(|e| panic!("last posts: {e}"));
    let mut at: Vec<(i64, String)> = rows.iter().map(|r| (lasts.get(&r.id).map_or(r.created_at, |m| m.created_at), r.id.clone())).collect();
    at.sort_by(|x, y| y.0.cmp(&x.0).then_with(|| x.1.cmp(&y.1)));
    at.into_iter().map(|(_, id)| id).collect()
}

async fn last_id(n: &Node, cid: &str) -> Option<String> {
    let rows = crate::db::channels::list(n.svc.db()).await.unwrap_or_else(|e| panic!("list: {e}"));
    n.svc.last_posts(Some(&n.me), &rows).await.unwrap_or_else(|e| panic!("last posts: {e}")).remove(cid).map(|m| m.id)
}

/// The list of chats orders the channels by their last post (owner
/// 2026-10-23: «не по тому, когда тыкнул на него, а по последнему посту»):
/// opening a channel, reading it, a comment, a reaction, an edit and news
/// of the journal leave the order; a new post moves its channel up; a
/// post of a section the reader does not read (never had, or lost) moves
/// nothing and is not the last post of the row.
#[tokio::test]
async fn the_channels_are_ordered_by_their_last_post_the_reader_may_read() {
    let w = World::new();
    let (owner, a, b) = (w.node().await, w.node().await, w.node().await);
    let all = [&owner, &a, &b];
    let (c1, _) = owner.svc.create(&owner.keys, NewChannel { name: "One".into(), ..new_channel() }).await.unwrap_or_else(|e| panic!("create: {e}"));
    w.clock.advance(5);
    let (c2, _) = owner.svc.create(&owner.keys, NewChannel { name: "Two".into(), ..new_channel() }).await.unwrap_or_else(|e| panic!("create: {e}"));
    w.pump(&[&owner]).await;
    for cid in [&c1, &c2] {
        for n in [&a, &b] {
            join_by_link(&w, &owner, n, cid, &all).await;
        }
    }
    w.clock.advance(600);
    settle(&w, &owner, &all).await;
    // No post yet: by when each came here, the newer first.
    assert_eq!(list_order(&owner).await, [c2.clone(), c1.clone()], "no post: by creation");
    w.clock.advance(60);
    let (p1, _) = owner.svc.send_post(&owner.keys, &c1, "first", TextFormat::Plain).await.unwrap_or_else(|e| panic!("post: {e}"));
    w.clock.advance(60);
    let (p2, _) = owner.svc.send_post(&owner.keys, &c2, "second", TextFormat::Plain).await.unwrap_or_else(|e| panic!("post: {e}"));
    settle(&w, &owner, &all).await;
    let by_post = [c2.clone(), c1.clone()];
    for n in all {
        assert_eq!(list_order(n).await, by_post, "the newest post first");
    }
    assert_eq!(last_id(&a, &c1).await.as_deref(), Some(p1.as_str()));
    // Opened, read, commented, reacted to, edited, its journal changed:
    // the older channel stays below.
    w.clock.advance(60);
    a.svc.history(&a.keys, &c1, 10, None).await.unwrap_or_else(|e| panic!("history: {e}"));
    crate::db::content::set_read(a.svc.db(), &c1, w.clock.now()).await.unwrap_or_else(|e| panic!("read: {e}"));
    a.svc.send_comment(&a.keys, &c1, &p1, None, "a comment", TextFormat::Plain).await.unwrap_or_else(|e| panic!("comment: {e}"));
    b.svc.send_reaction(&b.keys, &c1, &p1, "👍", true).await.unwrap_or_else(|e| panic!("reaction: {e}"));
    owner.svc.send_edit(&owner.keys, &c1, &p1, "first, edited", TextFormat::Plain).await.unwrap_or_else(|e| panic!("edit: {e}"));
    owner.svc.act(&owner.keys, &c1, OpBody::SetRole { who: pk(&b), role: Role::Author }).await.unwrap_or_else(|e| panic!("role: {e}"));
    settle(&w, &owner, &all).await;
    for n in all {
        assert_eq!(list_order(n).await, by_post, "nothing but a post moves a channel");
    }
    // A new post moves its channel up.
    w.clock.advance(60);
    let (p3, _) = owner.svc.send_post(&owner.keys, &c1, "third", TextFormat::Plain).await.unwrap_or_else(|e| panic!("post: {e}"));
    settle(&w, &owner, &all).await;
    for n in all {
        assert_eq!(list_order(n).await, [c1.clone(), c2.clone()], "the new post moves One up");
    }
    // A section of Two that `a` reads and `b` does not.
    let (gold, _) = owner.svc.scope_add(&owner.keys, &c2, "Gold", SchemeId::SdV1, History::Open).await.unwrap_or_else(|e| panic!("scope_add: {e}"));
    let (lvl, _) = owner.svc.level_set(&owner.keys, &c2, None, "Patron", vec![gold.clone()], LevelCan::BASE).await.unwrap_or_else(|e| panic!("level: {e}"));
    w.pump(&all).await;
    owner.svc.set_levels(&owner.keys, &c2, &pk(&a), &[lvl]).await.unwrap_or_else(|e| panic!("set_levels: {e}"));
    settle(&w, &owner, &all).await;
    w.clock.advance(60);
    let (pg, _) = owner.svc.send_post_in(&owner.keys, &c2, Some(&gold), "for patrons", TextFormat::Plain).await.unwrap_or_else(|e| panic!("post: {e}"));
    settle(&w, &owner, &all).await;
    assert_eq!(list_order(&a).await, [c2.clone(), c1.clone()], "a reads Gold: its post moves Two up");
    assert_eq!(last_id(&a, &c2).await.as_deref(), Some(pg.as_str()));
    assert_eq!(list_order(&b).await, [c1.clone(), c2.clone()], "b does not read Gold: Two stays below");
    assert_eq!(last_id(&b, &c2).await.as_deref(), Some(p2.as_str()));
    assert_eq!(list_order(&owner).await, [c2.clone(), c1.clone()], "the owner reads every section");
    // `a` loses Gold: its post stays in the database, and is no longer the last.
    owner.svc.set_levels(&owner.keys, &c2, &pk(&a), &[]).await.unwrap_or_else(|e| panic!("set_levels: {e}"));
    settle(&w, &owner, &all).await;
    let raw = crate::db::content::last_posts(a.svc.db(), a.me.as_hex(), &[]).await.unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(raw.get(&c2).map(|m| m.id.as_str()), Some(pg.as_str()), "the post of Gold is still here");
    assert_eq!(last_id(&a, &c2).await.as_deref(), Some(p2.as_str()), "the newest post a reads");
    assert_eq!(list_order(&a).await, [c1.clone(), c2.clone()]);
    assert_eq!(last_id(&a, &c1).await.as_deref(), Some(p3.as_str()));
    for n in all {
        assert!(n.errors().is_empty(), "{:?}", n.errors());
    }
}

/// How long a page of a section takes (a switch of sections, 2026-10-23):
/// 240 posts over everyone's and two sections, comments and reactions on
/// some; printed, not asserted. `cargo test -p communities-core -- --ignored --nocapture a_page_of_a_section_is_timed`.
#[tokio::test]
#[ignore = "a measurement"]
async fn a_page_of_a_section_is_timed() {
    let w = World::new();
    let (owner, admin, a, b, cid) = channel(&w).await;
    let all = [&owner, &admin, &a, &b];
    let (gold, _) = owner.svc.scope_add(&owner.keys, &cid, "Premium", SchemeId::SdV1, History::Open).await.unwrap_or_else(|e| panic!("scope_add: {e}"));
    let (news, _) = owner.svc.scope_add(&owner.keys, &cid, "News", SchemeId::SdV1, History::Open).await.unwrap_or_else(|e| panic!("scope_add: {e}"));
    settle(&w, &owner, &all).await;
    let into = [None, Some(gold.as_str()), Some(news.as_str())];
    for i in 0..240usize {
        let (id, _) = owner.svc.send_post_in(&owner.keys, &cid, into[i % 3], &format!("post {i}\nwords"), TextFormat::Markdown).await.unwrap_or_else(|e| panic!("post: {e}"));
        if i % 4 == 0 {
            owner.svc.send_comment(&owner.keys, &cid, &id, None, "a comment", TextFormat::Plain).await.unwrap_or_else(|e| panic!("comment: {e}"));
        }
        if i % 3 == 0 {
            owner.svc.send_reaction(&owner.keys, &cid, &id, "👍", true).await.unwrap_or_else(|e| panic!("react: {e}"));
        }
    }
    w.pump(&all).await;
    for (name, sc) in [("all", None), ("everyone", Some(SCOPE_EVERYONE)), ("gold", Some(gold.as_str())), ("news", Some(news.as_str()))] {
        let mut took = Vec::new();
        let mut n = 0;
        for _ in 0..7 {
            let t = std::time::Instant::now();
            let page = owner.svc.history_page_in(&owner.keys, &cid, sc, 30, None).await.unwrap_or_else(|e| panic!("{e}"));
            took.push(t.elapsed().as_micros());
            n = page.items.len();
        }
        took.sort_unstable();
        println!("history_page_in {name}: {n} items, median {} µs, max {} µs", took[3], took[6]);
    }
}
