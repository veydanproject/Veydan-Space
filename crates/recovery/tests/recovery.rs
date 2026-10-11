// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The crate end to end: one owner sets recovery up, loses everything and
//! comes back by the shares and by the words; the golden set of
//! `golden/recovery-v1.json`, which changes only with a format version;
//! and malformed input of every parser, which may fail but never panic.

use secp256k1::{Keypair, Secp256k1, XOnlyPublicKey};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use veydan_recovery::devicelink::{self, LinkCode, LinkPayload};
use veydan_recovery::escrow::{self, Escrow, Header, LockKey, RecoveryParams, SyncKeys};
use veydan_recovery::pow::{self, Challenge, Purpose};
use veydan_recovery::rk::{self, Label, RecoveryKey};
use veydan_recovery::shamir::{self, Share, ShareError};
use veydan_recovery::{KdfParams, ShamirError};
use vsss_rs::{Gf256, IdentifierGf256};
use zeroize::Zeroizing;

const GOLDEN: &str = include_str!("../golden/recovery-v1.json");

const RK: [u8; 16] = [
    0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff,
];
const NSEC: [u8; 32] = [0x42; 32];
const GENERATION: u32 = 3;
const K: u8 = 3;
const N: u8 = 5;
const SALT: [u8; 16] = [0x11; 16];
const NONCE: [u8; 24] = [0x22; 24];
const T_PUB: [u8; 32] = [0xab; 32];
const LINK_NONCE: [u8; 24] = [0x33; 24];
const LINK_CODE: &str = "abandon ability able about above absent absorb abstract";
const SERVER_KEY: &[u8] = b"golden-server-key";
const POW_D: u8 = 3;
const FAST: KdfParams = KdfParams {
    memory_kib: 64,
    iterations: 1,
    parallelism: 1,
};

fn rk() -> RecoveryKey {
    RecoveryKey::from_bytes(RK).unwrap()
}

fn npub() -> [u8; 32] {
    rk::npub_of(&NSEC).unwrap()
}

/// Coefficients from a counter-based SHA-256 stream: the golden shares
/// are reproducible.
fn golden_fill() -> impl FnMut(&mut [u8]) -> Result<(), ShamirError> {
    let mut counter = 0u32;
    move |buf: &mut [u8]| {
        for chunk in buf.chunks_mut(32) {
            let mut h = Sha256::new();
            h.update(b"veydan-recovery golden coefficients");
            h.update(counter.to_be_bytes());
            counter += 1;
            let block = h.finalize();
            chunk.copy_from_slice(&block[..chunk.len()]);
        }
        Ok(())
    }
}

fn golden_escrow() -> Escrow {
    Escrow {
        npub: npub(),
        created_at: 1_791_000_000,
        generation: GENERATION,
        nsec: NSEC,
        sync: SyncKeys::cloud("3f2a9c1e-5b7d-4e8a-9c0b-1d2e3f4a5b6c", [0x77; 32]),
        lock: Some(LockKey {
            vault_id: "b1c2d3e4-f5a6-4b7c-8d9e-0f1a2b3c4d5e".to_owned(),
            vault_key: [0x99; 32],
        }),
        recovery: RecoveryParams {
            k: K,
            n: N,
            generation: GENERATION,
        },
    }
}

fn golden_payload() -> LinkPayload {
    LinkPayload {
        npub: npub(),
        nsec: NSEC,
        created_at: 1_791_000_000,
        sync: Some(SyncKeys::cloud(
            "3f2a9c1e-5b7d-4e8a-9c0b-1d2e3f4a5b6c",
            [0x77; 32],
        )),
    }
}

fn golden_challenge() -> [u8; pow::CHALLENGE_LEN] {
    Challenge {
        ts: 1_791_000_000,
        d: POW_D,
        purpose: Purpose::RecoveryRequest,
        r: [0, 1, 2, 3, 4, 5, 6, 7],
    }
    .encode(SERVER_KEY)
}

/// Scenario 4 and then scenario 3, from the setup to the account.
#[test]
fn owner_sets_up_loses_everything_and_comes_back() {
    // Setup on the owner's device: a key, the escrow, the shares.
    let key = RecoveryKey::generate().unwrap();
    let words = key.words();
    let owner = npub();
    let mut record = golden_escrow();
    record.generation = 1;
    record.recovery.generation = 1;
    let sealed = escrow::seal(&key, &record).unwrap();
    let address = key.escrow_auth().unwrap().escrow_address();
    let shares = shamir::split(&key, &owner, 1, 2, 3).unwrap();
    // The shares travel as bytes to three trusted people.
    let stored: Vec<Zeroizing<[u8; 88]>> = shares.iter().map(Share::to_bytes).collect();

    // Scenario 4: a new device with nothing; two of the three answer, one
    // of them with a share someone changed on the way. The answers are
    // kept in the order the seals came in: the position names the sender.
    let mut tampered = stored[2].clone();
    tampered[45] ^= 0x40;
    let answers = [
        Share::from_bytes(&stored[0][..]).unwrap(),
        Share::from_bytes(&tampered[..]).unwrap(),
        Share::from_bytes(&stored[1][..]).unwrap(),
    ];
    let got = shamir::combine(&answers).unwrap();
    assert_eq!(got.rk, key);
    assert_eq!(got.forged, vec![1]);
    assert_eq!((got.k, got.n, got.generation, got.npub), (2, 3, 1, owner));

    // The recovered key opens the escrow at the address it derives.
    assert_eq!(got.rk.escrow_auth().unwrap().escrow_address(), address);
    let opened = escrow::open(&got.rk, &sealed).unwrap();
    assert_eq!(opened, record);
    assert_eq!(opened.nsec, NSEC);
    assert_eq!(rk::npub_of(&opened.nsec).unwrap(), owner);
    // And the vault passphrase follows from the nsec.
    assert_eq!(
        rk::sync_vault_passphrase(&opened.nsec),
        rk::sync_vault_passphrase(&NSEC)
    );

    // Scenario 3: the words alone.
    let typed = RecoveryKey::from_words(&words).unwrap();
    assert_eq!(typed, key);
    assert_eq!(escrow::open(&typed, &sealed).unwrap(), record);
    // The lock opens by the wrap v2 input of the same key.
    assert_eq!(typed.lock_wrap_input(), key.lock_wrap_input());

    // A re-issue: the old shares do not mix with the new generation, and
    // the old escrow sits at another address.
    let next = RecoveryKey::generate().unwrap();
    let next_shares = shamir::split(&next, &owner, 2, 2, 3).unwrap();
    let mixed = vec![answers[0].clone(), next_shares[0].clone()];
    assert_eq!(
        shamir::combine(&mixed).unwrap_err(),
        ShamirError::MixedGenerations
    );
    assert_ne!(next.escrow_auth().unwrap().escrow_address(), address);
    assert_eq!(
        escrow::open(&next, &sealed).unwrap_err(),
        escrow::EscrowError::Decrypt
    );
}

/// Scenario 2: the new device shows a code and T, the old one wraps the
/// nsec for it.
#[test]
fn device_link_end_to_end() {
    // B: a temporary key T and a code.
    let secp = Secp256k1::new();
    let t_secret = secp256k1::SecretKey::from_byte_array(&[0x5a; 32]).unwrap();
    let t_pub = t_secret.x_only_public_key(&secp).0.serialize();
    let code = LinkCode::generate().unwrap();
    let uri = devicelink::link_uri(&t_pub, &code);
    let fingerprint_b = devicelink::fingerprint(&t_pub);

    // A: scans the QR, shows the same fingerprint, wraps the nsec.
    let (t_pub_a, code_a) = devicelink::parse_link_uri(&uri).unwrap();
    assert_eq!(t_pub_a, t_pub);
    assert_eq!(code_a, code);
    assert_eq!(devicelink::fingerprint(&t_pub_a), fingerprint_b);
    let payload = golden_payload();
    let blob = devicelink::seal(&code_a, &t_pub_a, &payload).unwrap();

    // B: opens with its code and T.
    let got = devicelink::open(&code, &t_pub, &blob).unwrap();
    assert_eq!(got, payload);
    // The passphrase of the vault follows from the nsec when no sync keys
    // came along.
    assert_eq!(
        rk::sync_vault_passphrase(&got.nsec),
        rk::sync_vault_passphrase(&NSEC)
    );

    // A T someone substituted: the code was computed from the real T.pub,
    // the blob does not open.
    let other = secp256k1::SecretKey::from_byte_array(&[0x5b; 32]).unwrap();
    let other_pub = other.x_only_public_key(&secp).0.serialize();
    assert_ne!(devicelink::fingerprint(&other_pub), fingerprint_b);
    assert_eq!(
        devicelink::open(&code, &other_pub, &blob).unwrap_err(),
        devicelink::DeviceLinkError::Decrypt
    );
    // No camera: the d tag of the kind 30078 event is the digest of the code.
    assert_eq!(
        code.d_tag(),
        hex::encode(Sha256::digest(code.as_str().as_bytes()))
    );
}

/// The address the client derives from `auth` equals what vstore derives
/// from the x-only signer of the NIP-98 event.
#[test]
fn escrow_address_matches_the_nip98_signer() {
    let auth = rk().escrow_auth().unwrap();
    let secp = Secp256k1::new();
    let keypair = Keypair::from_secret_key(&secp, &auth.secret_key().unwrap());
    let msg = Sha256::digest(b"GET https://store.example.com/vault/v1/escrow/...");
    let sig = secp.sign_schnorr_no_aux_rand(&msg, &keypair);
    // vstore: the pubkey of the event, 32 bytes x-only.
    let signer = keypair.x_only_public_key().0.serialize();
    let verifier = XOnlyPublicKey::from_byte_array(&signer).unwrap();
    secp.verify_schnorr(&sig, &msg, &verifier).unwrap();
    assert_eq!(signer, auth.public_x_only());
    assert_eq!(rk::escrow_address(&signer), auth.escrow_address());
    assert_eq!(hex::encode(Sha256::digest(signer)), auth.escrow_address());
}

/// The client solves what the server issued, the server checks it; a
/// lowered challenge and a wrong purpose fail at the signature.
#[test]
fn pow_client_and_server() {
    let issued = Challenge {
        ts: 1_791_000_000,
        d: 2,
        purpose: Purpose::Activate,
        r: [7; 8],
    };
    let wire = pow::to_base64url(&issued.encode(SERVER_KEY));
    // Client.
    let bytes = pow::from_base64url(&wire).unwrap();
    let seen = Challenge::peek(&bytes).unwrap();
    assert_eq!(seen, issued);
    let nonce = pow::solve(&bytes, seen.d).unwrap();
    // Server, after a key rotation: the signature, the freshness, the
    // work, then the HMAC part into its memory against a replay.
    let back = pow::from_base64url(&wire).unwrap();
    let parsed = Challenge::decode(&back, &[b"new-key", SERVER_KEY]).unwrap();
    assert_eq!(parsed.purpose, Purpose::Activate);
    assert!(parsed.is_fresh(parsed.ts + 30));
    assert!(!parsed.is_fresh(parsed.ts + 121));
    pow::verify(&back, nonce, parsed.d).unwrap();
    let mut recent: Vec<[u8; 16]> = Vec::new();
    let key = pow::replay_key(&back).unwrap();
    assert!(!recent.contains(&key));
    recent.push(key);
    assert!(recent.contains(&pow::replay_key(&back).unwrap()));
    assert_eq!(
        Challenge::decode(&back, &[b"new-key"]).unwrap_err(),
        pow::PowError::Invalid
    );
}

#[derive(Deserialize)]
struct Golden {
    version: u8,
    rk_hex: String,
    rk_words: String,
    nsec_hex: String,
    npub_hex: String,
    generation: u32,
    k: u8,
    n: u8,
    labels: Labels,
    auth_pub_hex: String,
    escrow_address: String,
    lock_wrap_input: String,
    sync_vault_passphrase: String,
    shares_hex: Vec<String>,
    escrow: GoldenEscrow,
    device_link: GoldenLink,
    pow: GoldenPow,
    check_code: GoldenCheck,
}

#[derive(Deserialize)]
struct Labels {
    #[serde(rename = "veydan-recovery/escrow-auth/1")]
    escrow_auth: String,
    #[serde(rename = "veydan-recovery/escrow-enc/1")]
    escrow_enc: String,
    #[serde(rename = "veydan-recovery/share-commit/1")]
    share_commit: String,
    #[serde(rename = "veydan-recovery/share-mac/1")]
    share_mac: String,
    #[serde(rename = "veydan-recovery/lock-wrap/1")]
    lock_wrap: String,
}

#[derive(Deserialize)]
struct GoldenEscrow {
    salt_hex: String,
    nonce_hex: String,
    plaintext_cbor_hex: String,
    sealed_hex: String,
}

#[derive(Deserialize)]
struct GoldenLink {
    t_pub_hex: String,
    code: String,
    fingerprint: Vec<String>,
    d_tag: String,
    nonce_hex: String,
    blob_hex: String,
}

#[derive(Deserialize)]
struct GoldenPow {
    server_key: String,
    challenge_hex: String,
    challenge_base64url: String,
    d: u8,
    nonce: u64,
    h_hex: String,
}

#[derive(Deserialize)]
struct GoldenCheck {
    temp_pub_hex: String,
    code: String,
}

fn golden() -> Golden {
    serde_json::from_str(GOLDEN).unwrap()
}

#[test]
fn golden_key_labels_and_words() {
    let g = golden();
    assert_eq!(g.version, 1);
    assert_eq!(hex::decode(&g.rk_hex).unwrap(), RK);
    let key = rk();
    assert_eq!(key.words().as_str(), g.rk_words);
    assert_eq!(RecoveryKey::from_words(&g.rk_words).unwrap(), key);
    assert_eq!(hex::decode(&g.nsec_hex).unwrap(), NSEC);
    assert_eq!(hex::decode(&g.npub_hex).unwrap(), npub());
    assert_eq!((g.generation, g.k, g.n), (GENERATION, K, N));
    assert_eq!(
        hex::encode(*key.derive(Label::EscrowAuth)),
        g.labels.escrow_auth
    );
    assert_eq!(
        hex::encode(*key.derive(Label::EscrowEnc)),
        g.labels.escrow_enc
    );
    assert_eq!(
        hex::encode(*key.derive(Label::ShareCommit)),
        g.labels.share_commit
    );
    assert_eq!(
        hex::encode(*key.derive(Label::ShareMac)),
        g.labels.share_mac
    );
    assert_eq!(
        hex::encode(*key.derive(Label::LockWrap)),
        g.labels.lock_wrap
    );
    let auth = key.escrow_auth().unwrap();
    assert_eq!(hex::encode(auth.public_x_only()), g.auth_pub_hex);
    assert_eq!(auth.escrow_address(), g.escrow_address);
    assert_eq!(key.lock_wrap_input().as_str(), g.lock_wrap_input);
    assert_eq!(
        rk::sync_vault_passphrase(&NSEC).as_str(),
        g.sync_vault_passphrase
    );
}

#[test]
fn golden_shares() {
    let g = golden();
    let shares: Vec<Share> = g
        .shares_hex
        .iter()
        .map(|h| Share::from_bytes(&hex::decode(h).unwrap()).unwrap())
        .collect();
    assert_eq!(shares.len(), N as usize);
    // Reproduced from the deterministic coefficients.
    let again = shamir::split_with(&rk(), &npub(), GENERATION, K, N, &mut golden_fill()).unwrap();
    assert_eq!(again, shares);
    // Any three give the key, two do not.
    let got = shamir::combine(&[shares[4].clone(), shares[1].clone(), shares[2].clone()]).unwrap();
    assert_eq!(got.rk, rk());
    assert!(got.forged.is_empty());
    assert_eq!(
        shamir::combine(&shares[..2]).unwrap_err(),
        ShamirError::NotEnoughShares { have: 2, need: 3 }
    );
    assert_eq!(hex::encode(shares[0].commit()), g.labels.share_commit[..32]);
}

#[test]
fn golden_escrow_opens() {
    let g = golden();
    let salt: [u8; 16] = hex::decode(&g.escrow.salt_hex).unwrap().try_into().unwrap();
    let nonce: [u8; 24] = hex::decode(&g.escrow.nonce_hex)
        .unwrap()
        .try_into()
        .unwrap();
    assert_eq!((salt, nonce), (SALT, NONCE));
    let record = golden_escrow();
    assert_eq!(
        hex::encode(record.to_cbor().as_slice()),
        g.escrow.plaintext_cbor_hex
    );
    let sealed = hex::decode(&g.escrow.sealed_hex).unwrap();
    let (header, _) = Header::parse(&sealed).unwrap();
    assert_eq!(header.kdf, KdfParams::production());
    assert_eq!(escrow::open(&rk(), &sealed).unwrap(), record);
    assert_eq!(
        escrow::seal_with(&rk(), &record, &KdfParams::production(), SALT, NONCE).unwrap(),
        sealed
    );
}

#[test]
fn golden_device_link_opens() {
    let g = golden();
    assert_eq!(hex::decode(&g.device_link.t_pub_hex).unwrap(), T_PUB);
    let code = LinkCode::parse(&g.device_link.code).unwrap();
    assert_eq!(code.as_str(), LINK_CODE);
    assert_eq!(
        devicelink::fingerprint(&T_PUB).to_vec(),
        g.device_link.fingerprint
    );
    assert_eq!(code.d_tag(), g.device_link.d_tag);
    let nonce: [u8; 24] = hex::decode(&g.device_link.nonce_hex)
        .unwrap()
        .try_into()
        .unwrap();
    assert_eq!(nonce, LINK_NONCE);
    let blob = hex::decode(&g.device_link.blob_hex).unwrap();
    assert_eq!(
        devicelink::open(&code, &T_PUB, &blob).unwrap(),
        golden_payload()
    );
    assert_eq!(
        devicelink::seal_with(
            &code,
            &T_PUB,
            &golden_payload(),
            &KdfParams::production(),
            LINK_NONCE
        )
        .unwrap(),
        blob
    );
}

#[test]
fn golden_pow_and_check_code() {
    let g = golden();
    assert_eq!(g.pow.server_key.as_bytes(), SERVER_KEY);
    let challenge = hex::decode(&g.pow.challenge_hex).unwrap();
    assert_eq!(challenge, golden_challenge());
    assert_eq!(pow::to_base64url(&challenge), g.pow.challenge_base64url);
    assert_eq!(g.pow.d, POW_D);
    Challenge::decode(&challenge, &[SERVER_KEY]).unwrap();
    assert_eq!(
        hex::encode(pow::hash(&challenge, g.pow.nonce).unwrap()),
        g.pow.h_hex
    );
    pow::verify(&challenge, g.pow.nonce, g.pow.d).unwrap();
    assert_eq!(pow::solve(&challenge, g.pow.d).unwrap(), g.pow.nonce);
    let temp_pub: [u8; 32] = hex::decode(&g.check_code.temp_pub_hex)
        .unwrap()
        .try_into()
        .unwrap();
    assert_eq!(temp_pub, [0xab; 32]);
    assert_eq!(pow::check_code(&temp_pub), g.check_code.code);
    assert_eq!(g.check_code.code, "K8PV-5RHZ-2M2C");
}

/// A `rand_core` source for vsss-rs over the xorshift below: the
/// cross-check wants an independent split, not independent entropy.
struct VsssRng(XorShift);

impl rand_core::TryRng for VsssRng {
    type Error = std::convert::Infallible;

    fn try_next_u32(&mut self) -> Result<u32, Self::Error> {
        Ok(self.0.next() as u32)
    }

    fn try_next_u64(&mut self) -> Result<u64, Self::Error> {
        Ok(self.0.next())
    }

    fn try_fill_bytes(&mut self, dst: &mut [u8]) -> Result<(), Self::Error> {
        self.0.fill(dst);
        Ok(())
    }
}

impl rand_core::TryCryptoRng for VsssRng {}

/// `index ‖ y` of a share: the form vsss-rs combines.
fn vsss_form(share: &Share) -> Vec<u8> {
    let b = share.to_bytes();
    let mut v = Vec::with_capacity(17);
    v.push(b[39]);
    v.extend_from_slice(&b[40..56]);
    v
}

/// cloud-recovery.md §13: our shares are combined by vsss-rs (an
/// independent Shamir over the same field, polynomial 0x11b) and theirs by
/// our interpolation, for every k and n of the contract.
#[test]
fn cross_check_with_vsss_rs() {
    // Ours -> theirs: every 3-subset of the golden shares.
    let g = golden();
    let golden_shares: Vec<Vec<u8>> = g
        .shares_hex
        .iter()
        .map(|h| vsss_form(&Share::from_bytes(&hex::decode(h).unwrap()).unwrap()))
        .collect();
    for a in 0..5 {
        for b in a + 1..5 {
            for c in b + 1..5 {
                let subset = vec![
                    golden_shares[c].clone(),
                    golden_shares[a].clone(),
                    golden_shares[b].clone(),
                ];
                assert_eq!(Gf256::combine_bytes(&subset).unwrap(), RK, "{a} {b} {c}");
            }
        }
    }
    // Ours -> theirs: fresh shares for every (k, n), the first k and the
    // last k.
    for n in 2..=7u8 {
        for k in 2..=n {
            let ours = shamir::split(&rk(), &npub(), 1, k, n).unwrap();
            let all: Vec<Vec<u8>> = ours.iter().map(vsss_form).collect();
            let first: Vec<Vec<u8>> = all[..k as usize].to_vec();
            let last: Vec<Vec<u8>> = all[(n - k) as usize..].to_vec();
            assert_eq!(Gf256::combine_bytes(&first).unwrap(), RK, "k={k} n={n}");
            assert_eq!(Gf256::combine_bytes(&last).unwrap(), RK, "k={k} n={n}");
            if k < n {
                // k - 1 of them do not give the key, whichever code combines.
                let short: Vec<Vec<u8>> = all[..(k - 1) as usize].to_vec();
                if k > 2 {
                    assert_ne!(Gf256::combine_bytes(&short).unwrap(), RK, "k={k} n={n}");
                }
            }
        }
    }
    // Theirs -> ours: vsss-rs splits with the indexes 1..=n, our
    // interpolation at 0 gives the key from any k of the shares.
    let mut rng = VsssRng(XorShift(0x2545_f491_4f6c_dd1d));
    for n in 2..=7u8 {
        for k in 2..=n {
            let ids = (1..=n).map(|i| IdentifierGf256(Gf256(i)));
            let theirs = Gf256::split_bytes_with_participant_ids_iter(
                k as usize, n as usize, RK, &mut rng, ids,
            )
            .unwrap();
            assert_eq!(theirs.len(), n as usize);
            for (i, s) in theirs.iter().enumerate() {
                assert_eq!(s.len(), 17);
                assert_eq!(s[0], i as u8 + 1);
            }
            for subset in [&theirs[..k as usize], &theirs[(n - k) as usize..]] {
                for (byte, expected) in RK.iter().enumerate() {
                    let points: Vec<(u8, u8)> =
                        subset.iter().map(|s| (s[0], s[byte + 1])).collect();
                    assert_eq!(
                        shamir::interpolate(&points, 0),
                        Some(*expected),
                        "k={k} n={n} byte {byte}"
                    );
                }
            }
            // Their shares agree with ours byte for byte: both are the one
            // polynomial through the same k points.
            let points: Vec<(u8, u8)> = theirs[..k as usize].iter().map(|s| (s[0], s[1])).collect();
            for s in &theirs[k as usize..] {
                assert_eq!(shamir::interpolate(&points, s[0]), Some(s[1]));
            }
        }
    }
}

/// A small PRNG for the inputs below; no crate of its own.
struct XorShift(u64);

impl XorShift {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn fill(&mut self, buf: &mut [u8]) {
        for b in buf {
            *b = self.next() as u8;
        }
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// Every parser and `combine` on random and mutated bytes: an error or a
/// value, never a panic. A stand-in for `cargo fuzz`, which this
/// workspace does not run: the seed comes from the clock, so every run
/// tries new inputs, and is printed with a failure; `VEYDAN_RECOVERY_SEED`
/// replays one.
#[test]
fn malformed_input_never_panics() {
    let seed = std::env::var("VEYDAN_RECOVERY_SEED")
        .ok()
        .and_then(|s| u64::from_str_radix(s.trim_start_matches("0x"), 16).ok())
        .unwrap_or_else(|| {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0);
            0x9e37_79b9_7f4a_7c15 ^ nanos
        });
    let seed = if seed == 0 {
        0x9e37_79b9_7f4a_7c15
    } else {
        seed
    };
    eprintln!("malformed_input_never_panics: VEYDAN_RECOVERY_SEED={seed:#x}");
    let mut rng = XorShift(seed);
    let shares = shamir::split(&rk(), &npub(), 1, 3, 5).unwrap();
    let sealed = escrow::seal_with(&rk(), &golden_escrow(), &FAST, SALT, NONCE).unwrap();
    let code = LinkCode::parse(LINK_CODE).unwrap();
    let blob = devicelink::seal_with(&code, &T_PUB, &golden_payload(), &FAST, LINK_NONCE).unwrap();
    let plain_escrow = golden_escrow().to_cbor();
    let plain_link = golden_payload().to_cbor();

    for round in 0..2000 {
        // Random bytes of a random length into every parser.
        let len = rng.below(200);
        let mut junk = vec![0u8; len];
        rng.fill(&mut junk);
        let _ = Share::from_bytes(&junk);
        let _ = Header::parse(&junk);
        let _ = Escrow::from_cbor(&junk);
        let _ = LinkPayload::from_cbor(&junk);
        let _ = Challenge::peek(&junk);
        let _ = Challenge::decode(&junk, &[SERVER_KEY]);
        let _ = pow::from_base64url(&String::from_utf8_lossy(&junk));
        let _ = RecoveryKey::from_words(&String::from_utf8_lossy(&junk));
        let _ = LinkCode::parse(&String::from_utf8_lossy(&junk));
        let _ = devicelink::parse_link_uri(&String::from_utf8_lossy(&junk));

        // A valid thing with a few bytes changed.
        let mut s = shares[round % 5].to_bytes();
        for _ in 0..1 + rng.below(3) {
            let at = rng.below(s.len());
            s[at] = rng.next() as u8;
        }
        let parsed = Share::from_bytes(&s[..]);
        let mut set: Vec<Share> = shares.iter().take(1 + rng.below(5)).cloned().collect();
        if let Ok(p) = parsed {
            set.push(p);
        }
        let _ = shamir::combine(&set);

        // The escrow and the blob past their parameter bytes, so a mutation
        // never asks for more memory than the fast parameters.
        let mut e = sealed.clone();
        let at = 13 + rng.below(e.len() - 13);
        e[at] ^= 1 << rng.below(8);
        let _ = escrow::open(&rk(), &e);
        let mut b = blob.clone();
        let at = rng.below(b.len());
        b[at] ^= 1 << rng.below(8);
        let _ = devicelink::open_with(&code, &T_PUB, &b, &FAST);

        // The plaintexts, mutated and truncated.
        let mut pe = plain_escrow.to_vec();
        let at = rng.below(pe.len());
        pe[at] = rng.next() as u8;
        let _ = Escrow::from_cbor(&pe);
        let _ = Escrow::from_cbor(&pe[..rng.below(pe.len())]);
        let mut pl = plain_link.to_vec();
        let at = rng.below(pl.len());
        pl[at] = rng.next() as u8;
        let _ = LinkPayload::from_cbor(&pl);
        let _ = LinkPayload::from_cbor(&pl[..rng.below(pl.len())]);
    }
    // The form checks are what they say.
    assert_eq!(Share::from_bytes(&[]), Err(ShareError::Length(0)));
}

/// Prints the golden set: run by hand when the format version changes,
/// and the output goes to `golden/recovery-v1.json`.
#[test]
#[ignore = "writes the golden set to stdout; run by hand on a format change"]
fn print_golden() {
    let key = rk();
    let shares = shamir::split_with(&key, &npub(), GENERATION, K, N, &mut golden_fill()).unwrap();
    let record = golden_escrow();
    let sealed = escrow::seal_with(&key, &record, &KdfParams::production(), SALT, NONCE).unwrap();
    let code = LinkCode::parse(LINK_CODE).unwrap();
    let blob = devicelink::seal_with(
        &code,
        &T_PUB,
        &golden_payload(),
        &KdfParams::production(),
        LINK_NONCE,
    )
    .unwrap();
    let challenge = golden_challenge();
    let nonce = pow::solve(&challenge, POW_D).unwrap();
    let auth = key.escrow_auth().unwrap();
    let labels: serde_json::Map<String, serde_json::Value> = Label::ALL
        .iter()
        .map(|l| {
            (
                String::from_utf8(l.info().to_vec()).unwrap(),
                serde_json::Value::String(hex::encode(*key.derive(*l))),
            )
        })
        .collect();
    let out = serde_json::json!({
        "version": 1,
        "note": "Golden set of crates/recovery, format v1. Fixed inputs; changes only with a format version. Regenerate: cargo test -p veydan-recovery --test recovery print_golden -- --ignored --nocapture",
        "rk_hex": hex::encode(RK),
        "rk_words": key.words().as_str(),
        "nsec_hex": hex::encode(NSEC),
        "npub_hex": hex::encode(npub()),
        "generation": GENERATION,
        "k": K,
        "n": N,
        "labels": labels,
        "auth_pub_hex": hex::encode(auth.public_x_only()),
        "escrow_address": auth.escrow_address(),
        "lock_wrap_input": key.lock_wrap_input().as_str(),
        "sync_vault_passphrase": rk::sync_vault_passphrase(&NSEC).as_str(),
        "shares_coefficients": "sha256(\"veydan-recovery golden coefficients\" || counter u32 BE), counter from 0, 32 bytes a block",
        "shares_hex": shares.iter().map(|s| hex::encode(s.to_bytes().as_slice())).collect::<Vec<_>>(),
        "escrow": {
            "kdf": "argon2id 65536 KiB, t 3, p 1",
            "salt_hex": hex::encode(SALT),
            "nonce_hex": hex::encode(NONCE),
            "plaintext_cbor_hex": hex::encode(record.to_cbor().as_slice()),
            "sealed_hex": hex::encode(&sealed),
        },
        "device_link": {
            "t_pub_hex": hex::encode(T_PUB),
            "code": LINK_CODE,
            "fingerprint": devicelink::fingerprint(&T_PUB),
            "d_tag": code.d_tag(),
            "nonce_hex": hex::encode(LINK_NONCE),
            "blob_hex": hex::encode(&blob),
        },
        "pow": {
            "server_key": String::from_utf8(SERVER_KEY.to_vec()).unwrap(),
            "challenge_hex": hex::encode(challenge),
            "challenge_base64url": pow::to_base64url(&challenge),
            "d": POW_D,
            "nonce": nonce,
            "h_hex": hex::encode(pow::hash(&challenge, nonce).unwrap()),
        },
        "check_code": {
            "temp_pub_hex": hex::encode([0xabu8; 32]),
            "sha256_hex": hex::encode(Sha256::digest([0xabu8; 32])),
            "code": pow::check_code(&[0xab; 32]),
        },
    });
    println!("{}", serde_json::to_string_pretty(&out).unwrap());
}
