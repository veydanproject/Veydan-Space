// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! What of a channel lives in the secret store of the host, never in
//! `communities.db` (internal/communities-stage1.md §4): the keeper's key
//! pair and the owner's root secret, a master handed to a keeper without
//! the root, and the secrets of the link, which the keeper and every
//! member hold.

use crate::link::LinkSecret;
use keydist::{GenerationMaster, RootSecret};
use messenger_core::{MessengerError, Result, SecretStore};
use nostr::key::{Keys, SecretKey};
use std::collections::BTreeMap;

/// How many link epochs are kept: the current one and the one before,
/// which a keeper still takes for a while after a `rotate_link` whose
/// secret has not come (wire §10).
const LINK_EPOCHS_KEPT: usize = 2;

pub fn keeper_name(community_id: &str) -> String {
    format!("communities.keeper.{community_id}")
}

pub fn root_name(community_id: &str) -> String {
    format!("communities.root.{community_id}")
}

/// The serial of the keeper series the root secret held was made for (a
/// new owner, an emergency with a new root): of two roots that reach a
/// device by notes of its own, the one of the newer series stays.
pub fn root_serial_name(community_id: &str) -> String {
    format!("communities.root_serial.{community_id}")
}

pub fn master_name(community_id: &str, scope_id: &str, gen: u32) -> String {
    format!("communities.master.{community_id}.{scope_id}.{gen}")
}

pub fn link_name(community_id: &str) -> String {
    format!("communities.link.{community_id}")
}

/// The secret of the key of the channel given to me for the owner's Cloud
/// (wire §9.2).
pub fn cloud_name(community_id: &str) -> String {
    format!("communities.cloud.{community_id}")
}

pub async fn put_cloud_key(secrets: &dyn SecretStore, community_id: &str, keys: &Keys) -> Result<()> {
    secrets.put(&cloud_name(community_id), keys.secret_key().as_secret_bytes()).await
}

pub async fn cloud_key(secrets: &dyn SecretStore, community_id: &str) -> Result<Option<Keys>> {
    let Some(raw) = secrets.get(&cloud_name(community_id)).await? else { return Ok(None) };
    let sk = SecretKey::from_slice(&raw).map_err(|e| MessengerError::Crypto(e.to_string()))?;
    Ok(Some(Keys::new(sk)))
}

pub async fn forget_cloud_key(secrets: &dyn SecretStore, community_id: &str) -> Result<()> {
    secrets.delete(&cloud_name(community_id)).await
}

fn bytes32(v: &[u8], what: &str) -> Result<[u8; 32]> {
    v.try_into().map_err(|_| MessengerError::Crypto(format!("{what} is not 32 bytes")))
}

pub async fn put_keeper_keys(secrets: &dyn SecretStore, community_id: &str, keys: &Keys) -> Result<()> {
    secrets.put(&keeper_name(community_id), keys.secret_key().as_secret_bytes()).await
}

pub async fn keeper_keys(secrets: &dyn SecretStore, community_id: &str) -> Result<Option<Keys>> {
    let Some(raw) = secrets.get(&keeper_name(community_id)).await? else { return Ok(None) };
    let sk = SecretKey::from_slice(&raw).map_err(|e| MessengerError::Crypto(e.to_string()))?;
    Ok(Some(Keys::new(sk)))
}

pub async fn put_root(secrets: &dyn SecretStore, community_id: &str, root: &RootSecret) -> Result<()> {
    secrets.put(&root_name(community_id), root.as_bytes()).await
}

/// Keeps a root secret with the serial of the series it was made for.
pub async fn put_root_of(secrets: &dyn SecretStore, community_id: &str, root: &RootSecret, serial: u32) -> Result<()> {
    put_root(secrets, community_id, root).await?;
    secrets.put(&root_serial_name(community_id), serial.to_string().as_bytes()).await
}

/// The serial of the series the root held was made for (0: the channel's
/// first, or a root kept before the serial was).
pub async fn root_serial(secrets: &dyn SecretStore, community_id: &str) -> Result<u32> {
    let Some(raw) = secrets.get(&root_serial_name(community_id)).await? else { return Ok(0) };
    Ok(std::str::from_utf8(&raw).ok().and_then(|s| s.parse().ok()).unwrap_or(0))
}

/// Forgets the root secret (a former owner: the channel's root is the new
/// owner's own).
pub async fn forget_root(secrets: &dyn SecretStore, community_id: &str) -> Result<()> {
    secrets.delete(&root_name(community_id)).await?;
    secrets.delete(&root_serial_name(community_id)).await
}

/// Forgets the keeper's key pair (the keeper went to another device).
pub async fn forget_keeper_keys(secrets: &dyn SecretStore, community_id: &str) -> Result<()> {
    secrets.delete(&keeper_name(community_id)).await
}

pub async fn root(secrets: &dyn SecretStore, community_id: &str) -> Result<Option<RootSecret>> {
    let Some(raw) = secrets.get(&root_name(community_id)).await? else { return Ok(None) };
    Ok(Some(RootSecret::from_bytes(bytes32(&raw, "a root secret")?)))
}

pub async fn put_master(secrets: &dyn SecretStore, community_id: &str, scope_id: &str, gen: u32, master: &GenerationMaster) -> Result<()> {
    secrets.put(&master_name(community_id, scope_id, gen), master.as_bytes()).await
}

pub async fn master(secrets: &dyn SecretStore, community_id: &str, scope_id: &str, gen: u32) -> Result<Option<GenerationMaster>> {
    let Some(raw) = secrets.get(&master_name(community_id, scope_id, gen)).await? else { return Ok(None) };
    Ok(Some(GenerationMaster::from_bytes(bytes32(&raw, "a generation master")?)))
}

/// The link secrets held, by link epoch.
pub async fn links(secrets: &dyn SecretStore, community_id: &str) -> Result<BTreeMap<u32, LinkSecret>> {
    let Some(raw) = secrets.get(&link_name(community_id)).await? else { return Ok(BTreeMap::new()) };
    let map: BTreeMap<u32, String> = serde_json::from_slice(&raw).map_err(|e| MessengerError::Storage(e.to_string()))?;
    map.into_iter().map(|(e, s)| Ok((e, LinkSecret::decode(&s)?))).collect()
}

/// Keeps the secret of a link epoch; `false` when another secret is held
/// for it already (the first one stays: wire §10 names no author of a
/// secret in the feed, so a later one is not taken over it).
pub async fn put_link(secrets: &dyn SecretStore, community_id: &str, link_epoch: u32, secret: &LinkSecret) -> Result<bool> {
    let mut held = links(secrets, community_id).await?;
    match held.get(&link_epoch) {
        Some(s) if s == secret => return Ok(true),
        Some(_) => return Ok(false),
        None => {}
    }
    held.insert(link_epoch, secret.clone());
    while held.len() > LINK_EPOCHS_KEPT {
        held.pop_first();
    }
    let map: BTreeMap<u32, String> = held.iter().map(|(e, s)| (*e, s.encode())).collect();
    secrets.put(&link_name(community_id), &serde_json::to_vec(&map)?).await?;
    Ok(true)
}

/// Keeps the secret of a link epoch from a source that is checked: the
/// keeper's issue, the author of the link epoch in the feed, a note of my
/// own. It takes the place of one taken from a link pasted in (`put_link`
/// keeps the first): a forged URL must not shadow the real secret.
pub async fn put_link_trusted(secrets: &dyn SecretStore, community_id: &str, link_epoch: u32, secret: &LinkSecret) -> Result<()> {
    let mut held = links(secrets, community_id).await?;
    if held.get(&link_epoch) == Some(secret) {
        return Ok(());
    }
    held.insert(link_epoch, secret.clone());
    while held.len() > LINK_EPOCHS_KEPT {
        held.pop_first();
    }
    let map: BTreeMap<u32, String> = held.iter().map(|(e, s)| (*e, s.encode())).collect();
    secrets.put(&link_name(community_id), &serde_json::to_vec(&map)?).await
}

/// Forgets every secret of a channel.
pub async fn forget(secrets: &dyn SecretStore, community_id: &str) -> Result<()> {
    for name in [keeper_name(community_id), root_name(community_id), root_serial_name(community_id), link_name(community_id), cloud_name(community_id)] {
        secrets.delete(&name).await?;
    }
    Ok(())
}
