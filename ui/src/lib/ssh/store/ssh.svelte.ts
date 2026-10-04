// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { listen } from '@tauri-apps/api/event';
import { api } from '$lib/ssh/api';
import type { SshConnection, SshSessionInfo } from '$lib/ssh/types';

// Terminal data callbacks keyed by session_id
type DataCallback = (data: Uint8Array) => void;

class SshStore {
  connections = $state<SshConnection[]>([]);
  sessions = $state<SshSessionInfo[]>([]);

  panelOpen = $state(false);

  // Active terminal overlay — session id currently shown in SSHSessionBar
  activeTerminalId = $state<string | null>(null);

  private dataCallbacks = new Map<string, DataCallback>();
  // Buffer data received before terminal opens
  private dataBuffer = new Map<string, Uint8Array[]>();
  private unlisten: (() => void)[] = [];
  private loaded = false;

  openPanel() {
    this.panelOpen = true;
  }

  closePanel() {
    this.panelOpen = false;
  }

  registerDataCallback(sessionId: string, cb: DataCallback) {
    this.dataCallbacks.set(sessionId, cb);
    // Flush buffered data — schedule via microtask so xterm is ready
    const buf = this.dataBuffer.get(sessionId);
    if (buf && buf.length > 0) {
      this.dataBuffer.delete(sessionId);
      Promise.resolve().then(() => {
        for (const chunk of buf) cb(chunk);
      });
    }
  }

  unregisterDataCallback(sessionId: string) {
    this.dataCallbacks.delete(sessionId);
  }

  async ensureLoaded() {
    if (this.loaded) return;
    this.loaded = true;
    await this.loadConnections();
    await this.loadSessions();
    this.startListeners();
  }

  async loadConnections() {
    try {
      this.connections = await api.ssh.connectionList();
    } catch {}
  }

  async loadSessions() {
    try {
      this.sessions = await api.ssh.sessionList();
    } catch {}
  }

  private startListeners() {
    const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
    if (!isTauri) return;

    listen<{ session_id: string; connection_id: string; status: string; error?: string }>(
      'ssh://status-changed',
      (e) => {
        const { session_id, connection_id, status, error } = e.payload;
        const idx = this.sessions.findIndex((s) => s.session_id === session_id);
        if (idx >= 0) {
          this.sessions[idx] = { ...this.sessions[idx], status: status as SshSessionInfo['status'], error: error ?? null };
        }
      }
    ).then((fn) => this.unlisten.push(fn));

    listen<{ session_id: string; data_base64: string }>(
      'ssh://data',
      (e) => {
        const { session_id, data_base64 } = e.payload;
        const binary = atob(data_base64);
        const bytes = new Uint8Array(binary.length);
        for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);

        const cb = this.dataCallbacks.get(session_id);
        if (cb) {
          cb(bytes);
        } else {
          // Buffer until terminal registers its callback
          const buf = this.dataBuffer.get(session_id) ?? [];
          buf.push(bytes);
          this.dataBuffer.set(session_id, buf);
        }
      }
    ).then((fn) => this.unlisten.push(fn));
  }

  async connect(connectionId: string): Promise<string> {
    const sessionId = await api.ssh.connect(connectionId);

    let conn = this.connections.find((c) => c.id === connectionId);
    // If not in store yet (e.g. called from profile tab with its own local list),
    // fetch it so the session info is complete and the terminal can render.
    if (!conn) {
      try {
        conn = await api.ssh.connectionGet(connectionId);
        this.connections = [...this.connections, conn];
      } catch {}
    }

    const info: SshSessionInfo = {
      session_id: sessionId,
      connection_id: connectionId,
      connection_name: conn?.name ?? connectionId,
      host: conn?.host ?? '',
      port: conn?.port ?? 22,
      status: 'connecting',
      error: null,
      connected_at: null,
    };
    this.sessions = [...this.sessions, info];

    // Auto-open terminal for the new session
    this.activeTerminalId = sessionId;

    return sessionId;
  }

  async disconnect(sessionId: string) {
    await api.ssh.disconnect(sessionId);
  }

  async removeSession(sessionId: string) {
    await api.ssh.sessionRemove(sessionId);
    this.sessions = this.sessions.filter((s) => s.session_id !== sessionId);
    this.dataCallbacks.delete(sessionId);
    this.dataBuffer.delete(sessionId);
    if (this.activeTerminalId === sessionId) {
      this.activeTerminalId = null;
    }
  }

  sessionById(sessionId: string): SshSessionInfo | undefined {
    return this.sessions.find((s) => s.session_id === sessionId);
  }

  // Sessions for a specific connection (may be multiple)
  sessionsForConnection(connectionId: string): SshSessionInfo[] {
    return this.sessions.filter((s) => s.connection_id === connectionId);
  }

  activeSession(connectionId: string): SshSessionInfo | undefined {
    return this.sessions.find(
      (s) => s.connection_id === connectionId && s.status === 'connected'
    );
  }

  destroy() {
    this.unlisten.forEach((fn) => fn());
    this.unlisten = [];
  }
}

export const sshStore = new SshStore();
