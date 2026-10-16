// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// A report of a person to Veydan's moderators. Every place that offers it
// (a message, a member, a person's card, a request) calls `reportStore.open`
// with who and where; `ReportHost` shows the form once per page. The input
// of the command is built here, not in the form, so that each entry is
// checked by a test without a screen.

import { get } from 'svelte/store';
import { t } from '$lib/core/i18n';
import { toast } from '$lib/core/ui/toast.svelte';
import { messengerApi, type ReportContext, type ReportInput, type ReportReason, type ReportResult, type ReportSubReason } from '../api';

export const REPORT_REASONS: ReportReason[] = ['child_harm', 'spam', 'harassment', 'illegal', 'other'];
export const REPORT_SUBS: ReportSubReason[] = ['sexual', 'grooming', 'violence_illegal', 'suicide_selfharm', 'sect_recruitment'];
export const REPORT_COMMENT_MAX = 2000;

export interface ReportTarget {
  /** Whom the report is on; a person when absent. A group has no accused key here: the runtime picks its owner. */
  target?: 'person' | 'group';
  /** The hex key of the person reported; empty for a group. */
  accused: string;
  /** What the person is called here, for «Block {name}». */
  name: string;
  context: ReportContext;
  /** Opened from a message: that message ends the attached ones, and they are attached by default. */
  messageId?: string | null;
  /** A chat of two or a request: the report may block the person. */
  canBlock: boolean;
  /** Called with the result once the report is sent or saved (a request that was blocked is gone). */
  onsent?: (result: ReportResult) => void;
}

class ReportStore {
  target = $state<ReportTarget | null>(null);
  reason = $state<ReportReason | null>(null);
  sub = $state<ReportSubReason | null>(null);
  comment = $state('');
  attach = $state(false);
  block = $state(false);
  busy = $state(false);

  open(target: ReportTarget) {
    this.target = target;
    this.reason = null;
    this.sub = null;
    this.comment = '';
    // The messages are offered when the report starts from one; a person's card has none to point at.
    this.attach = !!target.messageId;
    this.block = target.canBlock;
    this.busy = false;
  }

  close() {
    if (this.busy) return;
    this.target = null;
  }

  get ready() {
    return !!this.target && !!this.reason && !this.busy;
  }

  /** What the command is given; null while no reason is chosen. */
  input(): ReportInput | null {
    const target = this.target;
    if (!target || !this.reason) return null;
    const comment = this.comment.trim().slice(0, REPORT_COMMENT_MAX);
    const group = target.target === 'group';
    return {
      ...(group ? { target: 'group' as const } : {}),
      accused: group ? '' : target.accused,
      context: target.context,
      reason: this.reason,
      // Only a harm to a child has kinds; one chosen before the reason changed must not travel.
      sub_reason: this.reason === 'child_harm' ? this.sub : null,
      comment: comment || null,
      attach: this.attach,
      message_id: this.attach ? target.messageId ?? null : null,
      block: !group && target.canBlock && this.block,
    };
  }

  async send() {
    const input = this.input();
    const target = this.target;
    if (!input || !target || this.busy) return;
    const tr = get(t);
    this.busy = true;
    try {
      const result = await messengerApi.report.send(input);
      this.busy = false;
      this.target = null;
      if (result.status === 'queued') toast.info(tr('msg_report_queued'));
      else if (input.attach && result.attached > 0) toast.success(tr('msg_report_sent_n', { n: String(result.attached) }));
      else toast.success(tr('msg_report_sent'));
      target.onsent?.(result);
    } catch (e) {
      this.busy = false;
      // The form stays open with what was typed: a refusal is not a reason to write it again.
      const text = String((e as { message?: unknown } | null)?.message ?? e);
      toast.error(text.includes('rate_limited') ? tr('msg_report_rate_limited')
        : text.includes('group_banned') ? tr('msg_err_group_banned')
        : text.includes('report_target') ? tr('msg_err_report_target')
        : tr('msg_report_failed'));
    }
  }
}

export const reportStore = new ReportStore();
