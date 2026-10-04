// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { api } from '$lib/ssh/api';
import { formatError } from '$lib/core/utils';
import type {
  FileEntry,
  TransferDoneEvent,
  TransferItemInput,
  TransferKind,
  TransferProgressEvent,
} from '$lib/ssh/types';

export type PanelSource = { kind: 'local' } | { kind: 'remote'; connectionId: string };
export type SortKey = 'name' | 'size' | 'mtime' | 'permissions';

export interface FilePanelState {
  source: PanelSource;
  path: string;
  entries: FileEntry[];
  loading: boolean;
  error: string | null;
  sortKey: SortKey;
  sortDir: 1 | -1;
  showHidden: boolean;
  /** Selected entry paths (multi-select arrives in a later phase). */
  selected: string[];
}

export interface SftpKeyboardPrompt {
  connectionId: string;
  name: string;
  instructions: string;
  prompts: { prompt: string; echo: boolean }[];
  /** First-connection host key awaiting the user's trust decision. */
  hostKey: string | null;
}

export interface TransferInfo {
  id: string;
  kind: TransferKind;
  label: string;
  status: 'running' | 'done' | 'error' | 'cancelled';
  error: string | null;
  currentFile: string;
  filesDone: number;
  filesTotal: number;
  filesSkipped: number;
  bytesDone: number;
  bytesTotal: number;
  /** bytes/sec, exponential moving average */
  speed: number;
  targetPanel: 0 | 1;
  /** Set for a move: delete these sources once the copy finishes cleanly. */
  moveSources: { panel: 0 | 1; source: PanelSource; paths: string[] } | null;
}

export interface ConflictAsk {
  name: string;
  isDir: boolean;
  remaining: number;
}

type ConflictDecision = 'overwrite' | 'skip' | 'abort';

function emptyPanel(): FilePanelState {
  return {
    source: { kind: 'local' },
    path: '',
    entries: [],
    loading: false,
    error: null,
    sortKey: 'name',
    sortDir: 1,
    showHidden: false,
    selected: [],
  };
}

/** POSIX path helpers — remote paths are plain strings, never URL/PathBuf. */
export function joinPosix(dir: string, name: string): string {
  return dir.endsWith('/') ? dir + name : `${dir}/${name}`;
}

export function parentPosix(path: string): string {
  if (path === '/' || path === '') return '/';
  const trimmed = path.endsWith('/') ? path.slice(0, -1) : path;
  const idx = trimmed.lastIndexOf('/');
  return idx <= 0 ? '/' : trimmed.slice(0, idx);
}

class FilesStore {
  panels = $state<[FilePanelState, FilePanelState]>([emptyPanel(), emptyPanel()]);
  activePanel = $state<0 | 1>(0);
  /** Pending keyboard-interactive prompt (2FA) for an SFTP connect. */
  prompt = $state<SftpKeyboardPrompt | null>(null);

  transfers = $state<TransferInfo[]>([]);
  /** Conflict currently shown to the user (null = no dialog). */
  conflict = $state<ConflictAsk | null>(null);

  private conflictResolver: ((d: ConflictDecision, applyAll: boolean) => void) | null = null;
  private speedSamples = new Map<string, { bytes: number; time: number }>();
  private selectionAnchor: [string | null, string | null] = [null, null];
  private initPromise: Promise<void> | null = null;
  private unlisten: (() => void)[] = [];

  /** Idempotent; concurrent callers all await the same initialization. */
  ensureLoaded(): Promise<void> {
    if (!this.initPromise) this.initPromise = this.init();
    return this.initPromise;
  }

  private async init() {
    this.startListeners();
    // Both panels start on the local home directory.
    try {
      const home = await api.fs.home();
      await Promise.all([this.navigate(0, home), this.navigate(1, home)]);
    } catch (e) {
      this.panels[0].error = formatError(e);
    }
  }

  private startListeners() {
    const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
    if (!isTauri) return;

    import('@tauri-apps/api/event').then(({ listen }) => {
      // SFTP connects relay keyboard-interactive (2FA) prompts under a
      // synthetic session id "sftp:{connection_id}".
      listen<{
        session_id: string;
        name: string;
        instructions: string;
        prompts: { prompt: string; echo: boolean }[];
        host_key?: string | null;
      }>('ssh://keyboard-prompt', (e) => {
        const { session_id, name, instructions, prompts, host_key } = e.payload;
        if (!session_id.startsWith('sftp:')) return;
        this.prompt = {
          connectionId: session_id.slice('sftp:'.length),
          name,
          instructions,
          prompts,
          hostKey: host_key ?? null,
        };
      }).then((fn) => this.unlisten.push(fn));

      listen<{ connection_id: string; status: string; error: string | null }>(
        'sftp://status-changed',
        (e) => {
          const { connection_id, status, error } = e.payload;
          if (status !== 'disconnected' && status !== 'error') return;
          for (const panel of this.panels) {
            if (panel.source.kind === 'remote' && panel.source.connectionId === connection_id && !panel.loading) {
              panel.error = error ?? panel.error;
            }
          }
        }
      ).then((fn) => this.unlisten.push(fn));

      listen<TransferProgressEvent>('sftp://transfer-progress', (e) => {
        const p = e.payload;
        const t = this.transfers.find((t) => t.id === p.transfer_id);
        if (!t) return;
        // Speed: EMA over event deltas
        const now = performance.now();
        const prev = this.speedSamples.get(t.id);
        if (prev && now > prev.time && p.bytes_done > prev.bytes) {
          const inst = ((p.bytes_done - prev.bytes) / (now - prev.time)) * 1000;
          t.speed = t.speed > 0 ? 0.3 * inst + 0.7 * t.speed : inst;
        }
        this.speedSamples.set(t.id, { bytes: p.bytes_done, time: now });

        t.currentFile = p.current_file;
        t.filesDone = p.files_done;
        t.filesTotal = p.files_total;
        t.bytesDone = p.bytes_done;
        t.bytesTotal = p.bytes_total;
      }).then((fn) => this.unlisten.push(fn));

      listen<TransferDoneEvent>('sftp://transfer-done', (e) => {
        const p = e.payload;
        const t = this.transfers.find((t) => t.id === p.transfer_id);
        if (!t) return;
        t.status = p.error ? 'error' : p.cancelled ? 'cancelled' : 'done';
        t.error = p.error;
        t.filesDone = p.files_done;
        t.filesSkipped = p.files_skipped;
        if (t.status === 'done') t.bytesDone = t.bytesTotal;
        this.speedSamples.delete(t.id);
        // A clean cross-source move deletes its originals now.
        if (t.status === 'done' && t.moveSources) {
          this.deleteMoveSources(t.moveSources);
        }
        this.refresh(t.targetPanel);
      }).then((fn) => this.unlisten.push(fn));
    });
  }

  async respondPrompt(response: string) {
    const p = this.prompt;
    if (!p) return;
    this.prompt = null;
    await api.sftp.respondPrompt(p.connectionId, response);
  }

  cancelPrompt() {
    // Without an answer the backend times out the connect on its own (120s);
    // dropping the overlay is enough for the UI.
    this.prompt = null;
  }

  /** Switch a panel to local FS or a remote server, landing on its home dir. */
  async setSource(i: 0 | 1, source: PanelSource) {
    const panel = this.panels[i];
    panel.source = source;
    panel.entries = [];
    panel.selected = [];
    panel.error = null;
    panel.loading = true;
    try {
      const home =
        source.kind === 'local'
          ? await api.fs.home()
          : (await api.sftp.connect(source.connectionId)).home;
      await this.navigate(i, home);
    } catch (e) {
      panel.error = formatError(e);
      panel.loading = false;
    }
  }

  async navigate(i: 0 | 1, path: string) {
    const panel = this.panels[i];
    const source = panel.source;
    panel.loading = true;
    panel.error = null;
    try {
      const entries =
        source.kind === 'local'
          ? await api.fs.list(path)
          : await api.sftp.list(source.connectionId, path);
      // Ignore stale responses if the user switched source/path meanwhile.
      if (this.panels[i].source === source) {
        panel.path = path;
        panel.entries = entries;
        panel.selected = [];
      }
    } catch (e) {
      panel.error = formatError(e);
    } finally {
      panel.loading = false;
    }
  }

  async up(i: 0 | 1) {
    const panel = this.panels[i];
    const parent = parentPosix(panel.path);
    if (parent !== panel.path) await this.navigate(i, parent);
  }

  async refresh(i: 0 | 1) {
    await this.navigate(i, this.panels[i].path);
  }

  setSort(i: 0 | 1, key: SortKey) {
    const panel = this.panels[i];
    if (panel.sortKey === key) {
      panel.sortDir = panel.sortDir === 1 ? -1 : 1;
    } else {
      panel.sortKey = key;
      panel.sortDir = 1;
    }
  }

  toggleHidden(i: 0 | 1) {
    this.panels[i].showHidden = !this.panels[i].showHidden;
  }

  /** Entries filtered by the hidden-files toggle and sorted (dirs first). */
  visibleEntries(i: 0 | 1): FileEntry[] {
    const panel = this.panels[i];
    const { sortKey, sortDir } = panel;
    const items = panel.showHidden
      ? [...panel.entries]
      : panel.entries.filter((e) => !e.name.startsWith('.'));
    items.sort((a, b) => {
      if (a.is_dir !== b.is_dir) return a.is_dir ? -1 : 1;
      let cmp = 0;
      switch (sortKey) {
        case 'size':
          cmp = a.size - b.size;
          break;
        case 'mtime':
          cmp = (a.mtime ?? 0) - (b.mtime ?? 0);
          break;
        case 'permissions':
          cmp = a.octal.localeCompare(b.octal);
          break;
        default:
          cmp = a.name.localeCompare(b.name, undefined, { numeric: true, sensitivity: 'base' });
      }
      if (cmp === 0 && sortKey !== 'name') {
        cmp = a.name.localeCompare(b.name, undefined, { numeric: true, sensitivity: 'base' });
      }
      return cmp * sortDir;
    });
    return items;
  }

  // ── Selection ────────────────────────────────────────────────────────────

  /** Click selection: plain = single, ctrl = toggle, shift = range from anchor. */
  select(i: 0 | 1, entry: FileEntry, mods: { ctrl: boolean; shift: boolean }) {
    this.activePanel = i;
    const panel = this.panels[i];
    if (mods.shift && this.selectionAnchor[i]) {
      const visible = this.visibleEntries(i);
      const a = visible.findIndex((e) => e.path === this.selectionAnchor[i]);
      const b = visible.findIndex((e) => e.path === entry.path);
      if (a >= 0 && b >= 0) {
        const [from, to] = a < b ? [a, b] : [b, a];
        panel.selected = visible.slice(from, to + 1).map((e) => e.path);
        return;
      }
    }
    if (mods.ctrl) {
      panel.selected = panel.selected.includes(entry.path)
        ? panel.selected.filter((p) => p !== entry.path)
        : [...panel.selected, entry.path];
    } else {
      panel.selected = [entry.path];
    }
    this.selectionAnchor[i] = entry.path;
  }

  selectAll(i: 0 | 1) {
    this.panels[i].selected = this.visibleEntries(i).map((e) => e.path);
  }

  // ── Copy between panels (transfers) ──────────────────────────────────────

  /** Direction supported in this phase: exactly one panel is remote. */
  canCopy(from: 0 | 1): boolean {
    const src = this.panels[from];
    const dst = this.panels[from === 0 ? 1 : 0];
    if (src.selected.length === 0) return false;
    return (
      (src.source.kind === 'local') !== (dst.source.kind === 'local') &&
      !src.loading &&
      !dst.loading
    );
  }

  /**
   * Copy the selection of panel `from` into the other panel.
   * Top-level name conflicts are confirmed via the conflict dialog
   * (overwrite / skip / apply-to-all); unfinished `.veydanpart` files are
   * resumed automatically by the backend.
   */
  async copyToOtherPanel(from: 0 | 1, opts: { deleteSourceAfter?: boolean } = {}) {
    if (!this.canCopy(from)) return;
    const to = from === 0 ? 1 : 0;
    const src = this.panels[from];
    const dst = this.panels[to];
    const srcSource = src.source;

    const kind: TransferKind = src.source.kind === 'local' ? 'upload' : 'download';
    const connectionId =
      src.source.kind === 'remote'
        ? src.source.connectionId
        : (dst.source as { kind: 'remote'; connectionId: string }).connectionId;

    const entries = src.entries.filter((e) => src.selected.includes(e.path));
    if (entries.length === 0) return;

    // Top-level conflict detection + interactive resolution
    const items: TransferItemInput[] = [];
    let applyAllDecision: ConflictDecision | null = null;
    for (let idx = 0; idx < entries.length; idx++) {
      const entry = entries[idx];
      const dstPath = joinPosix(dst.path, entry.name);
      const existing =
        dst.source.kind === 'local'
          ? await api.fs.stat(dstPath)
          : await api.sftp.stat(connectionId, dstPath);

      let overwrite = false;
      if (existing && !(existing.is_dir && entry.is_dir)) {
        // dir→dir merges silently; anything else asks
        let decision = applyAllDecision;
        if (!decision) {
          const [d, all] = await this.askConflict({
            name: entry.name,
            isDir: entry.is_dir,
            remaining: entries.length - idx - 1,
          });
          decision = d;
          if (all) applyAllDecision = d;
        }
        if (decision === 'abort') return;
        if (decision === 'skip') continue;
        overwrite = true;
      }
      items.push({ src_path: entry.path, dst_path: dstPath, overwrite });
    }
    if (items.length === 0) return;

    const label =
      items.length === 1
        ? entries.find((e) => joinPosix(dst.path, e.name) === items[0].dst_path)?.name ?? items[0].src_path
        : `${items.length}`;

    const transferId = await api.sftp.transferStart(kind, connectionId, items);
    this.transfers.push({
      id: transferId,
      kind,
      label,
      status: 'running',
      error: null,
      currentFile: '',
      filesDone: 0,
      filesTotal: 0,
      filesSkipped: 0,
      bytesDone: 0,
      bytesTotal: 0,
      speed: 0,
      targetPanel: to,
      moveSources: opts.deleteSourceAfter
        ? { panel: from, source: srcSource, paths: items.map((it) => it.src_path) }
        : null,
    });
  }

  private askConflict(ask: ConflictAsk): Promise<[ConflictDecision, boolean]> {
    this.conflict = ask;
    return new Promise((resolve) => {
      this.conflictResolver = (d, all) => {
        this.conflict = null;
        this.conflictResolver = null;
        resolve([d, all]);
      };
    });
  }

  resolveConflict(decision: ConflictDecision, applyAll: boolean) {
    this.conflictResolver?.(decision, applyAll);
  }

  async cancelTransfer(id: string) {
    await api.sftp.transferCancel(id);
  }

  clearFinishedTransfers() {
    this.transfers = this.transfers.filter((t) => t.status === 'running');
  }

  // ── File / folder operations ─────────────────────────────────────────────

  /** Whether two panels share the same source (enables cheap rename-move). */
  private sameSource(a: PanelSource, b: PanelSource): boolean {
    if (a.kind === 'local' && b.kind === 'local') return true;
    return a.kind === 'remote' && b.kind === 'remote' && a.connectionId === b.connectionId;
  }

  async mkdir(i: 0 | 1, name: string) {
    const panel = this.panels[i];
    const path = joinPosix(panel.path, name);
    if (panel.source.kind === 'local') await api.fs.mkdir(path);
    else await api.sftp.mkdir(panel.source.connectionId, path);
    await this.refresh(i);
    this.selectByName(i, name);
  }

  async createFile(i: 0 | 1, name: string) {
    const panel = this.panels[i];
    const path = joinPosix(panel.path, name);
    if (panel.source.kind === 'local') await api.fs.createFile(path);
    else await api.sftp.createFile(panel.source.connectionId, path);
    await this.refresh(i);
    this.selectByName(i, name);
  }

  async rename(i: 0 | 1, entry: FileEntry, newName: string) {
    if (!newName || newName === entry.name) return;
    const panel = this.panels[i];
    const to = joinPosix(panel.path, newName);
    if (panel.source.kind === 'local') await api.fs.rename(entry.path, to);
    else await api.sftp.rename(panel.source.connectionId, entry.path, to);
    await this.refresh(i);
    this.selectByName(i, newName);
  }

  /** Delete every selected entry (recursive for directories). */
  async deleteSelected(i: 0 | 1) {
    const panel = this.panels[i];
    const paths = [...panel.selected];
    for (const path of paths) {
      if (panel.source.kind === 'local') await api.fs.delete(path);
      else await api.sftp.delete(panel.source.connectionId, path);
    }
    await this.refresh(i);
  }

  /** Apply one mode to every selected entry. */
  async chmodSelected(i: 0 | 1, mode: number) {
    const panel = this.panels[i];
    const paths = [...panel.selected];
    for (const path of paths) {
      if (panel.source.kind === 'local') await api.fs.chmod(path, mode);
      else await api.sftp.chmod(panel.source.connectionId, path, mode);
    }
    await this.refresh(i);
  }

  /**
   * Move the selection to the other panel. Same source → server-side rename
   * (instant). Different sources → copy, then delete the originals once the
   * transfer finishes cleanly.
   */
  async moveToOtherPanel(from: 0 | 1) {
    const to = from === 0 ? 1 : 0;
    const src = this.panels[from];
    const dst = this.panels[to];
    const entries = src.entries.filter((e) => src.selected.includes(e.path));
    if (entries.length === 0) return;

    if (this.sameSource(src.source, dst.source)) {
      for (const entry of entries) {
        const target = joinPosix(dst.path, entry.name);
        if (src.source.kind === 'local') await api.fs.rename(entry.path, target);
        else await api.sftp.rename(src.source.connectionId, entry.path, target);
      }
      await Promise.all([this.refresh(from), this.refresh(to)]);
      return;
    }
    // Cross-source: reuse the copy pipeline, then delete sources on success.
    await this.copyToOtherPanel(from, { deleteSourceAfter: true });
  }

  private async deleteMoveSources(mv: { panel: 0 | 1; source: PanelSource; paths: string[] }) {
    for (const path of mv.paths) {
      try {
        if (mv.source.kind === 'local') await api.fs.delete(path);
        else await api.sftp.delete(mv.source.connectionId, path);
      } catch {
        // A failed delete leaves the original in place — safer than losing data.
      }
    }
    // Refresh the source panel if it still shows the same source.
    if (this.sameSource(this.panels[mv.panel].source, mv.source)) {
      await this.refresh(mv.panel);
    }
  }

  private selectByName(i: 0 | 1, name: string) {
    const match = this.panels[i].entries.find((e) => e.name === name);
    this.panels[i].selected = match ? [match.path] : [];
  }

  clipboardPath(entry: FileEntry) {
    navigator.clipboard?.writeText(entry.path).catch(() => {});
  }

  destroy() {
    this.unlisten.forEach((fn) => fn());
    this.unlisten = [];
  }
}

export const filesStore = new FilesStore();
