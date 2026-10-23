// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! `keydist-sim`: a deterministic discrete-time simulator of a community on
//! one scope (internal/communities-keydist.md, 10 stage 2 and 12).
//!
//! One step is one minute. Per day the joins, leaves and bans are spread
//! over the minutes by a seeded generator; the keeper is driven through its
//! real API (`join`, `revoke`, `rotate`, `pending`, `published`,
//! `migrate_if_needed`). Only two things bypass the API for speed: the
//! initial population (the state is built directly, as `tests/timing.rs`
//! does) and the bulk of a lazy migration to a new generation (a few
//! hundred members a day go through `bundle_for`, the rest get their leaf
//! by a direct state edit, because a million HKDF bundles would only
//! measure the host's CPU).
//!
//! What it counts (a GET is one fetch of an event, a shard or a proof from
//! the relay or blob store):
//!
//! - an online client fetches the event and its own shard (and its proof in
//!   Merkle mode) for every epoch built while it is online; a client online
//!   a share `p` of the time sees `p` of the epochs. The shard sizes a
//!   client fetches are sampled from the real shards of every epoch (32
//!   random members, each online with probability `p`, decided by the
//!   generator, and the shard of its own leaf);
//! - a client back after `D` days offline fetches the current key (one
//!   event, one shard, one proof in Merkle mode) and, to read the past, per
//!   missed epoch: under `open` history the chain sits in the events (the
//!   specification counts 0 extra; the events themselves must still be read
//!   one by one, reported apart); under `from-join` one shard (and proof)
//!   per missed epoch, the events apart. Epochs in the window come from the
//!   log of the run, or the run's mean rate times `D` when `D` is longer
//!   than the run (marked `*`).
//!
//! Members who leave or are banned are drawn uniformly from all members
//! (the worst case for the cover: revoked leaves scattered everywhere).

use std::collections::HashMap;
use std::time::Instant;

use keydist::keeper::MemberRec;
use keydist::{BlobKind, KeeperState, Rotate, SdKeeper, GenerationMaster,
    CsKeeper, EpochOut, FlatKeeper, GenerationKind, History, KeyDistribution, MemberId, Policy,
    Revoke, SchemeId, ScopeRef, Thresholds, TreeParams, XorShift64,
};

const T0: u64 = 1_800_000_000;
const DAY_MIN: u64 = 1440;
/// Clients sampled per epoch for the shard a client fetches.
const SAMPLES: usize = 32;
/// Members per day that migrate through the real `bundle_for`.
const REAL_MIGRATIONS: usize = 300;
/// The NIP-44 envelope of a bundle, bytes (measured by keydist-bench ~100).
const NIP44_OVERHEAD: usize = 100;

struct Args {
    label: String,
    members: u64,
    depth: u8,
    lps: u32,
    days: u64,
    history: History,
    open_scope: bool,
    joins_pct: f64,
    leaves_pct: f64,
    bans_per_day: u64,
    spam_wave: u64,
    online_pct: f64,
    active_daily_pct: f64,
    ban_window: u64,
    leave_window: u64,
    join_window: u64,
    min_interval: u64,
    grace_days: u64,
    revoked_ratio: f64,
    p95_kb: usize,
    capacity_ratio: f64,
    seed: u64,
    scheme: SchemeId,
    csv: Option<String>,
    md: Option<String>,
    quiet: bool,
}

fn usage() -> ! {
    eprintln!(
        "usage: keydist-sim --preset 10k|100k|1m|10m [--members N] [--depth 14|17|20|24] \
[--leaves-per-shard 1024|4096|16384]\n  [--days D] [--history open|from-join] [--scope open|closed] \
[--joins-pct 1.0] [--leaves-pct 0.3]\n  [--bans-per-day 100] [--spam-wave 2000] [--online-pct 20] \
[--active-daily-pct 60] [--ban-window 300]\n  [--leave-window 3600] [--join-window 300] [--min-interval 300] [--grace-days 30] [--revoked-ratio 0.5] \
[--p95-kb 256] [--capacity-ratio 0.9]\n  [--seed 1] [--scheme cs|sd|flat] [--label NAME] [--csv path] [--md path] [--quiet]"
    );
    std::process::exit(2)
}

fn parse_args() -> Args {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut kv: HashMap<String, String> = HashMap::new();
    let mut quiet = false;
    let mut i = 0;
    while i < argv.len() {
        let k = argv[i].trim_start_matches("--").to_string();
        if !argv[i].starts_with("--") {
            usage();
        }
        if k == "quiet" {
            quiet = true;
            i += 1;
            continue;
        }
        let Some(v) = argv.get(i + 1) else { usage() };
        kv.insert(k, v.clone());
        i += 2;
    }
    let preset = kv.get("preset").cloned();
    let (pm, pd) = match preset.as_deref() {
        Some("10k") => (10_000, 17),
        Some("100k") => (100_000, 20),
        Some("1m") | None => (1_000_000, 24),
        Some("10m") => (10_000_000, 24),
        Some(_) => usage(),
    };
    fn num<T: std::str::FromStr>(kv: &HashMap<String, String>, k: &str, d: T) -> T {
        match kv.get(k) {
            Some(v) => v.parse().unwrap_or_else(|_| {
                eprintln!("bad value for --{k}: {v}");
                usage()
            }),
            None => d,
        }
    }
    let members: u64 = num(&kv, "members", pm);
    // Below a million members the bans scale down with the community, so
    // that 10k members are not banned out of existence in five days.
    let scale = (members as f64 / 1_000_000.0).min(1.0);
    let bans_default = ((100.0 * scale).round() as u64).max(1);
    let spam_default = ((2000.0 * scale).round() as u64).max(1);
    let history = match kv.get("history").map(String::as_str) {
        None | Some("open") => History::Open,
        Some("from-join") => History::FromJoin,
        Some(_) => usage(),
    };
    let open_scope = match kv.get("scope").map(String::as_str) {
        None | Some("closed") => false,
        Some("open") => true,
        Some(_) => usage(),
    };
    let scheme = match kv.get("scheme").map(String::as_str) {
        None | Some("cs") => SchemeId::CsV1,
        Some("flat") => SchemeId::FlatV1,
        Some("sd") => SchemeId::SdV1,
        Some(_) => usage(),
    };
    let a = Args {
        label: kv.get("label").cloned().unwrap_or_else(|| preset.clone().unwrap_or_else(|| format!("{members}"))),
        members,
        depth: num(&kv, "depth", pd),
        lps: num(&kv, "leaves-per-shard", 4096),
        days: num(&kv, "days", 30),
        history,
        open_scope,
        joins_pct: num(&kv, "joins-pct", 1.0),
        leaves_pct: num(&kv, "leaves-pct", 0.3),
        bans_per_day: num(&kv, "bans-per-day", bans_default),
        spam_wave: num(&kv, "spam-wave", spam_default),
        online_pct: num(&kv, "online-pct", 20.0),
        active_daily_pct: num(&kv, "active-daily-pct", 60.0),
        ban_window: num(&kv, "ban-window", 300),
        leave_window: num(&kv, "leave-window", 3600),
        join_window: num(&kv, "join-window", 300),
        min_interval: num(&kv, "min-interval", 300),
        grace_days: num(&kv, "grace-days", 30),
        revoked_ratio: num(&kv, "revoked-ratio", 0.5),
        p95_kb: num(&kv, "p95-kb", 256),
        capacity_ratio: num(&kv, "capacity-ratio", 0.9),
        seed: num(&kv, "seed", 1),
        scheme,
        csv: kv.get("csv").cloned(),
        md: kv.get("md").cloned(),
        quiet,
    };
    if a.days == 0 || a.members == 0 {
        usage();
    }
    a
}

/// A keeper of either scheme, with the few operations that need the
/// concrete type.
enum Keeper {
    Cs(CsKeeper),
    Sd(SdKeeper),
    Flat(FlatKeeper),
    Taken,
}

impl Keeper {
    fn kd(&mut self) -> &mut dyn KeyDistribution {
        match self {
            Keeper::Cs(k) => k,
            Keeper::Sd(k) => k,
            Keeper::Flat(k) => k,
            Keeper::Taken => unreachable!("keeper taken"),
        }
    }

    /// The state of a tree keeper.
    fn tree_state(&self) -> Option<&KeeperState> {
        match self {
            Keeper::Cs(k) => Some(k.state()),
            Keeper::Sd(k) => Some(k.state()),
            _ => None,
        }
    }

    fn take_tree_state(&mut self) -> Option<KeeperState> {
        match std::mem::replace(self, Keeper::Taken) {
            Keeper::Cs(k) => Some(k.into_state()),
            Keeper::Sd(k) => Some(k.into_state()),
            other => {
                *self = other;
                None
            }
        }
    }

    /// A tree keeper from a state, on a seeded generator, without the
    /// change log (the simulator keeps no rows).
    fn from_tree_state(scheme: SchemeId, st: KeeperState, rng: Box<dyn keydist::Rng>) -> Keeper {
        match scheme {
            SchemeId::SdV1 => {
                let mut k = SdKeeper::from_state(st).expect("state is valid").with_rng(rng);
                k.set_change_log(false);
                Keeper::Sd(k)
            }
            _ => {
                let mut k = CsKeeper::from_state(st).expect("state is valid").with_rng(rng);
                k.set_change_log(false);
                Keeper::Cs(k)
            }
        }
    }
}

fn rand_id(rng: &mut XorShift64) -> MemberId {
    let mut b = [0u8; 32];
    for c in b.chunks_mut(8) {
        c.copy_from_slice(&rng.next_u64().to_le_bytes());
    }
    MemberId(b)
}

/// Counts of sizes: exact quantiles without keeping every shard.
#[derive(Default, Clone)]
struct Hist {
    counts: HashMap<u32, u64>,
    total: u64,
    max: u32,
}

impl Hist {
    fn add(&mut self, v: usize) {
        let v = v as u32;
        *self.counts.entry(v).or_insert(0) += 1;
        self.total += 1;
        self.max = self.max.max(v);
    }
    fn merge(&mut self, o: &Hist) {
        for (k, c) in &o.counts {
            *self.counts.entry(*k).or_insert(0) += c;
        }
        self.total += o.total;
        self.max = self.max.max(o.max);
    }
    fn quantile(&self, q: f64) -> u32 {
        if self.total == 0 {
            return 0;
        }
        let mut v: Vec<(u32, u64)> = self.counts.iter().map(|(k, c)| (*k, *c)).collect();
        v.sort_unstable();
        let need = ((self.total as f64 * q).ceil() as u64).max(1);
        let mut acc = 0;
        for (k, c) in v {
            acc += c;
            if acc >= need {
                return k;
            }
        }
        self.max
    }
}

#[derive(Default)]
struct Day {
    epochs: u64,
    upload: u64,
    hist: Hist,
    event_sum: u64,
    cover_sum: u64,
    cover_max: u64,
    client_bytes: f64,
    client_gets: f64,
    joins: u64,
    leaves: u64,
    bans: u64,
}

struct EpochLog {
    at_min: u64,
    event: usize,
    /// Mean bytes of the shard a sampled client fetches, and of its witness
    /// (the index blob or the proof; 0 for flat).
    shard: f64,
    witness: f64,
}

struct Row {
    day: u64,
    epochs: u64,
    upload: u64,
    p50: u32,
    p95: u32,
    max: u32,
    event_mean: u64,
    members: u64,
    revoked: u64,
    next_leaf: u64,
    generation: u32,
    cover_mean: f64,
    cover_max: u64,
    joins: u64,
    leaves: u64,
    bans: u64,
}

#[derive(Default)]
struct Reasons {
    first: u64,
    ban: u64,
    leave: u64,
    join: u64,
    lazy: u64,
}

struct GenEvent {
    day: u64,
    gen: u32,
    kind: String,
}

fn vm_hwm_kb() -> Option<u64> {
    let s = std::fs::read_to_string("/proc/self/status").ok()?;
    s.lines().find(|l| l.starts_with("VmHWM:"))?.split_whitespace().nth(1)?.parse().ok()
}

fn fmt_b(b: f64) -> String {
    if b >= 1e9 {
        format!("{:.2} GB", b / 1e9)
    } else if b >= 1e6 {
        format!("{:.2} MB", b / 1e6)
    } else if b >= 1e3 {
        format!("{:.1} KB", b / 1e3)
    } else {
        format!("{b:.0} B")
    }
}

/// One day's lazy migration of members that become active: some through
/// the real `bundle_for`, the rest by editing the state. Returns the
/// number migrated and the longest `missed` page a real bundle carried.
fn migrate_day(
    keeper: &mut Keeper,
    list: &mut Vec<MemberId>,
    rng: &mut XorShift64,
    active_pct: f64,
    reseed: u64,
    now: u64,
) -> (u64, usize) {
    let Some(st) = keeper.tree_state() else { return (0, 0) };
    let unm = st.unmigrated;
    if unm == 0 {
        list.clear();
        return (0, 0);
    }
    let want = ((unm as f64 * active_pct / 100.0).ceil() as usize).max(1);
    let gen = st.gens.last().expect("a generation").gen;
    let mut chosen: Vec<MemberId> = Vec::with_capacity(want.min(list.len()));
    while chosen.len() < want && !list.is_empty() {
        let idx = rng.below(list.len() as u64) as usize;
        let m = list.swap_remove(idx);
        if st.members.get(&m).is_some_and(|r| r.leaf_in(gen).is_none()) {
            chosen.push(m);
        }
    }
    let mut done = 0u64;
    let mut missed_len = 0usize;
    let real = chosen.len().min(REAL_MIGRATIONS);
    for m in &chosen[..real] {
        if let Ok(iss) = keeper.kd().bundle_for(*m, now) {
            done += 1;
            missed_len = missed_len.max(iss.missed.len());
        }
    }
    if chosen.len() > real {
        let scheme = keeper.kd().scheme();
        let mut st = keeper.take_tree_state().expect("a tree keeper");
        for m in &chosen[real..] {
            let Some(rec) = st.members.get_mut(m) else { continue };
            let Ok(leaf) = st.gens.last_mut().expect("a generation").assign() else { break };
            rec.put(gen, leaf);
            // `unmigrated` must equal the members without a leaf in the
            // current generation, or `from_state` refuses the state.
            st.unmigrated = st.unmigrated.saturating_sub(1);
            done += 1;
        }
        if st.unmigrated == 0 {
            st.missed.clear();
        }
        *keeper = Keeper::from_tree_state(scheme, st, Box::new(XorShift64::new(reseed)));
    }
    (done, missed_len)
}

/// Mean bytes of the shard and of its witness (the index blob, or the
/// proof in Merkle mode) a sampled online client fetches for one epoch.
fn sample_clients(
    keeper: &Keeper,
    pop: &[MemberId],
    out: &EpochOut,
    rng: &mut XorShift64,
    online_pct: f64,
) -> (f64, f64) {
    let Some(st) = keeper.tree_state() else {
        return (keydist::flat::FLAT_SECRET_LEN as f64, 0.0);
    };
    let sizes: HashMap<(u32, u32), usize> = out.shards().map(|s| ((s.gen, s.no), s.bytes.len())).collect();
    let mut wit: HashMap<(u32, u32), usize> = HashMap::new();
    for b in &out.blobs {
        match b.kind {
            BlobKind::Index => {
                wit.insert((b.gen, u32::MAX), b.bytes.len());
            }
            BlobKind::Proof => {
                wit.insert((b.gen, b.no), b.bytes.len());
            }
            BlobKind::Shard => {}
        }
    }
    let lps = u64::from(st.params.leaves_per_shard);
    let cur = st.gens.last().expect("a generation").gen;
    let (mut sum, mut wsum, mut n) = (0.0, 0.0, 0u32);
    for _ in 0..SAMPLES {
        let m = pop[rng.below(pop.len() as u64) as usize];
        // A member is online at this epoch with probability online-pct.
        if (rng.below(10_000) as f64) >= online_pct * 100.0 {
            continue;
        }
        let Some(rec) = st.members.get(&m) else { continue };
        let slot = rec.slots.iter().flatten().find(|s| s.gen == cur).or_else(|| rec.slots.iter().flatten().next());
        let Some(slot) = slot else { continue };
        let no = (u64::from(slot.leaf) / lps) as u32;
        if let Some(sz) = sizes.get(&(slot.gen, no)) {
            sum += *sz as f64;
            wsum += wit.get(&(slot.gen, u32::MAX)).or_else(|| wit.get(&(slot.gen, no))).copied().unwrap_or(0) as f64;
            n += 1;
        }
    }
    if n == 0 {
        // Nobody sampled was online: use the mean shard and witness.
        let cnt = sizes.len().max(1) as f64;
        let mean = sizes.values().sum::<usize>() as f64 / cnt;
        let w = wit.values().sum::<usize>() as f64 / wit.len().max(1) as f64;
        return (mean, w);
    }
    (sum / f64::from(n), wsum / f64::from(n))
}

fn main() {
    let a = parse_args();
    let wall = Instant::now();
    let scope = ScopeRef { community: [1; 32], scope: [2; 32] };
    let params = TreeParams { depth: a.depth, leaves_per_shard: a.lps };
    let mut rng = XorShift64::new(a.seed.wrapping_mul(0x9e37_79b9).wrapping_add(17));
    let keeper_rng = || Box::new(XorShift64::new(a.seed ^ 0xabcd_ef01));
    let mut pop: Vec<MemberId> = Vec::with_capacity(a.members as usize * 11 / 10);

    // The initial population, built into the state directly.
    let thresholds = Thresholds {
        capacity_ratio: a.capacity_ratio,
        revoked_ratio: a.revoked_ratio,
        shard_p95_bytes: a.p95_kb * 1024,
        grace_secs: a.grace_days * 86_400,
    };
    let policy = Policy::new(a.ban_window, a.leave_window, a.open_scope, a.history)
        .with_join_window(a.join_window)
        .with_min_epoch_interval(a.min_interval)
        .with_thresholds(thresholds.clone());
    let mut keeper = match a.scheme {
        SchemeId::FlatV1 => {
            let mut st = FlatKeeper::new(scope, policy.clone()).state().clone();
            for i in 0..a.members {
                let id = rand_id(&mut rng);
                st.members.insert(id, keydist::flat::FlatRec { since_epoch: 0, ordinal: i as u32 });
                pop.push(id);
            }
            st.next_ordinal = a.members as u32;
            Keeper::Flat(FlatKeeper::from_state(st).expect("flat state").with_rng(keeper_rng()))
        }
        scheme => {
            let master = GenerationMaster::from_bytes([0x4d; 32]);
            let mut st = match scheme {
                SchemeId::SdV1 => SdKeeper::new(scope, policy.clone(), params, master, T0).map(SdKeeper::into_state),
                _ => CsKeeper::new(scope, policy.clone(), params, master, T0).map(CsKeeper::into_state),
            }
            .unwrap_or_else(|e| {
                eprintln!("bad parameters: {e}");
                std::process::exit(2)
            });
            if a.members > st.gens[0].tree.capacity() {
                eprintln!("{} members do not fit a tree of depth {}", a.members, a.depth);
                std::process::exit(2);
            }
            for i in 0..a.members {
                let id = rand_id(&mut rng);
                st.members.insert(
                    id,
                    MemberRec { since_epoch: 0, slots: [Some(keydist::keeper::GenLeaf { gen: 1, leaf: i as u32 }), None] },
                );
                pop.push(id);
            }
            st.gens[0].next_leaf = a.members as u32;
            Keeper::from_tree_state(scheme, st, keeper_rng())
        }
    };
    let setup_s = wall.elapsed().as_secs_f64();

    let bundle_len = {
        let m = pop[0];
        keeper.kd().bundle_for(m, T0).map(|i| i.bundle.len()).unwrap_or(0)
    };

    let mut days: Vec<Row> = Vec::new();
    let mut run_hist = Hist::default();
    let mut elog: Vec<EpochLog> = Vec::new();
    let mut reasons = Reasons::default();
    let mut gens: Vec<GenEvent> = Vec::new();
    let mut slowest_ms = 0.0f64;
    let mut build_total_ms = 0.0f64;
    let (mut epochs_total, mut upload_total, mut event_total) = (0u64, 0u64, 0u64);
    let (mut cover_max_run, mut cover_sum_run) = (0u64, 0u64);
    let (mut merkle_epochs, mut event_max) = (0u64, 0usize);
    let mut missed_peak = 0usize;
    let mut unmigrated_peak = 0u64;
    let mut missed_keys_seen = 0usize;
    let mut migrated_by_gen: HashMap<u32, u64> = HashMap::new();
    let mut mig_list: Vec<MemberId> = Vec::new();
    let (mut full_rejects, mut gen_ops) = (0u64, 0u64);
    let (mut client_bytes_run, mut client_gets_run) = (0.0f64, 0.0f64);
    let p_online = a.online_pct / 100.0;
    let (mut bans_q, mut leaves_q) = (0u64, 0u64);

    for day in 0..a.days {
        let n0 = pop.len() as f64;
        let nj = (n0 * a.joins_pct / 100.0).round() as usize;
        let nl = (n0 * a.leaves_pct / 100.0).round() as usize;
        let mut joins = vec![0u32; DAY_MIN as usize];
        let mut leaves = vec![0u32; DAY_MIN as usize];
        let mut bans = vec![0u32; DAY_MIN as usize];
        for _ in 0..nj {
            joins[rng.below(DAY_MIN) as usize] += 1;
        }
        for _ in 0..nl {
            leaves[rng.below(DAY_MIN) as usize] += 1;
        }
        for _ in 0..a.bans_per_day {
            bans[rng.below(DAY_MIN) as usize] += 1;
        }
        let hour = rng.below(24);
        for _ in 0..a.spam_wave {
            bans[(hour * 60 + rng.below(60)) as usize] += 1;
        }

        // The members who become active today migrate to the new generation.
        if !mig_list.is_empty() {
            let now = T0 + day * 86_400;
            let (n, ml) = migrate_day(&mut keeper, &mut mig_list, &mut rng, a.active_daily_pct, a.seed ^ ((day + 1) * 0x51ed), now);
            if n > 0 {
                let g = keeper.kd().stats().generation;
                *migrated_by_gen.entry(g).or_insert(0) += n;
            }
            missed_keys_seen = missed_keys_seen.max(ml);
        }

        let mut d = Day::default();
        for m in 0..DAY_MIN {
            let now = T0 + day * 86_400 + m * 60;
            for _ in 0..joins[m as usize] {
                let id = rand_id(&mut rng);
                match keeper.kd().join(id, now) {
                    Ok(_) => {
                        pop.push(id);
                        d.joins += 1;
                    }
                    Err(_) => full_rejects += 1,
                }
            }
            for _ in 0..leaves[m as usize] {
                if pop.is_empty() {
                    break;
                }
                let idx = rng.below(pop.len() as u64) as usize;
                let id = pop.swap_remove(idx);
                keeper.kd().revoke(&[id], Revoke::Leave, now);
                leaves_q += 1;
                d.leaves += 1;
            }
            for _ in 0..bans[m as usize] {
                if pop.is_empty() {
                    break;
                }
                let idx = rng.below(pop.len() as u64) as usize;
                let id = pop.swap_remove(idx);
                keeper.kd().revoke(&[id], Revoke::Ban, now);
                bans_q += 1;
                d.bans += 1;
            }
            let mut master = [0u8; 32];
            for c in master.chunks_mut(8) {
                c.copy_from_slice(&rng.next_u64().to_le_bytes());
            }
            if let Some(go) = keeper.kd().migrate_if_needed(&thresholds, &GenerationMaster::from_bytes(master), now).expect("migration") {
                gen_ops += 1;
                let kind = match &go.kind {
                    GenerationKind::Planned { reason, .. } => format!("planned:{reason:?}").to_lowercase(),
                    GenerationKind::Emergency => "emergency".into(),
                    GenerationKind::Initial => "initial".into(),
                };
                gens.push(GenEvent { day, gen: go.gen, kind });
                mig_list = pop.clone();
            }
            let sched = keeper.kd().stats().schedule;
            let first = keeper.kd().stats().epoch == 0;
            let t = Instant::now();
            let Rotate::Built(out) = keeper.kd().rotate(now, false) else { continue };
            let ms = t.elapsed().as_secs_f64() * 1000.0;
            slowest_ms = slowest_ms.max(ms);
            build_total_ms += ms;
            assert!(keeper.kd().pending().is_some(), "a built epoch must be pending");
            keeper.kd().published(out.event.epoch);

            // Reasons.
            if first {
                reasons.first += 1;
            }
            if sched.queued > 0 && bans_q > 0 {
                reasons.ban += 1;
            }
            if sched.queued > 0 && leaves_q > 0 && !a.open_scope {
                reasons.leave += 1;
            }
            if sched.waiting > 0 {
                reasons.join += 1;
            }
            if sched.lazy > 0 {
                reasons.lazy += 1;
            }
            bans_q = 0;
            leaves_q = 0;

            // Sizes.
            let mut up = out.upload_bytes();
            let sizes = out.shard_sizes();
            if a.scheme == SchemeId::FlatV1 {
                up = out.secrets.iter().map(|(_, b)| b.len()).sum();
            }
            for s in &sizes {
                d.hist.add(*s);
            }
            let ev = serde_json::to_vec(&out.event).map(|v| v.len()).unwrap_or(0);
            if out.event.trees.iter().any(|t| t.root.is_some()) {
                merkle_epochs += 1;
            }
            event_max = event_max.max(ev);
            let cover: u64 = out.cover.iter().map(|(_, c)| *c as u64).sum();
            d.cover_sum += cover;
            d.cover_max = d.cover_max.max(cover);
            d.epochs += 1;
            d.upload += up as u64;
            d.event_sum += ev as u64;
            let (cb, wb) = sample_clients(&keeper, &pop, &out, &mut rng, a.online_pct);
            // Event, witness (index or proof), shard: three GETs.
            let gets = if wb > 0.0 { 3.0 } else { 2.0 };
            d.client_bytes += p_online * (cb + wb + ev as f64);
            d.client_gets += p_online * gets;
            elog.push(EpochLog { at_min: day * DAY_MIN + m, event: ev, shard: cb, witness: wb });
            missed_peak = missed_peak.max(keeper.kd().stats().missed_len as usize);
            unmigrated_peak = unmigrated_peak.max(keeper.kd().stats().unmigrated);
        }
        let st = keeper.kd().stats();
        unmigrated_peak = unmigrated_peak.max(st.unmigrated);
        let (p50, p95, max) = (d.hist.quantile(0.5), d.hist.quantile(0.95), d.hist.max);
        days.push(Row {
            day: day + 1,
            epochs: d.epochs,
            upload: d.upload,
            p50,
            p95,
            max,
            event_mean: d.event_sum.checked_div(d.epochs).unwrap_or(0),
            members: st.members,
            revoked: st.revoked,
            next_leaf: st.next_leaf,
            generation: st.generation,
            cover_mean: if d.epochs > 0 { d.cover_sum as f64 / d.epochs as f64 } else { 0.0 },
            cover_max: d.cover_max,
            joins: d.joins,
            leaves: d.leaves,
            bans: d.bans,
        });
        epochs_total += d.epochs;
        upload_total += d.upload;
        event_total += d.event_sum;
        cover_max_run = cover_max_run.max(d.cover_max);
        cover_sum_run += d.cover_sum;
        client_bytes_run += d.client_bytes;
        client_gets_run += d.client_gets;
        run_hist.merge(&d.hist);
        if !a.quiet && day % 5 == 4 {
            eprintln!(
                "day {}: {} epochs, members {}, revoked {}, gen {}, {:.0}s",
                day + 1,
                d.epochs,
                st.members,
                st.revoked,
                st.generation,
                wall.elapsed().as_secs_f64()
            );
        }
    }

    // ---- results ----
    let final_stats = keeper.kd().stats();
    let revoked_all: u64 = match &keeper {
        Keeper::Cs(cs) => cs.state().gens.iter().map(|g| g.revoked.len() as u64).sum(),
        _ => 0,
    };
    let est_mem = final_stats.members as f64 * (32.0 + std::mem::size_of::<MemberRec>() as f64) + revoked_all as f64 * 4.0;
    let hwm = vm_hwm_kb().map(|k| k as f64 * 1024.0);
    let nd = a.days as f64;
    let epochs_per_day = epochs_total as f64 / nd;
    let ev_mean = event_total.checked_div(epochs_total).unwrap_or(0);
    let mode = if merkle_epochs == 0 {
        "hash list"
    } else if merkle_epochs == epochs_total {
        "Merkle"
    } else {
        "mixed"
    };
    let wall_s = wall.elapsed().as_secs_f64();
    let (p50, p95, pmax) = (run_hist.quantile(0.5), run_hist.quantile(0.95), run_hist.max);
    let migrated_total: u64 = migrated_by_gen.values().sum();
    let mig_bytes = migrated_total as f64 * (bundle_len + NIP44_OVERHEAD) as f64;
    let history = match a.history {
        History::Open => "open",
        History::FromJoin => "from-join",
    };
    let scope_s = if a.open_scope { "open" } else { "closed" };

    // Offline catch-up from the epoch log.
    let end_min = a.days * DAY_MIN;
    let get_n = |e: &EpochLog| if e.witness > 0.0 { 3.0 } else { 2.0 };
    let (cur_gets, cur_bytes) = match elog.last() {
        Some(l) => (get_n(l), l.event as f64 + l.shard + l.witness),
        None => (0.0, 0.0),
    };
    let mut offline = String::new();
    offline.push_str(
        "| offline | epochs in window | current key GETs | current key bytes | history GETs | history bytes |\n|---|---|---|---|---|---|\n",
    );
    for dd in [1u64, 7, 30, 90] {
        // (epochs, events bytes, shard+witness bytes, from-join GETs)
        let (n, ev_b, sw_b, fj_g, star) = if dd <= a.days {
            let from = end_min - dd * DAY_MIN;
            let w: Vec<&EpochLog> = elog.iter().filter(|e| e.at_min >= from).collect();
            (
                w.len() as f64,
                w.iter().map(|e| e.event as f64).sum::<f64>(),
                w.iter().map(|e| e.shard + e.witness).sum::<f64>(),
                w.iter().map(|e| get_n(e)).sum::<f64>(),
                "",
            )
        } else {
            let k = elog.len() as f64 / nd * dd as f64;
            let c = elog.len().max(1) as f64;
            let me = elog.iter().map(|e| e.event as f64).sum::<f64>() / c;
            let ms = elog.iter().map(|e| e.shard + e.witness).sum::<f64>() / c;
            let mg = elog.iter().map(get_n).sum::<f64>() / c;
            (k, k * me, k * ms, k * mg, "*")
        };
        // Open history: the chain is in the events, one event per missed
        // epoch. From-join: per missed epoch the event, the witness and the
        // shard.
        let (hg, hb) = match a.history {
            History::Open => (n, ev_b),
            History::FromJoin => (fj_g, ev_b + sw_b),
        };
        offline.push_str(&format!(
            "| {dd} d | {n:.0}{star} | {cur_gets:.0} | {} | {hg:.0} | {} |\n",
            fmt_b(cur_bytes),
            fmt_b(hb),
        ));
    }

    let mut gen_txt = String::new();
    if gens.is_empty() {
        gen_txt.push_str("none\n");
    }
    let mut prev_day: Option<u64> = None;
    for g in &gens {
        let gap = prev_day.map_or(String::new(), |p| format!(", {} d after the previous", g.day - p));
        let mg = migrated_by_gen.get(&g.gen).copied().unwrap_or(0);
        gen_txt.push_str(&format!(
            "- day {}: generation {} ({}){gap}; migrated {} members, traffic {}\n",
            g.day,
            g.gen,
            g.kind,
            mg,
            fmt_b(mg as f64 * (bundle_len + NIP44_OVERHEAD) as f64)
        ));
        prev_day = Some(g.day);
    }
    let grace_d = a.grace_days.max(1) as f64;

    let header = "| run | scheme | members | depth | shard width | history | scope | days | bundle | epochs/day | shard p50 | shard p95 | shard max | cover mean / max | upload / epoch | upload / day | event (mode) | client GETs/day | client bytes/day | generations | missed peak (keys) | RSS peak | est. state | wall | slowest epoch | migration traffic | keeper total / 30 d |";
    let sep = "|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|";
    let row = format!(
        "| {} | {} | {} | {} | {} | {} | {} | {} | {} B | {:.0} | {} | {} | {} | {:.0} / {} | {} | {} | {} ({}) | {:.1} | {} | {} | {} | {} | {} | {:.1} s | {:.0} ms | {} | {} |",
        a.label,
        a.scheme,
        a.members,
        a.depth,
        a.lps,
        history,
        scope_s,
        a.days,
        bundle_len,
        epochs_per_day,
        fmt_b(f64::from(p50)),
        fmt_b(f64::from(p95)),
        fmt_b(f64::from(pmax)),
        cover_sum_run as f64 / epochs_total.max(1) as f64,
        cover_max_run,
        fmt_b(upload_total as f64 / epochs_total.max(1) as f64),
        fmt_b(upload_total as f64 / nd),
        fmt_b(ev_mean as f64),
        mode,
        client_gets_run / nd,
        fmt_b(client_bytes_run / nd),
        gens.len(),
        missed_peak,
        hwm.map_or("n/a".into(), fmt_b),
        fmt_b(est_mem),
        wall_s,
        slowest_ms,
        fmt_b(mig_bytes),
        fmt_b((upload_total + event_total) as f64 / nd * 30.0 + mig_bytes / nd * 30.0),
    );
    let md = format!(
        "## Run {label}\n\nseed {seed}, scheme {scheme}, {members} members at start, depth {depth}, shard width {lps}, history {history}, scope {scope_s}, {days} days; joins {jp} %/day, leaves {lp} %/day, bans {bpd}/day + a wave of {spam} within one hour, online {on} %, grace {gd} d, revoked ratio {rr}, ban window {bw} s, leave window {lw} s, join window {jw} s, least interval {mi} s.\n\n### Row\n{header}\n{sep}\n{row}\n\n### Epoch reasons (an epoch may count under several)\n\nepochs {et}: first {r0}, ban batch {r1}, closed leave {r2}, join activation (from-join) {r3}, lazy open leave carried {r4}. Event mode: {mode}; largest event {evmax} B; epoch build total {bt:.1} s.\n\n### Offline client ({history} history)\n\nCurrent key = the newest event + the witness (the index blob, or the proof in Merkle mode) + the client's shard: three GETs. History = under open the chain is inside the events: one event per missed epoch; under from-join per missed epoch the event, the witness and the shard. Keeper total / 30 d = blobs + events + migration traffic, scaled to 30 days.\n\n{offline}\n### Generations\n\n{gen_txt}\nmigrations: {gn}; members migrated {mm}; migration traffic {mt} in total = {mpd}/day over a grace of {gd} d (bundle {bl} B + {ov} B NIP-44); longest `missed` list of a real bundle {mk} keys; unmigrated peak {up}; joins refused (generation full) {fr}.\n\n### Time and memory\n\nsetup {setup:.1} s, wall {wall_s:.1} s, slowest epoch {slow:.0} ms, mean epoch {mean:.0} ms; VmHWM {hwm}; estimate members x (32 + {mr} B) + revoked x 4 = {est}.\n",
        label = a.label,
        seed = a.seed,
        scheme = a.scheme,
        members = a.members,
        depth = a.depth,
        lps = a.lps,
        days = a.days,
        jp = a.joins_pct,
        lp = a.leaves_pct,
        bpd = a.bans_per_day,
        spam = a.spam_wave,
        on = a.online_pct,
        gd = a.grace_days,
        rr = a.revoked_ratio,
        jw = a.join_window,
        mi = a.min_interval,
        bw = a.ban_window,
        lw = a.leave_window,
        et = epochs_total,
        r0 = reasons.first,
        r1 = reasons.ban,
        r2 = reasons.leave,
        r3 = reasons.join,
        r4 = reasons.lazy,
        evmax = event_max,
        bt = build_total_ms / 1000.0,
        gn = gen_ops,
        mm = migrated_total,
        mt = fmt_b(mig_bytes),
        mpd = fmt_b(mig_bytes / grace_d),
        bl = bundle_len,
        ov = NIP44_OVERHEAD,
        mk = missed_keys_seen,
        up = unmigrated_peak,
        fr = full_rejects,
        setup = setup_s,
        slow = slowest_ms,
        mean = build_total_ms / epochs_total.max(1) as f64,
        hwm = hwm.map_or("n/a".into(), fmt_b),
        mr = std::mem::size_of::<MemberRec>(),
        est = fmt_b(est_mem),
    );

    let mut csv = String::from(
        "day,epochs,upload_bytes,shard_p50,shard_p95,shard_max,event_bytes,members,revoked,next_leaf,generation,cover_mean,cover_max,joins,leaves,bans\n",
    );
    for r in &days {
        csv.push_str(&format!(
            "{},{},{},{},{},{},{},{},{},{},{},{:.0},{},{},{},{}\n",
            r.day, r.epochs, r.upload, r.p50, r.p95, r.max, r.event_mean, r.members, r.revoked, r.next_leaf,
            r.generation, r.cover_mean, r.cover_max, r.joins, r.leaves, r.bans
        ));
    }
    for (path, text) in [(&a.csv, &csv), (&a.md, &md)] {
        if let Some(p) = path {
            if let Some(dir) = std::path::Path::new(p).parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Err(e) = std::fs::write(p, text) {
                eprintln!("cannot write {p}: {e}");
                std::process::exit(1);
            }
        }
    }
    if !a.quiet {
        println!("{md}");
    }
}
