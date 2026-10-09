// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The sets of servers with a registry of volunteers (the fake of the
//! testkit): the trust level, the signed list, what is cached and when
//! it is asked for again.

use messenger_calls::engine::RelayPolicy;
use messenger_calls::registry::{ListFetch, Registry, KEY_REGISTRY_CHECKED};
use messenger_calls::{CallNode, NodeClass, NodeRef, NodeSource, ServerSets, SettingsServerSets, TrustLevel, KEY_CALL_NODES};
use messenger_store::{settings, Store};
use messenger_testkit::{FakeEngine, FakeNode, FakeRegistry};
use std::sync::atomic::Ordering;
use std::sync::Arc;

fn node(seed: u8, addr: &str) -> NodeRef {
    format!("{addr}#{}", format!("{seed:02x}").repeat(32)).parse().unwrap()
}

/// The sets with the fakes of the nodes and of the registry (the owner's
/// decision of 2026-10-09, «берём ближайший быстрый»): under `any` the
/// project's and the volunteers' nodes are one tier, and the nearest of
/// them carries the call and hosts the room; under `project_and_own` a
/// volunteer is not in the sets, so not even spoken to; a volunteer the
/// registry lists as degraded is not in them while an active one is.
#[tokio::test]
async fn under_any_the_nearest_of_the_project_and_the_volunteers_carries_the_call() {
    let store = Store::open_in_memory().await.unwrap();
    let sets = Arc::new(SettingsServerSets::new(store.clone()));
    let fake = FakeNode::new(FakeEngine::new());
    let (project, near, degraded) = (0, fake.add_node(), fake.add_node());
    fake.set_rtt(project, 60);
    fake.set_rtt(near, 10);
    fake.set_rtt(degraded, 5);
    sets.set_manifest(vec![fake.reference()]);
    let registry = FakeRegistry::new();
    registry.list(vec![(fake.node(near, NodeClass::Volunteer).node, "eu", 20), (fake.node(degraded, NodeClass::Volunteer).node, "eu", 5)]);
    registry.degrade(&fake.node(degraded, NodeClass::Volunteer).node);
    sets.set_registry(Arc::new(registry.registry(store.clone())));
    sets.call_nodes().await.unwrap();
    sets.registry().unwrap().settle().await;

    let nodes = sets.call_nodes().await.unwrap();
    assert_eq!(nodes.iter().map(|n| n.class).collect::<Vec<_>>(), vec![NodeClass::Project, NodeClass::Volunteer], "the degraded volunteer is left out");
    let client = fake.client();
    let picked = client.pick(&nodes, RelayPolicy::Auto, 0).await.unwrap();
    let id = |i: usize| fake.node(i, NodeClass::Project).node.id.to_string();
    assert_eq!(picked.nodes, vec![id(near), id(project)], "the volunteer at 10 ms before the project's at 60 ms; the project's is the spare");
    assert_eq!(client.pick_sfu(&nodes, 0).await.unwrap().node.node.id.to_string(), id(near), "the room goes to the nearest too");
    let known: Vec<String> = client.known().into_iter().map(|k| k.id).collect();
    assert!(known.contains(&id(near)) && known.contains(&id(project)) && !known.contains(&id(degraded)), "{known:?}");

    // The project's and own: the volunteer is not in the sets, and a
    // fresh client speaks to nobody but the project's.
    sets.set_trust(TrustLevel::ProjectAndOwn).await.unwrap();
    let nodes = sets.call_nodes().await.unwrap();
    assert_eq!(nodes.iter().map(|n| n.class).collect::<Vec<_>>(), vec![NodeClass::Project]);
    let client = fake.client();
    let picked = client.pick(&nodes, RelayPolicy::Auto, 0).await.unwrap();
    assert_eq!(picked.nodes, vec![id(project)]);
    assert_eq!(client.known().len(), 1, "no volunteer was asked");

    // Any, with my own node in the sets: it is first, however far, and
    // the tier below is not spoken to.
    sets.set_trust(TrustLevel::Any).await.unwrap();
    let own = fake.add_node();
    fake.set_rtt(own, 90);
    settings::set(&store, KEY_CALL_NODES, &format!(r#"["{}"]"#, fake.node(own, NodeClass::Own).node)).await.unwrap();
    let nodes = sets.call_nodes().await.unwrap();
    assert_eq!(nodes[0].class, NodeClass::Own);
    let client = fake.client();
    let picked = client.pick(&nodes, RelayPolicy::Auto, 0).await.unwrap();
    assert_eq!(picked.nodes, vec![id(own)]);
    assert_eq!(client.known().len(), 1, "my own node answered: nobody else saw the call");
}

#[tokio::test]
async fn the_trust_level_cuts_the_sets_and_the_registry_is_not_read_under_own_only() {
    let store = Store::open_in_memory().await.unwrap();
    let sets = Arc::new(SettingsServerSets::new(store.clone()));
    settings::set(&store, KEY_CALL_NODES, &format!(r#"["{}"]"#, node(1, "203.0.113.1:8443"))).await.unwrap();
    sets.set_manifest_classed(vec![
        (node(2, "203.0.113.2:8443"), NodeClass::Project),
        (node(3, "203.0.113.3:8443"), NodeClass::Volunteer),
        (node(9, "203.0.113.9:8443"), NodeClass::Own),
    ]);
    sets.set_cloud(vec![node(4, "203.0.113.4:8443")]);
    let registry = FakeRegistry::new();
    registry.list(vec![(node(5, "203.0.113.5:8443"), "eu", 30), (node(6, "203.0.113.6:8443"), "us", 10)]);
    registry.degrade(&node(6, "203.0.113.6:8443"));
    let asked = registry.asked();
    sets.set_registry(Arc::new(registry.registry(store.clone())));

    assert_eq!(sets.trust().await.unwrap(), TrustLevel::Any, "the default");
    // Never asked: the registry's part is empty now and asked for in
    // the background.
    let first = sets.call_nodes().await.unwrap();
    assert!(first.iter().all(|n| n.node.id.0[0] != 5), "nothing of the registry before its answer");
    sets.registry().unwrap().settle().await;
    assert_eq!(asked.load(Ordering::SeqCst), 1);
    let all = sets.call_nodes().await.unwrap();
    assert_eq!(
        all.iter().map(|n| (n.node.id.0[0], n.class)).collect::<Vec<_>>(),
        vec![(1, NodeClass::Own), (4, NodeClass::Cloud), (2, NodeClass::Project), (3, NodeClass::Volunteer), (5, NodeClass::Volunteer)],
        "own → cloud → manifest → registry; the manifest cannot name own nodes; the degraded node 6 is not used while 5 is active"
    );
    // No active node listed: the degraded one is used.
    registry.degrade(&node(5, "203.0.113.5:8443"));
    sets.registry().unwrap().refresh().await.unwrap();
    let ids: Vec<u8> = sets.call_nodes().await.unwrap().iter().map(|n| n.node.id.0[0]).collect();
    assert_eq!(ids, vec![1, 4, 2, 3, 5, 6]);
    registry.list(vec![(node(5, "203.0.113.5:8443"), "eu", 30), (node(6, "203.0.113.6:8443"), "us", 10)]);
    registry.degrade(&node(6, "203.0.113.6:8443"));
    sets.registry().unwrap().refresh().await.unwrap();
    sets.registry().unwrap().settle().await;
    assert_eq!(asked.load(Ordering::SeqCst), 3, "the registry was asked once in the background, then twice by hand; the answer is cached");

    sets.set_trust(TrustLevel::ProjectAndOwn).await.unwrap();
    let ids: Vec<u8> = sets.call_nodes().await.unwrap().iter().map(|n| n.node.id.0[0]).collect();
    assert_eq!(ids, vec![1, 4, 2], "no volunteer, from the manifest or the registry");

    sets.set_trust(TrustLevel::OwnOnly).await.unwrap();
    // The registry would be due again: it must not be asked.
    settings::set(&store, KEY_REGISTRY_CHECKED, "1").await.unwrap();
    let ids: Vec<u8> = sets.call_nodes().await.unwrap().iter().map(|n| n.node.id.0[0]).collect();
    assert_eq!(ids, vec![1]);
    sets.registry().unwrap().settle().await;
    assert_eq!(asked.load(Ordering::SeqCst), 3, "own only: the registry is not read");
    // Any again: due, so asked in the background, once.
    sets.set_trust(TrustLevel::Any).await.unwrap();
    sets.call_nodes().await.unwrap();
    sets.call_nodes().await.unwrap();
    sets.registry().unwrap().settle().await;
    assert_eq!(asked.load(Ordering::SeqCst), 4);

    // The screen sees everything, with where it came from and whether
    // the level lets it be used.
    let described = sets.describe().await.unwrap();
    assert_eq!(described.len(), 6);
    let d = |seed: u8| described.iter().find(|d| d.id.starts_with(&format!("{seed:02x}"))).unwrap();
    assert_eq!((d(1).source, d(1).trusted), (NodeSource::Setting, true));
    assert_eq!((d(2).source, d(2).trusted), (NodeSource::Manifest, true));
    assert_eq!((d(3).source, d(3).trusted), (NodeSource::Manifest, true));
    assert_eq!((d(5).source, d(5).region.as_deref(), d(5).load, d(5).state.as_deref()), (NodeSource::Registry, Some("eu"), Some(30), Some("active")));
    assert_eq!(d(6).state.as_deref(), Some("degraded"), "the screen sees the degraded node too");
    sets.set_trust(TrustLevel::ProjectAndOwn).await.unwrap();
    let described = sets.describe().await.unwrap();
    let d = |seed: u8| described.iter().find(|d| d.id.starts_with(&format!("{seed:02x}"))).unwrap();
    assert_eq!((d(2).trusted, d(3).trusted, d(5).trusted), (true, false, false));
    assert_eq!(TrustLevel::parse("own_only"), Some(TrustLevel::OwnOnly));
    assert_eq!(TrustLevel::OwnOnly.as_str(), "own_only");
    assert!(TrustLevel::OwnOnly.allows(NodeClass::Group), "the group's own choice passes every level");
    let _: Vec<CallNode> = all;
}

#[tokio::test]
async fn a_forged_or_expired_list_of_the_registry_is_no_set() {
    let store = Store::open_in_memory().await.unwrap();
    let registry = FakeRegistry::new();
    registry.list(vec![(node(5, "203.0.113.5:8443"), "eu", 0)]);
    // Signed by a key the root never delegated.
    let stranger = FakeRegistry::new();
    stranger.list(vec![(node(7, "203.0.113.7:8443"), "eu", 0)]);
    let fetch: ListFetch = stranger.fetch();
    let under_our_root = Arc::new(Registry::with_trust(store.clone(), fetch, vec!["https://registry.example/vlink".into()], registry.root_hex()));
    assert!(under_our_root.refresh().await.is_err(), "a list under another root is refused");
    assert!(under_our_root.nodes().await.unwrap().is_empty());

    // Expired by the time it is read: nothing.
    registry.expire_lists_at(1);
    let ours = Arc::new(registry.registry(store.clone()));
    assert!(ours.refresh().await.is_err());
    assert!(ours.nodes().await.unwrap().is_empty());
    assert!(ours.is_stale().await.unwrap());

    // A registry that is down: what was kept stays, the try is noted.
    registry.expire_lists_at(u64::MAX / 2);
    assert_eq!(ours.refresh().await.unwrap(), 1);
    registry.set_down(true);
    assert!(ours.refresh().await.is_err());
    let nodes = ours.nodes().await.unwrap();
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].node, node(5, "203.0.113.5:8443"));
    assert_eq!((nodes[0].state.as_str(), nodes[0].load, nodes[0].region.as_str()), ("active", 0, "eu"));
    assert!(!ours.is_stale().await.unwrap());
    assert!(ours.checked_at().await.unwrap().is_some());
}

/// The listing of the screen asks nothing of the registry unless the
/// level is `any` (wire.md §10: under `own_only` the registry is not
/// asked): what is cached is listed, untrusted, and the registry does
/// not learn of a device whose owner turned volunteers off.
#[tokio::test]
async fn the_screen_lists_the_registry_from_the_cache_unless_the_level_is_any() {
    let store = Store::open_in_memory().await.unwrap();
    let sets = Arc::new(SettingsServerSets::new(store.clone()));
    let registry = FakeRegistry::new();
    registry.list(vec![(node(5, "203.0.113.5:8443"), "eu", 30)]);
    let asked = registry.asked();
    sets.set_registry(Arc::new(registry.registry(store.clone())));
    // A list from before, due again.
    sets.registry().unwrap().refresh().await.unwrap();
    assert_eq!(asked.load(Ordering::SeqCst), 1);
    settings::set(&store, KEY_REGISTRY_CHECKED, "1").await.unwrap();
    assert!(sets.registry().unwrap().is_stale().await.unwrap());

    for level in [TrustLevel::OwnOnly, TrustLevel::ProjectAndOwn] {
        sets.set_trust(level).await.unwrap();
        let described = sets.describe().await.unwrap();
        let listed = described.iter().find(|d| d.source == NodeSource::Registry).unwrap_or_else(|| panic!("{level:?}: the cached node is listed"));
        assert!(!listed.trusted, "{level:?}: listed, not to be used");
        sets.registry().unwrap().settle().await;
        assert_eq!(asked.load(Ordering::SeqCst), 1, "{level:?}: the screen asked the registry nothing");
    }

    sets.set_trust(TrustLevel::Any).await.unwrap();
    assert!(sets.describe().await.unwrap().iter().any(|d| d.source == NodeSource::Registry && d.trusted));
    sets.registry().unwrap().settle().await;
    assert_eq!(asked.load(Ordering::SeqCst), 2, "any: due, so asked once, in the background");
}
