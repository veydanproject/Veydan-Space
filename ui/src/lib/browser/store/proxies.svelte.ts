// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { get } from 'svelte/store';
import { api } from '$lib/browser/api';
import { t } from '$lib/core/i18n';
import type { BulkImportResult, BulkProxyItem, Proxy, ProxyCheckResult } from '$lib/browser/types';

class ProxiesStore {
  list = $state<Proxy[]>([]);
  loading = $state(false);
  loaded = $state(false);
  private _promise: Promise<void> | null = null;

  async ensureLoaded() {
    if (this.loaded) return;
    if (this._promise) return this._promise;
    this._promise = this.refresh().finally(() => { this._promise = null; });
    return this._promise;
  }

  async refresh() {
    this.loading = true;
    try {
      this.list = await api.proxies.list();
      this.loaded = true;
    } finally {
      this.loading = false;
    }
  }

  byWorkspace(id: string): Proxy[] {
    return this.list.filter((p) => p.tags.includes(`workspace:${id}`));
  }

  private patch(id: string, fields: Partial<Proxy>) {
    this.list = this.list.map((p) => (p.id === id ? { ...p, ...fields } : p));
  }

  /** Insert a new proxy or replace the existing one with the same id. */
  upsert(proxy: Proxy) {
    const exists = this.list.some((p) => p.id === proxy.id);
    this.list = exists
      ? this.list.map((p) => (p.id === proxy.id ? proxy : p))
      : [proxy, ...this.list];
  }

  /** Bulk-create proxies; imported ones are merged into the cached list. */
  async bulkImport(items: BulkProxyItem[]): Promise<BulkImportResult> {
    const result = await api.proxies.bulkCreate(items);
    for (const proxy of result.imported) this.upsert(proxy);
    return result;
  }

  /**
   * Suffix for the delete confirmation naming what still points at this proxy.
   * Deleting detaches those rows, and a profile or SSH connection whose proxy is
   * gone refuses to connect rather than falling back to a direct connection — so
   * the confirmation has to say which ones stop working.
   * Advisory: returns '' when nothing is attached or the lookup fails.
   */
  async deleteWarning(id: string): Promise<string> {
    try {
      const usage = await api.proxies.usage(id);
      const tr = get(t);
      const parts: string[] = [];
      if (usage.profiles.length) {
        parts.push(`${tr('proxy_usage_profiles')}: ${usage.profiles.join(', ')}`);
      }
      if (usage.ssh_connections.length) {
        parts.push(`${tr('proxy_usage_ssh')}: ${usage.ssh_connections.join(', ')}`);
      }
      return parts.length ? ` ${tr('proxy_usage_warning')} ${parts.join('; ')}.` : '';
    } catch {
      return '';
    }
  }

  /** Delete a proxy and drop it from the cached list. */
  async remove(id: string): Promise<void> {
    await api.proxies.delete(id);
    this.list = this.list.filter((p) => p.id !== id);
  }

  /**
   * Run a connectivity check. On success (and when no new SSH host key needs
   * trusting) the cached proxy is marked active with the resolved IP/geo.
   * Returns the raw result so callers can handle the SSH fingerprint prompt.
   */
  async check(id: string): Promise<ProxyCheckResult> {
    const result = await api.proxies.check(id);
    if (!(result.ssh_fingerprint_is_new && result.ssh_fingerprint)) {
      this.patch(id, {
        status: 'active',
        last_ip: result.ip,
        country: this.list.find((p) => p.id === id)?.country || result.country,
        city: this.list.find((p) => p.id === id)?.city || result.city,
      });
    }
    return result;
  }

  /** Mark a proxy as failed after a check error. */
  markFailed(id: string) {
    this.patch(id, { status: 'failed' });
  }

  /** Pin an SSH host key (TOFU) and mark the proxy active. */
  async trustFingerprint(
    id: string,
    fingerprint: string,
    ip: string,
    country: string | null,
    city: string | null,
  ): Promise<void> {
    await api.proxies.trustFingerprint(id, fingerprint, ip, country, city);
    const current = this.list.find((p) => p.id === id);
    this.patch(id, {
      status: 'active',
      last_ip: ip,
      server_fingerprint: fingerprint,
      country: current?.country || country,
      city: current?.city || city,
    });
  }
}

export const proxiesStore = new ProxiesStore();
