// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Types of the browser module's commands: workspaces, profiles, proxies, Camoufox, export.

export interface Workspace {
  id: string;
  name: string;
  description: string | null;
  color: string;
  icon: string;
  notes: string | null;
  is_default: boolean;
  created_at: string;
  updated_at: string;
}

export interface CreateWorkspaceRequest {
  name: string;
  description?: string | null;
  color?: string;
  icon?: string;
}

export interface UpdateWorkspaceRequest {
  name?: string;
  description?: string | null;
  color?: string;
  icon?: string;
  notes?: string | null;
}

export interface WorkspaceStats {
  id: string;
  profile_count: number;
  proxy_count: number;
  active_count: number;
}

export interface WorkspaceColumn {
  id: string;
  workspace_id: string;
  name: string;
  tag_name: string;
  color: string;
  position: number;
  created_at: string;
}

export interface CreateWorkspaceColumnRequest {
  name: string;
  tag_name: string;
  color?: string;
}

export interface UpdateWorkspaceColumnRequest {
  name?: string;
  color?: string;
  position?: number;
}

export interface Profile {
  id: string;
  name: string;
  status: 'stopped' | 'running';
  profile_path: string;
  browser_type: string;
  proxy_id: string | null;
  fingerprint_preset: string;
  user_agent: string | null;
  platform: string | null;
  timezone: string | null;
  locale: string;
  languages: string;
  screen_width: number;
  screen_height: number;
  webrtc_mode: string;
  geolocation_enabled: boolean;
  latitude: number | null;
  longitude: number | null;
  webgl_vendor: string | null;
  webgl_renderer: string | null;
  notes: string | null;
  workspace_id: string | null;
  kanban_status: string;
  kanban_order: number;
  tags: string[];
  default_search_engine: string;
  history_enabled: boolean;
  created_at: string;
  updated_at: string;
  last_launch_at: string | null;
}

export interface CreateProfileRequest {
  name: string;
  workspace_id?: string;
  browser_type?: string;
  proxy_id?: string | null;
  fingerprint_preset?: string;
  user_agent?: string | null;
  platform?: string | null;
  timezone?: string | null;
  locale?: string;
  languages?: string;
  screen_width?: number;
  screen_height?: number;
  webrtc_mode?: string;
  geolocation_enabled?: boolean;
  latitude?: number | null;
  longitude?: number | null;
  webgl_vendor?: string | null;
  webgl_renderer?: string | null;
  notes?: string | null;
  default_search_engine?: string;
  history_enabled?: boolean;
}

export interface UpdateProfileRequest extends Partial<Omit<CreateProfileRequest, 'workspace_id'>> {}

export interface Proxy {
  id: string;
  name: string;
  proxy_type: string;
  host: string;
  port: number;
  username: string | null;
  /** Secrets stay in the backend; only their presence is reported. */
  has_password: boolean;
  country: string | null;
  city: string | null;
  status: 'unknown' | 'active' | 'failed';
  last_ip: string | null;
  last_check_at: string | null;
  tags: string[];
  has_private_key: boolean;
  server_fingerprint: string | null;
  created_at: string;
}

export interface CreateProxyRequest {
  name: string;
  proxy_type: string;
  host: string;
  port: number;
  tags?: string[];
  username?: string | null;
  password?: string | null;
  country?: string | null;
  city?: string | null;
  private_key?: string | null;
}

export interface BulkProxyItem {
  line_number: number;
  proxy_type: string;
  host: string;
  port: number;
  username?: string | null;
  password?: string | null;
}

export interface BulkImportRowResult {
  line_number: number;
  status: 'imported' | 'duplicate' | 'error';
  message?: string;
  id?: string;
}

export interface BulkImportResult {
  rows: BulkImportRowResult[];
  imported: Proxy[];
}

export interface ProxyCheckResult {
  ip: string;
  country: string | null;
  city: string | null;
  ok: boolean;
  ssh_fingerprint?: string | null;
  ssh_fingerprint_is_new?: boolean | null;
}

export interface PresetInfo {
  id: string;
  label: string;
}

export interface CookieEntry {
  host: string;
  name: string;
  value: string;
  path: string;
  expiry: number | null;
  secure: boolean;
  http_only: boolean;
}

export interface ProfileRawData {
  user_agent: string;
  platform: string;
  locale: string;
  languages: string;
  timezone: string | null;
  screen_width: number;
  screen_height: number;
  webrtc_mode: string;
  webgl_vendor: string | null;
  webgl_renderer: string | null;
  canvas_seed: number;
  audio_seed: number;
  fonts_seed: number;
  geolocation_enabled: boolean;
  latitude: number | null;
  longitude: number | null;
  camoufox_config: string;
  user_js: string;
  cookies: CookieEntry[];
}

export interface CamoufoxStatus {
  installed: boolean;
  version: string | null;
  camoufox_tag: string | null;
  path: string | null;
}

/** Browser capture: domain -> folder / tags */
export interface CaptureRule {
  domain: string;
  folder_id: string | null;
  tags: string[];
  template_id?: string | null;
}

export interface ExportOptions {
  include_proxy: boolean;
  include_proxy_password: boolean;
  include_files: boolean;
}

export interface ProfileExportData {
  name: string;
  browser_type: string;
  fingerprint_preset: string;
  user_agent: string | null;
  platform: string | null;
  timezone: string | null;
  locale: string;
  languages: string;
  screen_width: number;
  screen_height: number;
  webrtc_mode: string;
  geolocation_enabled: boolean;
  latitude: number | null;
  longitude: number | null;
  webgl_vendor: string | null;
  webgl_renderer: string | null;
  notes: string | null;
  kanban_status: string;
  tags: string[];
}

export interface ProxyExportData {
  name: string;
  proxy_type: string;
  host: string;
  port: number;
  username: string | null;
  password: string | null;
  country: string | null;
  city: string | null;
}

export interface ProfileExport {
  version: string;
  exported_at: string;
  profile: ProfileExportData;
  proxy: ProxyExportData | null;
}
