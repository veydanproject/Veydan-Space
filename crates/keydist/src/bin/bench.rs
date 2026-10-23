// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! `keydist-bench`: the benchmark of phase 1 (internal/communities-keydist.md
//! section 13), run on the PC and on the test phone.
//!
//! ```text
//! keydist-bench --out <json> [--quick] [--repeat 3] [--scheme cs|sd|flat]
//! ```
//!
//! Every measurement is repeated `--repeat` times; the JSON keeps the median
//! and the extremes. `--quick` cuts every count ten-fold (a smoke run).
//! Random inputs come from a seeded `XorShift64`, so two runs measure the
//! same work. The nonces of the epochs are not cryptographic here.
//!
//! The JSON is one object:
//!
//! ```text
//! { "env":     { target, arch, os, pointer_width, threads, cpu_model,
//!                cpu_cores, timestamp_utc },
//!   "scheme":  "cs" | "flat",
//!   "repeat":  3, "quick": false,
//!   "results": { <name>: ... , "memory": ... } }
//! ```
//!
//! Timed results are objects with `median_ns`, `min_ns`, `max_ns` (per unit
//! of work, the unit named by `per`) plus the context numbers. Keys of the
//! `cs` run:
//!
//! * `--scheme sd` measures the same keys with `Bundle::issue_sd` and the SD
//!   cover; `cover_nodes` then counts subsets.
//! * `bundle_issue.d20|d24`: `Bundle::issue` + `to_bytes`; `bytes` of a
//!   bundle, `n` bundles per sample.
//! * `bundle_issue_nip44.d20|d24`: the same plus the NIP-44 v2 envelope to a
//!   random recipient; `envelope_bytes`.
//! * `epoch_build.d<depth>_r<revoked>`: `assigned`, `revoked`, `cover_nodes`,
//!   `shards`, `upload_bytes`, `shard_p50|p95|max_bytes`; `median_ns` is
//!   cover + keys + AES-GCM blocks + shards + hashes; `cover_median_ns`,
//!   `cover_min_ns`, `cover_max_ns` are the cover alone; `build_median_ns`
//!   is everything but the cover.
//! * `open_epoch`: a member opens an epoch (verify + find + AES), per open;
//!   `mean_checks`, `members`, `shards`, `mean_shard_bytes`.
//! * `verify_shard.index|merkle`: `shard::verify` per shard, against the
//!   index blob (up to 512 shards a tree) or the shard's proof; `shards`,
//!   `mean_shard_bytes`, `index_bytes` | `proof_bytes`, `event_bytes`.
//! * `lazy_issue`: `bundle_for` after a planned migration, per bundle;
//!   `bundles_per_sec`; `join_issue`: `join` of new members, same fields.
//! * `state_delta`: a keeper of a million members (100 000 with `--quick`)
//!   at depth 24 with 20 000 revoked: the rows of `take_changes` (JSON
//!   bytes, and the time to take and serialize them) after one join, one
//!   ban, one epoch and its publication.
//! * `state_restore`: the whole state of that keeper as JSON (`bytes`),
//!   the time to write it, to parse it and to restore it (`from_state`
//!   with its invariant checks).
//! * `memory`: `vm_hwm_kb`, `vm_rss_kb` at the end, `largest_epoch_shards_bytes`,
//!   `largest_epoch_recipe_bytes`, `largest_epoch_revoked_set_bytes_estimate`.
//!
//! The `flat` run keeps `bundle_issue` out and measures `flat_join.n<members>` (per
//! member), `flat_epoch_build.n<members>` (all secrets of one epoch) and
//! `flat_open_secret`, then `memory`.

use std::hint::black_box;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use keydist::epoch::{self, EpochRecipe, TreeRecipe};
use keydist::generation::{GenState, GenerationKind};
use keydist::kdf::NodeKeys;
use keydist::keeper::{CsKeeper, GenLeaf, MemberRec, Policy, SdKeeper, TreeParams};
use keydist::sd::SdLabels;
use keydist::{
    member, shard, Bundle, EpochKey, EpochOut, FlatKeeper, GenerationMaster, History, Key32, KeeperState,
    KeyDistribution, MemberId, Revoke, SchemeId, ScopeRef, SecretBytes, Thresholds, Tree, XorShift64,
};
use serde_json::{json, Value};

const SCOPE: ScopeRef = ScopeRef { community: [1; 32], scope: [2; 32] };

struct Cfg {
    out: String,
    quick: bool,
    repeat: usize,
    flat: bool,
    sd: bool,
}

fn parse_args() -> Result<Cfg, String> {
    let mut cfg = Cfg { out: String::new(), quick: false, repeat: 3, flat: false, sd: false };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--out" => cfg.out = it.next().ok_or("--out needs a path")?,
            "--quick" => cfg.quick = true,
            "--repeat" => {
                cfg.repeat = it.next().and_then(|v| v.parse().ok()).filter(|n| *n > 0).ok_or("--repeat needs a number")?
            }
            "--scheme" => match it.next().as_deref() {
                Some("cs") => (cfg.flat, cfg.sd) = (false, false),
                Some("sd") => (cfg.flat, cfg.sd) = (false, true),
                Some("flat") => (cfg.flat, cfg.sd) = (true, false),
                _ => return Err("--scheme is cs, sd or flat".into()),
            },
            other => return Err(format!("unknown argument {other}")),
        }
    }
    if cfg.out.is_empty() {
        return Err("usage: keydist-bench --out <json> [--quick] [--repeat 3] [--scheme cs|sd|flat]".into());
    }
    Ok(cfg)
}

/// Median, min and max of samples.
fn stats(samples: &[f64]) -> (f64, f64, f64) {
    let mut s = samples.to_vec();
    s.sort_by(f64::total_cmp);
    let n = s.len();
    let med = if n % 2 == 1 { s[n / 2] } else { (s[n / 2 - 1] + s[n / 2]) / 2.0 };
    (med, s[0], s[n - 1])
}

/// Runs `f` `repeat` times; it returns the elapsed ns of the sample.
fn sample(repeat: usize, mut f: impl FnMut() -> f64) -> Vec<f64> {
    (0..repeat).map(|_| f()).collect()
}

/// The timing fields of a result, ns per unit (`samples` hold totals over `units`).
fn timing(samples: &[f64], units: f64, per: &str) -> serde_json::Map<String, Value> {
    let (med, min, max) = stats(samples);
    let mut m = serde_json::Map::new();
    m.insert("median_ns".into(), json!((med / units).round()));
    m.insert("min_ns".into(), json!((min / units).round()));
    m.insert("max_ns".into(), json!((max / units).round()));
    m.insert("per".into(), json!(per));
    m
}

fn with(mut m: serde_json::Map<String, Value>, extra: Value) -> Value {
    if let Value::Object(e) = extra {
        m.extend(e);
    }
    Value::Object(m)
}

fn ns(t: Instant) -> f64 {
    t.elapsed().as_nanos() as f64
}

fn scale(cfg: &Cfg, n: usize) -> usize {
    if cfg.quick {
        (n / 10).max(1)
    } else {
        n
    }
}

fn big_generation(depth: u8, assigned: u32, revoked: usize, seed: u64) -> GenState {
    let master = GenerationMaster::from_bytes([0x4d; 32]);
    let mut g = GenState::new(1, Tree::new(depth).unwrap(), 4096, master, GenerationKind::Initial, 0);
    g.next_leaf = assigned;
    let mut rng = XorShift64::new(seed);
    while g.revoked.len() < revoked {
        g.revoked.insert(rng.below(u64::from(assigned)) as u32);
    }
    g
}

/// The cover of a generation: nodes (`cs-v1`) or subsets (`sd-v1`).
enum Cover {
    Nodes(Vec<u64>),
    Subsets(Vec<(u64, u64)>),
}

impl Cover {
    fn of(g: &GenState, sd: bool) -> Self {
        if sd {
            Cover::Subsets(g.sd_cover())
        } else {
            Cover::Nodes(g.cover())
        }
    }

    fn len(&self) -> usize {
        match self {
            Cover::Nodes(v) => v.len(),
            Cover::Subsets(v) => v.len(),
        }
    }

    /// Bytes of the packed recipe entries (8 per node, 16 per subset; the
    /// nonces come from one seed).
    fn recipe_bytes(&self) -> usize {
        match self {
            Cover::Nodes(v) => v.len() * 8,
            Cover::Subsets(v) => v.len() * 16,
        }
    }
}

/// Bundle issuing for either scheme.
struct Issuer {
    nodes: NodeKeys,
    labels: Option<SdLabels>,
}

impl Issuer {
    fn new(sd: bool, g: &GenState) -> Self {
        Issuer { nodes: g.node_keys(&SCOPE), labels: sd.then(|| g.sd_labels(&SCOPE)) }
    }

    fn bundle(&self, tree: Tree, leaf: u32) -> Bundle {
        match &self.labels {
            Some(l) => Bundle::issue_sd(l, SCOPE, 1, tree, leaf),
            None => Bundle::issue(&self.nodes, SCOPE, 1, tree, leaf),
        }
    }
}

fn build_epoch(g: &GenState, lps: u32, rng: &mut XorShift64, cover: &Cover) -> EpochOut {
    let (scheme, tr) = match cover {
        Cover::Nodes(c) => (SchemeId::CsV1, TreeRecipe::new(1, g.tree, lps, u64::from(g.next_leaf), c)),
        Cover::Subsets(c) => (SchemeId::SdV1, TreeRecipe::new_sd(1, g.tree, lps, u64::from(g.next_leaf), c)),
    };
    let mut seed = [0u8; 32];
    keydist::Rng::fill(rng, &mut seed);
    let recipe = EpochRecipe {
        scheme,
        key: EpochKey { epoch: 1, key: Key32([9; 32]) },
        seed: Key32(seed),
        prev_epoch: None,
        at: 0,
        history: History::Open,
        chain: None,
        trees: vec![tr],
        recipients: Vec::new(),
        retires: Vec::new(),
    };
    let nodes = g.node_keys(&SCOPE);
    epoch::build(&SCOPE, &recipe, &|_| Some(nodes.clone()), &|_| SecretBytes::default()).unwrap()
}

fn percentile(sorted: &[usize], q: usize) -> usize {
    sorted[(sorted.len() * q).div_ceil(100) - 1]
}

fn bundle_issue(cfg: &Cfg) -> (Value, Value) {
    let n = scale(cfg, 1000);
    let (mut plain, mut nip) = (serde_json::Map::new(), serde_json::Map::new());
    for depth in [20u8, 24] {
        let tree = Tree::new(depth).unwrap();
        let g = GenState::new(1, tree, 4096, GenerationMaster::from_bytes([0x4d; 32]), GenerationKind::Initial, 0);
        let issuer = Issuer::new(cfg.sd, &g);
        let mut rng = XorShift64::new(u64::from(depth));
        let leaves: Vec<u32> = (0..n).map(|_| rng.below(1_000_000) as u32).collect();
        let mut bytes = 0;
        let s = sample(cfg.repeat, || {
            let t = Instant::now();
            for &l in &leaves {
                let b = issuer.bundle(tree, l).to_bytes();
                bytes = b.len();
                black_box(&b);
            }
            ns(t)
        });
        plain.insert(format!("d{depth}"), with(timing(&s, n as f64, "bundle"), json!({ "bytes": bytes, "n": n })));

        let sender = nostr::key::Keys::generate();
        let peers: Vec<nostr::key::PublicKey> = (0..n).map(|_| nostr::key::Keys::generate().public_key()).collect();
        let mut env_bytes = 0;
        let s = sample(cfg.repeat, || {
            let t = Instant::now();
            for (&l, peer) in leaves.iter().zip(&peers) {
                let b = issuer.bundle(tree, l).to_bytes();
                let e = nostr::nips::nip44::encrypt(sender.secret_key(), peer, &b[..], nostr::nips::nip44::Version::V2)
                    .expect("nip44 encrypt");
                env_bytes = e.len();
                black_box(&e);
            }
            ns(t)
        });
        nip.insert(
            format!("d{depth}"),
            with(timing(&s, n as f64, "bundle"), json!({ "bundle_bytes": bytes, "envelope_bytes": env_bytes, "n": n })),
        );
    }
    (Value::Object(plain), Value::Object(nip))
}

/// Returns the results and the sizes of the largest epoch for the memory note.
fn epoch_builds(cfg: &Cfg) -> (Value, (usize, usize, usize)) {
    let mut res = serde_json::Map::new();
    let mut biggest = (0, 0, 0);
    for (depth, assigned) in [(20u8, 900_000usize), (24, 1_000_000)] {
        let assigned = scale(cfg, assigned);
        for r in [1_000usize, 10_000, 50_000, 200_000] {
            let r = scale(cfg, r);
            let g = big_generation(depth, assigned as u32, r, 100 + u64::from(depth) + r as u64);
            let (mut cover_s, mut build_s, mut total_s) = (vec![], vec![], vec![]);
            let mut last: Option<(EpochOut, usize, usize)> = None;
            for _ in 0..cfg.repeat {
                let mut rng = XorShift64::new(7);
                let t = Instant::now();
                let cover = Cover::of(&g, cfg.sd);
                let tc = ns(t);
                let t2 = Instant::now();
                let out = build_epoch(&g, 4096, &mut rng, &cover);
                let tb = ns(t2);
                cover_s.push(tc);
                build_s.push(tb);
                total_s.push(tc + tb);
                last = Some((out, cover.len(), cover.recipe_bytes()));
            }
            let (out, cover_nodes, recipe_bytes) = last.expect("repeat > 0");
            let mut sizes = out.shard_sizes();
            sizes.sort_unstable();
            let shards_bytes: usize = sizes.iter().sum();
            if shards_bytes > biggest.0 {
                biggest = (shards_bytes, recipe_bytes, g.revoked.len() * 24);
            }
            let (cm, cmin, cmax) = stats(&cover_s);
            let (bm, _, _) = stats(&build_s);
            res.insert(
                format!("d{depth}_r{r}"),
                with(
                    timing(&total_s, 1.0, "epoch"),
                    json!({
                        "assigned": assigned, "revoked": r, "leaves_per_shard": 4096,
                        "cover_nodes": cover_nodes, "shards": sizes.len(),
                        "upload_bytes": out.upload_bytes(),
                        "shard_p50_bytes": percentile(&sizes, 50),
                        "shard_p95_bytes": percentile(&sizes, 95),
                        "shard_max_bytes": sizes[sizes.len() - 1],
                        "cover_median_ns": cm.round(), "cover_min_ns": cmin.round(), "cover_max_ns": cmax.round(),
                        "build_median_ns": bm.round(),
                    }),
                ),
            );
        }
    }
    (Value::Object(res), biggest)
}

fn opens_and_verifies(cfg: &Cfg) -> (Value, Value) {
    let (depth, assigned, r) = (24u8, scale(cfg, 1_000_000), scale(cfg, 10_000));
    let g = big_generation(depth, assigned as u32, r, 24);
    let cover = Cover::of(&g, cfg.sd);
    let mut rng = XorShift64::new(11);
    let out = build_epoch(&g, 4096, &mut rng, &cover);
    let ev = &out.event;
    let witness = &out.index(1).expect("index mode expected").bytes;
    let mut by_no: Vec<&[u8]> = vec![&[]; out.shard_sizes().len()];
    for s in out.shards() {
        by_no[s.no as usize] = &s.bytes;
    }
    let mean_shard = out.shard_sizes().iter().sum::<usize>() as f64 / by_no.len() as f64;
    let event_bytes = serde_json::to_vec(&out.event).expect("json").len();

    // The members: 10 000 distinct active leaves, bundles made before the clock starts.
    let n = scale(cfg, 10_000);
    let issuer = Issuer::new(cfg.sd, &g);
    let mut leaves = std::collections::BTreeSet::new();
    while leaves.len() < n {
        let l = rng.below(assigned as u64) as u32;
        if !g.revoked.contains(&l) {
            leaves.insert(l);
        }
    }
    let bundles: Vec<Bundle> = leaves.iter().map(|&l| issuer.bundle(g.tree, l)).collect();
    let nos: Vec<u32> = bundles.iter().map(|b| member::shard_of(b, ev).expect("shard")).collect();
    let mut checks = 0usize;
    let s = sample(cfg.repeat, || {
        checks = 0;
        let t = Instant::now();
        for (b, &no) in bundles.iter().zip(&nos) {
            let (k, c) = member::open_epoch_counted(b, ev, by_no[no as usize], witness).expect("opens");
            checks += c;
            black_box(&k);
        }
        ns(t)
    });
    let open = with(
        timing(&s, n as f64, "open"),
        json!({
            "members": n, "depth": depth, "assigned": assigned, "revoked": r,
            "shards": by_no.len(), "mean_shard_bytes": mean_shard.round(),
            "mean_checks": checks as f64 / n as f64,
        }),
    );
    drop(bundles);

    // verify_shard, index mode.
    let reps = scale(cfg, 1000);
    let s = sample(cfg.repeat, || {
        let t = Instant::now();
        for i in 0..reps {
            let no = i % by_no.len();
            black_box(shard::verify(ev, 1, no as u32, by_no[no], witness).expect("verifies"));
        }
        ns(t)
    });
    let index = with(
        timing(&s, reps as f64, "verify"),
        json!({
            "shards": by_no.len(), "mean_shard_bytes": mean_shard.round(),
            "index_bytes": witness.len(), "event_bytes": event_bytes,
        }),
    );
    drop(out);

    // Merkle mode: more than 512 shards.
    let mut rng = XorShift64::new(12);
    let merkle_lps: u32 = if cfg.quick { 32 } else { 256 };
    let out = build_epoch(&g, merkle_lps, &mut rng, &cover);
    let ev = &out.event;
    assert!(ev.trees[0].root.is_some(), "merkle mode expected");
    let event_bytes = serde_json::to_vec(&out.event).expect("json").len();
    let sizes = out.shard_sizes();
    let mean_shard = sizes.iter().sum::<usize>() as f64 / sizes.len() as f64;
    let mut by_no: Vec<(&[u8], &[u8])> = Vec::new();
    for s in out.shards() {
        let p = out.proof(1, s.no).expect("proof");
        by_no.push((&s.bytes, &p.bytes));
    }
    let s = sample(cfg.repeat, || {
        let t = Instant::now();
        for i in 0..reps {
            let no = i % by_no.len();
            let (b, p) = by_no[no];
            black_box(shard::verify(ev, 1, no as u32, b, p).expect("verifies"));
        }
        ns(t)
    });
    let merkle = with(
        timing(&s, reps as f64, "verify"),
        json!({
            "shards": by_no.len(), "leaves_per_shard": merkle_lps, "mean_shard_bytes": mean_shard.round(),
            "proof_bytes": by_no[0].1.len(), "event_bytes": event_bytes,
        }),
    );
    (open, json!({ "index": index, "merkle": merkle }))
}

fn member_id(rng: &mut XorShift64) -> MemberId {
    let mut b = [0u8; 32];
    for c in b.chunks_mut(8) {
        c.copy_from_slice(&rng.next_u64().to_le_bytes());
    }
    MemberId(b)
}

fn lazy_issue(cfg: &Cfg) -> Value {
    let n = scale(cfg, 10_000);
    let (mut join_s, mut lazy_s) = (vec![], vec![]);
    for _ in 0..cfg.repeat {
        let mut rng = XorShift64::new(31);
        let params = TreeParams { depth: 20, leaves_per_shard: 4096 };
        let rng_k = Box::new(XorShift64::new(32));
        let m = GenerationMaster::from_bytes([5; 32]);
        let mut k: Box<dyn KeyDistribution> = if cfg.sd {
            Box::new(SdKeeper::new(SCOPE, Policy::open(), params, m, 0).expect("keeper").with_rng(rng_k))
        } else {
            Box::new(CsKeeper::new(SCOPE, Policy::open(), params, m, 0).expect("keeper").with_rng(rng_k))
        };
        let ids: Vec<MemberId> = (0..n).map(|_| member_id(&mut rng)).collect();
        let t = Instant::now();
        for id in &ids {
            black_box(k.join(*id, 0).expect("join"));
        }
        join_s.push(ns(t));
        let th = Thresholds { capacity_ratio: 0.0, ..Thresholds::default() };
        k.migrate_if_needed(&th, &GenerationMaster::from_bytes([6; 32]), 0)
            .expect("a new master")
            .expect("migration starts");
        let t = Instant::now();
        for id in &ids {
            black_box(k.bundle_for(*id, 1).expect("bundle_for"));
        }
        lazy_s.push(ns(t));
    }
    let rate = |s: &[f64]| (n as f64 / (stats(s).0 / 1e9)).round();
    json!({
        "lazy": with(timing(&lazy_s, n as f64, "bundle"), json!({ "n": n, "bundles_per_sec": rate(&lazy_s) })),
        "join_issue": with(timing(&join_s, n as f64, "join"), json!({ "n": n, "bundles_per_sec": rate(&join_s) })),
    })
}

/// A keeper state of `members` members at depth 24 with `revoked` of
/// them put out, laid down directly (a million `join`s would issue a
/// million bundles), on a seeded source.
fn big_keeper_state(sd: bool, members: u32, revoked: usize) -> KeeperState {
    let scheme = if sd { SchemeId::SdV1 } else { SchemeId::CsV1 };
    let m = GenerationMaster::from_bytes([0x4d; 32]);
    let mut st = CsKeeper::new(SCOPE, Policy::closed(), TreeParams::default(), m, 0).expect("keeper").into_state();
    st.scheme = scheme;
    let mut rng = XorShift64::new(77);
    let mut out = std::collections::BTreeSet::new();
    while out.len() < revoked {
        out.insert(rng.below(u64::from(members)) as u32);
    }
    for leaf in 0..members {
        if !out.contains(&leaf) {
            let rec = MemberRec { since_epoch: 0, slots: [Some(GenLeaf { gen: 1, leaf }), None] };
            st.members.insert(member_id(&mut rng), rec);
        }
    }
    st.gens[0].next_leaf = members;
    st.gens[0].revoked = out;
    st
}

/// Times one operation and the rows it leaves (`take_changes` and their
/// JSON), median of `repeat` runs on fresh keepers.
fn state_delta_and_restore(cfg: &Cfg) -> (Value, Value) {
    let members = scale(cfg, 1_000_000) as u32;
    let revoked = scale(cfg, 20_000);
    let base = big_keeper_state(cfg.sd, members, revoked);
    let mut delta = serde_json::Map::new();
    type Op = fn(&mut dyn Delta, u64);
    let ops: [(&str, Op); 4] = [
        ("join", |k, i| k.op_join(i)),
        ("ban", |k, _| k.op_ban()),
        ("epoch", |k, _| k.op_epoch()),
        ("publish", |k, _| k.op_publish()),
    ];
    let mut keepers: Vec<Box<dyn Delta>> = (0..cfg.repeat).map(|_| restore(cfg.sd, base.clone())).collect();
    for (name, op) in ops {
        let (mut take_s, mut bytes) = (vec![], 0);
        for (i, k) in keepers.iter_mut().enumerate() {
            op(k.as_mut(), i as u64);
            let t = Instant::now();
            bytes = k.rows();
            take_s.push(ns(t));
        }
        delta.insert(name.into(), with(timing(&take_s, 1.0, "take_changes + JSON"), json!({ "bytes": bytes })));
    }
    drop(keepers);
    let mut ins = delta;
    ins.insert("members".into(), json!(members));
    ins.insert("revoked".into(), json!(revoked));
    let (mut save_s, mut parse_s, mut restore_s, mut bytes) = (vec![], vec![], vec![], 0);
    for _ in 0..cfg.repeat {
        let t = Instant::now();
        let json = serde_json::to_vec(&base).expect("json");
        save_s.push(ns(t));
        bytes = json.len();
        let t = Instant::now();
        let st: KeeperState = serde_json::from_slice(&json).expect("parses");
        parse_s.push(ns(t));
        drop(json);
        let t = Instant::now();
        black_box(restore(cfg.sd, st));
        restore_s.push(ns(t));
    }
    let restore_v = with(
        timing(&restore_s, 1.0, "from_state"),
        json!({
            "members": members, "revoked": revoked, "bytes": bytes,
            "save_median_ns": stats(&save_s).0.round(), "parse_median_ns": stats(&parse_s).0.round(),
        }),
    );
    (Value::Object(ins), restore_v)
}

/// The operations of `state_delta` on a keeper of either scheme.
trait Delta {
    fn op_join(&mut self, i: u64);
    fn op_ban(&mut self);
    fn op_epoch(&mut self);
    fn op_publish(&mut self);
    /// Takes the rows and gives the bytes of their JSON.
    fn rows(&mut self) -> usize;
}

impl<S: keydist::Scheme> Delta for keydist::TreeKeeper<S> {
    fn op_join(&mut self, i: u64) {
        let mut rng = XorShift64::new(1000 + i);
        self.join(member_id(&mut rng), 10).expect("join");
    }

    fn op_ban(&mut self) {
        let m = *self.state().members.keys().next().expect("a member");
        self.revoke(&[m], Revoke::Ban, 10);
    }

    fn op_epoch(&mut self) {
        self.rotate(10, true).built().expect("an epoch");
    }

    fn op_publish(&mut self) {
        let e = self.state().pending.as_ref().expect("pending").key.epoch;
        assert!(self.published(e));
    }

    fn rows(&mut self) -> usize {
        serde_json::to_vec(&self.take_changes()).expect("json").len()
    }
}

fn restore(sd: bool, st: KeeperState) -> Box<dyn Delta> {
    let rng = || Box::new(XorShift64::new(78));
    if sd {
        Box::new(SdKeeper::from_state(st).expect("restores").with_rng(rng()))
    } else {
        Box::new(CsKeeper::from_state(st).expect("restores").with_rng(rng()))
    }
}


fn flat(cfg: &Cfg) -> serde_json::Map<String, Value> {
    let mut res = serde_json::Map::new();
    let mut builds = serde_json::Map::new();
    for members in [1_000usize, 10_000, 50_000] {
        let members = scale(cfg, members);
        let mut rng = XorShift64::new(41);
        let ids: Vec<MemberId> = (0..members).map(|_| member_id(&mut rng)).collect();
        let (mut join_s, mut build_s) = (vec![], vec![]);
        let mut bytes = 0;
        let mut ev = None;
        for _ in 0..cfg.repeat {
            let mut k = FlatKeeper::new(SCOPE, Policy::open()).with_rng(Box::new(XorShift64::new(42)));
            let t = Instant::now();
            for id in &ids {
                black_box(k.join(*id, 0).expect("join"));
            }
            join_s.push(ns(t));
            let t = Instant::now();
            let out = k.rotate(0, true).built().expect("epoch");
            build_s.push(ns(t));
            bytes = out.secrets.iter().map(|(_, b)| b.len()).sum::<usize>();
            ev = Some(out);
        }
        let out = ev.expect("repeat > 0");
        if members == scale(cfg, 1_000) {
            let secret = &out.secrets[0].1;
            let s = sample(cfg.repeat, || {
                let t = Instant::now();
                for _ in 0..scale(cfg, 10_000) {
                    black_box(keydist::open_secret(&out.event, secret).expect("opens"));
                }
                ns(t)
            });
            res.insert("flat_open_secret".into(), timing(&s, scale(cfg, 10_000) as f64, "open").into());
        }
        res.insert(format!("flat_join.n{members}"), with(timing(&join_s, members as f64, "join"), json!({ "n": members })));
        builds.insert(
            format!("n{members}"),
            with(timing(&build_s, 1.0, "epoch"), json!({ "members": members, "secret_bytes_total": bytes })),
        );
    }
    res.insert("flat_epoch_build".into(), Value::Object(builds));
    res
}

fn proc_kb(key: &str) -> Option<u64> {
    let s = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = s.lines().find(|l| l.starts_with(key))?;
    line.split_whitespace().nth(1)?.parse().ok()
}

fn env() -> Value {
    let cpuinfo = std::fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
    let model = ["Hardware", "model name", "Model"]
        .iter()
        .find_map(|k| {
            cpuinfo
                .lines()
                .find(|l| l.starts_with(k))
                .and_then(|l| l.split_once(':'))
                .map(|(_, v)| v.trim().to_string())
        })
        .map(Value::from)
        .unwrap_or(Value::Null);
    let cores = cpuinfo.lines().filter(|l| l.starts_with("processor")).count();
    let target = format!(
        "{}-{}{}",
        if cfg!(target_arch = "aarch64") {
            "aarch64"
        } else if cfg!(target_arch = "x86_64") {
            "x86_64"
        } else {
            "other"
        },
        if cfg!(target_os = "android") {
            "android"
        } else if cfg!(target_os = "linux") {
            "linux"
        } else if cfg!(target_os = "macos") {
            "macos"
        } else if cfg!(target_os = "windows") {
            "windows"
        } else {
            "other"
        },
        if cfg!(debug_assertions) { " (debug build)" } else { "" },
    );
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    json!({
        "target": target,
        "arch": std::env::consts::ARCH,
        "os": std::env::consts::OS,
        "pointer_width": usize::BITS,
        "threads": std::thread::available_parallelism().map(|n| n.get()).unwrap_or(0),
        "cpu_model": model,
        "cpu_cores": cores,
        "timestamp_utc": utc(secs),
    })
}

/// `YYYY-MM-DDTHH:MM:SSZ` from Unix seconds (civil-from-days algorithm).
fn utc(secs: u64) -> String {
    let (days, rem) = ((secs / 86_400) as i64, secs % 86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

fn main() {
    let cfg = match parse_args() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("keydist-bench: {e}");
            std::process::exit(2);
        }
    };
    let step = |name: &str| eprintln!("keydist-bench: {name}");
    let mut results = serde_json::Map::new();
    let mut largest = (0, 0, 0);
    if cfg.flat {
        step("flat");
        results.extend(flat(&cfg));
    } else {
        step("bundle_issue");
        let (plain, nip) = bundle_issue(&cfg);
        results.insert("bundle_issue".into(), plain);
        results.insert("bundle_issue_nip44".into(), nip);
        step("epoch_build");
        let (eb, big) = epoch_builds(&cfg);
        results.insert("epoch_build".into(), eb);
        largest = big;
        step("open_epoch, verify_shard");
        let (open, verify) = opens_and_verifies(&cfg);
        results.insert("open_epoch".into(), open);
        results.insert("verify_shard".into(), verify);
        step("lazy_issue");
        let lazy = lazy_issue(&cfg);
        results.insert("lazy_issue".into(), lazy["lazy"].clone());
        results.insert("join_issue".into(), lazy["join_issue"].clone());
        step("state_delta, state_restore");
        let (delta, restore) = state_delta_and_restore(&cfg);
        results.insert("state_delta".into(), delta);
        results.insert("state_restore".into(), restore);
    }
    results.insert(
        "memory".into(),
        json!({
            "vm_hwm_kb": proc_kb("VmHWM:"),
            "vm_rss_kb": proc_kb("VmRSS:"),
            "largest_epoch_shards_bytes": largest.0,
            "largest_epoch_recipe_bytes": largest.1,
            "largest_epoch_revoked_set_bytes_estimate": largest.2,
        }),
    );
    let doc = json!({ "env": env(), "scheme": if cfg.flat { "flat" } else if cfg.sd { "sd" } else { "cs" }, "repeat": cfg.repeat, "quick": cfg.quick, "results": results });
    let text = serde_json::to_string_pretty(&doc).expect("json");
    if let Some(dir) = std::path::Path::new(&cfg.out).parent().filter(|d| !d.as_os_str().is_empty()) {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(&cfg.out, text) {
        eprintln!("keydist-bench: cannot write {}: {e}", cfg.out);
        std::process::exit(1);
    }
    eprintln!("keydist-bench: {}", cfg.out);
}
