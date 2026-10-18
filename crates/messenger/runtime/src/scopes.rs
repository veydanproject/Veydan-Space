// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Which server a request goes to (internal/messenger-wire.md, «Серверы»).
//!
//! The pool knows servers, not groups: a group's scope is resolved here,
//! at the boundary between the outbox (which keeps the group in the stored
//! request, for its own checks) and the pool. A group lives on the server
//! of its relay: a private server's group goes to `Scope::Server`, one of
//! the home set, or of a relay that is no server of mine, to `Scope::Own`,
//! where every group went before servers.

use async_trait::async_trait;
use messenger_core::traits::RelayStatusSnapshot;
use messenger_core::{Ack, Outbound, RawEvent, Result, Scope, Transport};
use messenger_store::{servers, Store};
use messenger_transport::RelayPool;
use std::sync::Arc;

/// The private server group `group_id` lives on; `None` for the home set
/// (and for a group this device does not know): `servers::of_group`.
pub(crate) async fn server_of_group(store: &Store, group_id: &str) -> Result<Option<String>> {
    servers::of_group(store, group_id).await
}

/// The scope of a server: `Own` for the home set.
pub(crate) fn of_server(server: Option<&str>) -> Scope {
    match server {
        Some(id) => Scope::Server { id: id.to_string() },
        None => Scope::Own,
    }
}

/// The ids of the private servers, each its own scope.
pub(crate) async fn private_servers(store: &Store) -> Vec<String> {
    match servers::private(store).await {
        Ok(rows) => rows.into_iter().map(|s| s.id).collect(),
        Err(e) => {
            eprintln!("messenger servers: the private servers were not read: {e}");
            Vec::new()
        }
    }
}

/// A subscription id of `server`: the id of the home set as it was, the
/// server's with its id after `@`, so each scope keeps its own REQ.
pub(crate) fn sub_id(base: &str, server: Option<&str>) -> String {
    match server {
        Some(id) => format!("{base}@{id}"),
        None => base.to_string(),
    }
}

/// `out` with a group's scope turned into its server's.
pub(crate) async fn resolve(store: &Store, out: Outbound) -> Result<Outbound> {
    async fn scope(store: &Store, scope: Scope) -> Result<Scope> {
        match scope {
            Scope::Group { id } => Ok(of_server(server_of_group(store, &id).await?.as_deref())),
            other => Ok(other),
        }
    }
    Ok(match out {
        Outbound::PublishScoped { scope: s, event } => Outbound::PublishScoped {
            scope: scope(store, s).await?,
            event,
        },
        Outbound::Subscribe {
            id,
            filter,
            scope: s,
        } => Outbound::Subscribe {
            id,
            filter,
            scope: scope(store, s).await?,
        },
        Outbound::Sync {
            scope: s,
            filter,
            local,
        } => Outbound::Sync {
            scope: scope(store, s).await?,
            filter,
            local,
        },
        other => other,
    })
}

/// The pool as the outbox pumps into it: every request with its group's
/// scope resolved first.
pub(crate) struct Routed {
    pub(crate) store: Store,
    pub(crate) pool: Arc<RelayPool>,
}

#[async_trait]
impl Transport for Routed {
    async fn send(&self, out: Outbound) -> Result<Ack> {
        self.pool.send(resolve(&self.store, out).await?).await
    }

    fn events(&self) -> tokio::sync::mpsc::Receiver<RawEvent> {
        self.pool.events()
    }

    async fn status(&self) -> RelayStatusSnapshot {
        self.pool.status().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use messenger_core::outbound::{Filter, WireEvent};
    use messenger_core::EventId;
    use messenger_store::groups::{self, GroupRow};

    async fn group(store: &Store, id: &str, relay_url: &str) {
        let row = GroupRow {
            id: id.into(),
            kind: "private".into(),
            name: "Team".into(),
            about: String::new(),
            picture: String::new(),
            relay_url: relay_url.into(),
            owner: "o".into(),
            membership: "joined".into(),
            my_role: Some("owner".into()),
            members: 1,
            link_epoch: 0,
            created_at: 0,
            updated_at: 0,
            needs_update: false,
        };
        groups::upsert(store, &row).await.unwrap();
    }

    fn server(url: &str, kind: &str) -> servers::ServerRow {
        servers::ServerRow {
            id: String::new(),
            kind: kind.into(),
            name: None,
            relay_url: url.into(),
            relay_key: None,
            media_url: None,
            call_node: None,
            push_mode: servers::PUSH_OFF.into(),
            source: None,
            added_at: 0,
        }
    }

    fn published(scope: Scope) -> Outbound {
        Outbound::PublishScoped {
            scope,
            event: WireEvent {
                id: EventId::parse(&"e".repeat(64)).unwrap(),
                json: serde_json::json!({}),
            },
        }
    }

    fn scope_of(out: &Outbound) -> &Scope {
        match out {
            Outbound::PublishScoped { scope, .. }
            | Outbound::Subscribe { scope, .. }
            | Outbound::Sync { scope, .. } => scope,
            other => panic!("no scope: {other:?}"),
        }
    }

    /// A group of a private server goes to that server; one of the home
    /// set, of an unknown relay or unknown altogether to the home set; a
    /// scope that is no group stays as it is.
    #[tokio::test]
    async fn a_group_goes_to_the_server_of_its_relay() {
        let store = Store::open_in_memory().await.unwrap();
        let private = servers::put(&store, &server("wss://mine.example", servers::KIND_PRIVATE))
            .await
            .unwrap();
        servers::put(&store, &server("wss://home.example", servers::KIND_HOME))
            .await
            .unwrap();
        let (on_private, on_home, elsewhere) = ("a".repeat(64), "b".repeat(64), "c".repeat(64));
        group(&store, &on_private, "wss://Mine.example/").await;
        group(&store, &on_home, "wss://home.example").await;
        group(&store, &elsewhere, "wss://foreign.example").await;

        let g = |id: &str| Scope::Group { id: id.to_string() };
        let to = |s: Scope| async { resolve(&store, published(s)).await.unwrap() };
        assert_eq!(
            scope_of(&to(g(&on_private)).await),
            &Scope::Server {
                id: private.clone()
            }
        );
        assert_eq!(scope_of(&to(g(&on_home)).await), &Scope::Own);
        assert_eq!(scope_of(&to(g(&elsewhere)).await), &Scope::Own);
        assert_eq!(
            scope_of(&to(g(&"d".repeat(64))).await),
            &Scope::Own,
            "a group this device does not know"
        );
        assert_eq!(
            scope_of(&to(Scope::Server { id: "x".into() }).await),
            &Scope::Server { id: "x".into() }
        );

        let sub = Outbound::Subscribe {
            id: messenger_core::SubId("s".into()),
            filter: Filter(serde_json::json!({})),
            scope: g(&on_private),
        };
        assert_eq!(
            scope_of(&resolve(&store, sub).await.unwrap()),
            &Scope::Server {
                id: private.clone()
            }
        );
        let sync = Outbound::Sync {
            scope: g(&on_private),
            filter: Filter(serde_json::json!({})),
            local: vec![],
        };
        assert_eq!(
            scope_of(&resolve(&store, sync).await.unwrap()),
            &Scope::Server {
                id: private.clone()
            }
        );

        assert_eq!(private_servers(&store).await, vec![private.clone()]);
        assert_eq!(sub_id("dm-live", None), "dm-live");
        assert_eq!(
            sub_id("dm-live", Some(&private)),
            format!("dm-live@{private}")
        );
    }
}
