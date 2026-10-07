// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Generated from Rust (messenger-runtime/src/bindings.rs). Do not edit:
// run `make msg-types` after changing the Rust types.

/** Where a transfer is: queued, preparing (a photo is made smaller), checking what an earlier attempt kept, uploading, publishing its message, downloading, assembling, verifying. */
export type TransferStage = "queued" | "preparing" | "checking" | "uploading" | "publishing" | "downloading" | "assembling" | "verifying";

/** The payload of the runtime event `transfer.progress` (`messenger://event`). */
export type Progress = { transfer_id: string, message_id: string | null, chat_id: string | null, 
/**
 * `up` | `down`
 */
direction: "up" | "down", 
/**
 * queued | running | waiting_retry | paused | done | failed | cancelled
 */
status: "queued" | "running" | "waiting_retry" | "paused" | "done" | "failed" | "cancelled", 
/**
 * Plaintext bytes: confirmed chunks and part of those in flight. A
 * run starts where the last one stopped until it knows better (it
 * goes past checking, or starts the file over); from then on it never
 * goes back, unless what it counted is no longer stored (a new key,
 * another server). A run that ends (paused, failed, cancelled) tells
 * what is stored, as its row does: the requests it dropped count for
 * nothing, so the next run starts from that very number.
 */
done_bytes: number, total_bytes: number, 
/**
 * Always an `err.*` code.
 */
failure_reason: string | null, local_path: string | null, stage: TransferStage, 
/**
 * Chunks finished in the current stage (checking and assembling count
 * from 0); never goes back within a stage.
 */
chunks_done: number, chunks_total: number, chunk_size: number, 
/**
 * Bytes per second really moved (not chunks found on the server or
 * on disk), averaged over about three seconds and first told after
 * two; it fades while nothing moves, and is 0 after ten seconds of it.
 * Always 0 in a stage that moves nothing over the network (queued,
 * preparing, publishing, assembling, verifying).
 */
rate_bps: number, 
/**
 * Seconds left; none when nothing moved for three seconds, or less
 * than five remain.
 */
eta_secs: number | null, 
/**
 * When the next automatic attempt starts (unix ms), while waiting for it.
 */
retry_at_ms: number | null, 
/**
 * Automatic retries made in this run.
 */
attempt: number, file_name: string, mime: string, };

/** A transfer as `messenger_media_transfers` and `messenger_media_transfer` read it. */
export type TransferView = { id: string, direction: "up" | "down", message_id: string | null, chat_id: string | null, file_name: string, mime: string, size: number, status: "queued" | "running" | "waiting_retry" | "paused" | "done" | "failed" | "cancelled", done_bytes: number, 
/**
 * Uploads: runs started. Downloads: runs that ended failed, -1 once
 * cancelled.
 */
attempts: number, 
/**
 * Always an `err.*` code; `err.interrupted` for a transfer paused
 * because its app closed, which starts again by itself.
 */
failure_reason: string | null, local_path: string | null, stage: TransferStage, chunks_done: number, chunks_total: number, chunk_size: number, 
/**
 * Known while the transfer runs, here or in another process with this
 * data folder: speed, time left and when the next automatic attempt
 * starts.
 */
rate_bps: number, eta_secs: number | null, retry_at_ms: number | null, };
