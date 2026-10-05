// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Commands of the Tor module (platform-spec 11.3): the Tor Expert Bundle's
// install from torproject.org or from an archive, its removal, the settings
// and the instances of the daemon.

import { api as core, call } from '$lib/core/api';
import type { InstanceInfo, TorArchiveResult, TorDownloadState, TorSettings, TorStatus } from '$lib/tor/types';

export const api = {
  ...core,
  tor: {
    status: () => call<TorStatus>('tor_status'),
    download: () => call<void>('tor_download'),
    downloadState: () => call<TorDownloadState>('tor_download_state'),
    cancel: () => call<void>('tor_download_cancel'),
    /** `unknown_hash` installs nothing: ask, then call again with `allowUnknown`. */
    installFromArchive: (path: string, allowUnknown: boolean) =>
      call<TorArchiveResult>('tor_install_from_archive', { path, allowUnknown }),
    remove: () => call<void>('tor_remove'),
    getSettings: () => call<TorSettings>('tor_get_settings'),
    setSettings: (settings: TorSettings) => call<TorSettings>('tor_set_settings', { settings }),
    instances: () => call<InstanceInfo[]>('tor_instances'),
    /** Returns at once; the progress comes by the `tor://instances` event. Empty `exit` = any country. */
    start: (exit: string) => call<InstanceInfo>('tor_start', { exit }),
    stop: (key: string) => call<void>('tor_stop', { key }),
    newIdentity: (key: string) => call<void>('tor_new_identity', { key }),
    log: (key: string) => call<string[]>('tor_log', { key }),
    builtinBridges: () => call<string[]>('tor_builtin_bridges'),
  },
};
