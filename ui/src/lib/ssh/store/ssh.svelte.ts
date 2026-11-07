// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { listen } from '@tauri-apps/api/event';
import { api } from '$lib/ssh/api';
import type { SshConnection, SshSessionInfo } from '$lib/ssh/types';

// Terminal data callbacks keyed by session_id
type DataCallback = (data: Uint8Array) => void;

/** A question of the login (the server's key on first connect, a keyboard-interactive prompt). */
export interface KeyboardPromptPayload {
  session_id: string;
  name: string;
  instructions: string;
  prompts: { prompt: string; echo: boolean }[];
  /** First-connection host key awaiting the user's trust decision. */
  host_key?: string | null;
}

type StatusPayload = { session_id: string; connection_id: string; status: string; error?: string };

class SshStore {
  connections = $state<SshConnection[]>([]);
  sessions = $state<SshSessionInfo[]>([]);

  panelOpen = $state(false);

  // Active terminal overlay — session id currently shown in SSHSessionBar
  activeTerminalId = $state<string | null>(null);

  private dataCallbacks = new Map<string, DataCallback>();
  // Buffer data received before terminal opens
  private dataBuffer = new Map<string, Uint8Array[]>();
  /** The login question each session waits on, until its terminal takes it. */
  prompts = $state<Record<string, KeyboardPromptPayload>>({});
  /** The last status of a session `connect` has not listed yet: on localhost the login ends before it returns. */
  private earlyStatus = new Map<string, StatusPayload>();
  /** Connections with a `connect` under way, and how many: only their sessions' statuses are held. */
  private pendingConnects = new Map<string, number>();
  private unlisten: (() => void)[] = [];
  private listening: Promise<void> | null = null;
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
    await this.startListeners();
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

  /** Once; `connect` waits for it, so nothing the backend says about a new session goes past. */
  private startListeners(): Promise<void> {
    const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
    if (!isTauri) return Promise.resolve();
    this.listening ??= Promise.all([
      listen<StatusPayload>('ssh://status-changed', (e) => {
        const { session_id, status, error } = e.payload;
        // The login is over: a question it asked has no one to answer it.
        if (status === 'error' || status === 'disconnected') delete this.prompts[session_id];
        const idx = this.sessions.findIndex((s) => s.session_id === session_id);
        if (idx >= 0) {
          this.sessions[idx] = { ...this.sessions[idx], status: status as SshSessionInfo['status'], error: error ?? null };
        } else if (this.pendingConnects.has(e.payload.connection_id)) {
          this.earlyStatus.set(session_id, e.payload);
        }
      }),
      listen<{ session_id: string; data_base64: string }>('ssh://data', (e) => {
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
      }),
      // Held here, not in the terminal: the question comes before the terminal is drawn.
      // SFTP asks under "sftp:{connection_id}": the files store answers those.
      listen<KeyboardPromptPayload>('ssh://keyboard-prompt', (e) => {
        if (e.payload.session_id.startsWith('sftp:')) return;
        // The question carries no connection: one of a session not listed is held only while a connect is under way.
        if (!this.sessionById(e.payload.session_id) && this.pendingConnects.size === 0) return;
        this.prompts[e.payload.session_id] = e.payload;
      }),
    ]).then((fns) => {
      this.unlisten.push(...fns);
    });
    return this.listening;
  }

  /** The waiting login question of a session, handed out once. */
  takePrompt(sessionId: string): KeyboardPromptPayload | undefined {
    const p = this.prompts[sessionId];
    if (p) delete this.prompts[sessionId];
    return p;
  }

  async connect(connectionId: string): Promise<string> {
    await this.startListeners();
    this.pendingConnects.set(connectionId, (this.pendingConnects.get(connectionId) ?? 0) + 1);
    try {
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

      // Read after the awaits above: a status may come while the connection is fetched.
      const early = this.earlyStatus.get(sessionId);
      this.earlyStatus.delete(sessionId);
      const info: SshSessionInfo = {
        session_id: sessionId,
        connection_id: connectionId,
        connection_name: conn?.name ?? connectionId,
        host: conn?.host ?? '',
        port: conn?.port ?? 22,
        status: (early?.status as SshSessionInfo['status']) ?? 'connecting',
        error: early?.error ?? null,
        connected_at: null,
      };
      this.sessions = [...this.sessions, info];

      // Auto-open terminal for the new session
      this.activeTerminalId = sessionId;

      return sessionId;
    } finally {
      this.endConnect(connectionId);
    }
  }

  /** The last `connect` of a connection is over: what is held for its sessions has no one to take it. */
  private endConnect(connectionId: string) {
    const left = (this.pendingConnects.get(connectionId) ?? 1) - 1;
    if (left > 0) {
      this.pendingConnects.set(connectionId, left);
      return;
    }
    this.pendingConnects.delete(connectionId);
    for (const [id, st] of this.earlyStatus) {
      if (st.connection_id === connectionId) this.earlyStatus.delete(id);
    }
    // A question says no connection: the ones of sessions no connect listed go once none is under way.
    if (this.pendingConnects.size > 0) return;
    for (const id of Object.keys(this.prompts)) {
      if (!this.sessionById(id)) delete this.prompts[id];
    }
  }

  async disconnect(sessionId: string) {
    await api.ssh.disconnect(sessionId);
  }

  async removeSession(sessionId: string) {
    await api.ssh.sessionRemove(sessionId);
    this.sessions = this.sessions.filter((s) => s.session_id !== sessionId);
    this.dataCallbacks.delete(sessionId);
    this.dataBuffer.delete(sessionId);
    this.earlyStatus.delete(sessionId);
    delete this.prompts[sessionId];
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
    this.listening = null;
    this.earlyStatus.clear();
    this.pendingConnects.clear();
  }
}

export const sshStore = new SshStore();
