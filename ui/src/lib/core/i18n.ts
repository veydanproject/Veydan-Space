// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { writable, derived, get, type Readable } from 'svelte/store';
import { translations as moduleTranslations, locales as moduleLocales } from 'virtual:veydan-modules/i18n';
import { product } from '$lib/core/product';
import languageList from './languages.json';

/** A language of the UI: a `code` of languages.json (`en`, `ru`, `pt-BR`…). */
export type Locale = string;

/** One language of the UI (languages.json, platform-spec 11.8). */
export type Language = { code: Locale; tag: string; native: string };

/** The languages of the UI, in the order of the picker. */
export const LANGUAGES: readonly Language[] = languageList.languages;

/** The languages written in the dictionaries themselves; the rest come from `locales/<code>.json`. */
const BUILT_IN: readonly Locale[] = ['en', 'ru'];

export function isLocale(value: unknown): value is Locale {
  return typeof value === 'string' && LANGUAGES.some((l) => l.code === value);
}

/** The BCP 47 tag of a language for Intl and `<html lang>`. */
export function localeTag(code: Locale): string {
  return LANGUAGES.find((l) => l.code === code)?.tag ?? code;
}

const translations = {
  en: {
    note_template_none: 'No template',
    note_template_label: 'Template',
    // Names of the entity kinds: a reference is shown with them even where the kind's owner is absent (platform-spec 10.3).
    ctx_kind_workspace: 'Workspace',
    ctx_kind_profile: 'Profile',
    ctx_kind_proxy: 'Proxy',
    ctx_kind_ssh: 'SSH',
    ctx_kind_totp: 'TOTP',
    ctx_kind_password: 'Password',
    ctx_kind_note: 'Note',
    ctx_available_in_space: 'available in Veydan Space',
    ctx_action_open: 'Open',
    // The dictionaries of the product's modules (platform-spec 11.5); a module's
    // keys exist only where the module is.
    ...moduleTranslations.en,
    // Nav
    nav_profiles: 'Profiles',
    nav_settings: 'Settings',
    nav_back_to: 'Back to {name}',
    common_close: 'Close',
    common_cancel: 'Cancel',
    common_delete: 'Delete',
    common_confirm: 'OK',
    window_minimize: 'Minimize',
    window_maximize: 'Maximize',
    window_restore: 'Restore',
    settings_theme_dark: 'Dark',
    settings_theme_light: 'Light',
    theme_toggle: 'Toggle theme',
    settings_section_theme: 'Theme',
    settings_theme_chrome: 'Frame',
    settings_theme_accent: 'Accent',
    settings_theme_bg: 'Background',
    settings_theme_reset: 'Reset',
    workspaces_delete_confirm: 'Delete workspace "{name}"?',
    workspace_tab_notes: 'Notes',
    // Kanban
    kanban_col_new: 'New',
    kanban_col_warmup: 'Warmup',
    kanban_col_active: 'Active',
    kanban_col_dead: 'Dead',
    kanban_filter_country: 'All countries',
    table_col_status: 'Status',
    table_col_column: 'Column',
    // Profiles page
    profiles_title: 'Profiles',
    profiles_new: '+ New Profile',
    profiles_search: 'Search profiles...',
    profiles_filter_all: 'All',
    profiles_filter_running: 'Running',
    profiles_filter_stopped: 'Stopped',
    profiles_empty: 'No profiles yet.',
    profiles_empty_create: 'Create first profile',
    profiles_no_match: 'No profiles match your search.',
    profiles_no_proxy: 'No proxy',
    profiles_last_launch: 'Last launch',
    profiles_btn_launch: 'Launch',
    profiles_btn_stop: 'Stop',
    profiles_btn_edit: 'Edit',
    profiles_btn_clone: 'Clone',
    profile_section_privacy: 'Privacy',
    profile_field_ua: 'User-Agent Override (optional)',
    profile_field_ua_placeholder: 'Leave empty to use preset',
    profile_field_languages: 'Languages',
    proxy_col_ip: 'Last IP',
    cancel: 'Cancel',
    proxy_btn_checking: 'Checking…',
    proxy_last_ip: 'Last IP:',
    // Settings page
    settings_title: 'Settings',
    settings_group_general: 'General',
    settings_group_notes: 'Notes',
    settings_group_security: 'Security',
    settings_group_data: 'Data',
    settings_group_about: 'About',
    settings_section_language: 'Language',
    settings_language_label: 'Interface language',
    update_banner_available: 'Version {version} is available',
    update_banner_install: 'Install',
    update_banner_later: 'Later',
    settings_update_section: 'App updates',
    settings_update_check: 'Check for updates',
    settings_update_checking: 'Checking…',
    settings_update_up_to_date: 'You are on the latest version.',
    settings_update_available: 'Update available: {version}',
    settings_update_install: 'Install & restart',
    settings_update_downloading: 'Downloading update…',
    settings_update_installing: 'Installing… the app will restart',
    settings_update_unsupported:
      'Auto-update is not available for deb/rpm installs. Download the new version from the releases page.',
    settings_update_open_releases: 'Open releases page',
    settings_update_failed: 'Could not check for updates. Check the connection and try again.',
    settings_data_section: 'Demo & data',
    settings_data_hint: 'Load sample content for demos, or wipe local catalog data without reinstalling.',
    settings_demo_locale: 'Demo content language',
    settings_demo_locale_en: 'English',
    settings_demo_locale_ru: 'Русский',
    settings_demo_load: 'Load demo data',
    settings_demo_loading: 'Loading…',
    settings_demo_confirm: 'The data of these sections will be replaced with the demo set: {list}. The app lock becomes the password "demo"; its recovery key is not shown, so create a new one in Settings → App lock if you keep the demo. Continue?',
    settings_demo_done: 'Demo data loaded.',
    settings_clear_data: 'Clear app data',
    settings_clear_clearing: 'Clearing…',
    settings_clear_confirm: 'Delete all data of these sections: {list}? This cannot be undone. The app lock and its password stay as they are.',
    settings_clear_done: 'App data cleared.',
    dev_sync_link: 'For developers',
    dev_sync_title: 'Sync log',
    dev_sync_back: 'Settings',
    dev_sync_enable: 'Sync log',
    dev_sync_enable_hint: 'Records each sync cycle: start, skip, retry and errors. Off by default.',
    dev_sync_clear: 'Clear',
    dev_sync_copy: 'Copy',
    dev_sync_copied: 'Copied',
    dev_sync_empty: 'No lines yet. Turn the log on and wait for a sync cycle.',
    settings_section_about: 'About',
    settings_about_version: 'Version',
    settings_about_copyright: 'Copyright © 2026 Veydan Project',
    settings_about_license: 'License',
    settings_about_license_note:
      'Source-available: free to read, build, run, modify and redistribute — but you may not use it to build a product that competes with {app}. This is not OSI open source.',
    settings_about_link_license: 'Read the full license',
    settings_about_link_summary: 'License summary (5 languages)',
    settings_about_link_thirdparty: 'Third-party licenses',
    settings_about_repo: 'Source code',
    settings_about_link_repo: 'View on GitHub',
    settings_bug_report: 'Report a bug',
    settings_bug_hint: 'Fill in what happened, then send the text to this address.',
    settings_bug_copy: 'Copy',
    settings_bug_copied: 'Copied',
    settings_bug_mail: 'Open mail',
    app_name: '{app}',
    settings_bug_subject: '{app} bug report',
    settings_bug_os: 'OS',
    settings_bug_language: 'Language',
    settings_bug_date: 'Date',
    settings_bug_what: 'What happened:',
    settings_bug_steps: 'Steps:',
    settings_bug_expected: 'Expected:',
    back_profiles: '← Profiles',
    loading: 'Loading…',
    export_btn_json: 'Save JSON',
    export_btn_zip: 'Save ZIP',
    export_btn_json_loading: 'Exporting…',
    export_btn_zip_loading: 'Saving…',
    // Password generator
    pwgen_btn_toggle_theme: 'Toggle theme',
    // TOTP
    totp_btn_close: 'Close',
    totp_filter_workspace: 'Workspace',
    totp_field_tags: 'Tags',
    totp_profile_badge: 'Profile',
    pw_field_profile: 'Profile',
    pw_field_workspace: 'Workspace',
    pw_unlock_shared: 'Enter the lock password set on your other device.',
    pw_locked: 'Unlock to reveal',
    pw_unlock: 'Unlock',
    pw_mismatch: 'Password vault cannot be unlocked with the current {app} lock.',
    pw_reset: 'Reset password vault',
    pw_reset_confirm: 'Delete every saved password? This cannot be undone.',
    pw_err_locked: 'The vault is locked.',
    pw_err_mismatch: 'Password vault cannot be unlocked with the current {app} lock.',
    pw_err_decrypt: 'Could not decrypt this entry.',
    pw_notes: 'Notes',
    pw_new_note: 'New note',
    pw_note_search: 'Find a note…',
    notes_sync_never: 'never',
    note_sync_conflict_remote: 'Version from another device',
    note_sync_conflict_remote_short: 'Other device',
    note_sync_conflict_mine: 'My version',
    note_sync_conflict_keep: 'Keep my version',
    note_sync_conflict_take: 'Take the version from another device',
    note_sync_conflict_review: 'Review line by line',
    note_sync_conflict_both: 'Both',
    note_sync_conflict_all_mine: 'All mine',
    note_sync_conflict_all_theirs: 'All theirs',
    ctx_action_copy_url: 'Copy URL',
    // Command palette
    cmd_title: 'Command palette',
    cmd_placeholder: 'Type a command…',
    cmd_empty: 'No matching commands',
    cmd_app_settings: 'Open settings',
    cmd_app_lock: 'Lock {app}',
    // Attachment policy (settings)
    notes_areas_color: 'Pick color',
    notes_areas_placeholder: 'Area name',
    notes_areas_add: 'Add',
    notes_untitled: 'Untitled',
    notes_tags_add: 'Add tag',
    notes_tags_create: 'Create "{name}"',
    notes_tag_rename: 'Rename',
    notes_tag_delete: 'Delete',
    notes_tag_save: 'Save',
    // Settings – notes storage
    // Settings – browser capture rules
    // Notes – quick capture
    settings_lock_section: 'App lock',
    settings_lock_hint: 'A PIN or password is required to open {app}. Only an Argon2 hash is stored.',
    settings_lock_status_off: 'Off',
    settings_lock_kind: 'Lock with',
    settings_lock_current: 'Current PIN or password',
    settings_lock_new_pin: 'New PIN',
    settings_lock_new_password: 'New password',
    settings_lock_confirm_pin: 'Repeat PIN',
    settings_lock_confirm_password: 'Repeat password',
    settings_lock_min_len: 'At least 4 characters',
    settings_lock_pin_digits: 'Digits only',
    settings_lock_hint_label: 'Hint (optional)',
    settings_lock_hint_note: 'Never put the PIN or password itself here.',
    settings_lock_hint_ph: 'Shown after a wrong attempt',
    settings_lock_warning: 'Without a PIN or password, anyone with this device can open {app} and read what it keeps. {app} cannot reset a forgotten PIN or password: keep the recovery key you get after turning the lock on, or the data under the lock is lost for good.',
    settings_lock_enable: 'Turn on lock',
    settings_lock_change: 'Save changes',
    settings_lock_disable: 'Turn off lock',
    settings_lock_timeout: 'Auto-lock after',
    settings_lock_timeout_never: 'Never',
    settings_lock_timeout_min: '{n} min',
    settings_lock_recovery: 'Recovery key',
    settings_lock_recovery_has: 'Created',
    settings_lock_recovery_none: 'Not created',
    settings_lock_recovery_new: 'Create new key',
    settings_lock_recovery_new_hint: 'Enter the current PIN or password. The previous key stops working.',
    lock_kind_pin: 'PIN',
    lock_kind_password: 'Password',
    lock_title_pin: 'Enter PIN',
    lock_title_password: 'Enter password',
    lock_unlock: 'Unlock',
    lock_wrong: 'Wrong PIN or password',
    lock_hint_prefix: 'Hint: {hint}',
    lock_forgot: 'Forgot PIN or password?',
    lock_recovery_title: 'Your recovery key',
    lock_recovery_file_title: '{app} recovery key',
    lock_recovery_intro: 'Shown only once. With this key you can set a new PIN or password without losing the data under the lock. Store it somewhere safe, outside {app}.',
    lock_recovery_copy: 'Copy key',
    lock_recovery_copied: 'Copied',
    lock_recovery_save_file: 'Save to file',
    lock_recovery_share: 'Share',
    lock_recovery_ack: 'I have saved the key in a safe place',
    lock_recovery_done: 'Done',
    lock_recovery_invalid: 'Recovery key does not match.',
    lock_recover_title: 'Recover access',
    lock_recover_step: 'Step {n} of 3',
    lock_recover_step1: 'Enter the recovery key',
    lock_recover_step1_hint: 'The 24-character key you saved when you turned the lock on. Case and dashes do not matter.',
    lock_recover_paste: 'Paste',
    lock_recover_continue: 'Continue',
    lock_recover_step2: 'Choose a new PIN or password',
    lock_recover_step3: 'Save the new recovery key',
    lock_recover_step3_hint: 'The key you just used no longer works.',
    lock_recover_none: 'No recovery key exists for this lock. Access can only be restored by clearing {app} data on this device.',
    lock_recover_back: 'Back to unlock',
    // Settings – backup
    settings_backup_browse: 'Browse',
    settings_backup_save: 'Save',
    settings_backup_never: 'Never',
    settings_backup_cancel: 'Cancel',
    // Settings – sync (beta)
    settings_sync_section: 'Sync',
    settings_sync_beta: 'Beta',
    settings_sync_hint: 'Sync your data between devices through an end-to-end encrypted vault. The storage only ever sees ciphertext: a folder mirrored by any cloud client, an S3-compatible bucket, or a WebDAV share.',
    settings_sync_backend: 'Storage',
    settings_sync_backend_folder: 'Folder',
    settings_sync_backend_s3: 'S3-compatible',
    settings_sync_backend_webdav: 'WebDAV',
    settings_sync_folder: 'Vault folder',
    settings_sync_folder_placeholder: 'Choose a folder that your cloud client syncs',
    settings_sync_folder_hint: 'The path may differ on each device; the vault is identified by its manifest, not by the path.',
    settings_sync_s3_endpoint: 'Endpoint',
    settings_sync_s3_region: 'Region',
    settings_sync_s3_bucket: 'Bucket',
    settings_sync_s3_prefix: 'Prefix',
    settings_sync_s3_access_key: 'Access key',
    settings_sync_s3_secret_key: 'Secret key',
    settings_sync_s3_path_style: 'Path-style addressing',
    settings_sync_s3_path_style_hint: 'endpoint/bucket/key — required by MinIO and most self-hosted servers.',
    settings_sync_webdav_url: 'Collection URL',
    settings_sync_webdav_username: 'Username',
    settings_sync_webdav_password: 'Password',
    settings_sync_interval: 'Poll interval',
    settings_sync_unit_seconds: 'seconds',
    settings_sync_unit_minutes: 'minutes',
    settings_sync_interval_presets: 'Quick:',
    settings_sync_interval_remote_warn: 'Intervals under a minute are meant for testing: on S3/WebDAV they produce frequent requests and may cost money.',
    settings_sync_profile_files: 'Sync browser profile files',
    settings_sync_profile_files_hint: 'Cookies, sessions and history of each profile. Off: settings still sync, and a running profile is marked on other devices.',
    settings_sync_device_name: 'Device name',
    settings_sync_device_name_hint: 'Shown on your other devices.',
    settings_sync_device_name_profiles_hint: 'Shown on other devices while a profile runs here.',
    settings_sync_interval_sec: '{n} s',
    settings_sync_interval_min: '{n} min',
    settings_sync_profile_conflicts: 'Profile files changed on both sides',
    settings_sync_gc: 'Vault blobs',
    settings_sync_gc_value: '{total} stored, {removed} removed at last cleanup {when}',
    settings_sync_gc_hint: 'Unreferenced blobs are removed once a day after a 24-hour grace period.',
    settings_sync_lf_gc: 'Large-file objects',
    settings_sync_lf_section: 'Large file transfer',
    settings_sync_lf_hint: 'This device only. Applies to new files; already uploaded files keep their chunk size.',
    settings_sync_lf_chunk: 'Chunk size, MiB',
    settings_sync_lf_chunk_hint: '{min}–{max}. Larger chunks mean fewer requests, more memory.',
    settings_sync_lf_parallelism: 'Parallel chunks',
    settings_sync_lf_parallelism_hint: '{min}–{max} chunks in flight at once.',
    settings_sync_lf_resume: 'Resume interrupted transfers',
    settings_sync_lf_resume_hint: 'Keeps partial downloads on disk and continues from the last complete chunk.',
    settings_sync_lf_ram: 'Estimated peak memory per transfer: about {mib} MiB.',
    settings_sync_lf_chunk_warn: 'Changing the chunk size does not re-encode files already in the vault.',
    settings_sync_phase_attachments_up: 'Uploading attachments',
    settings_sync_phase_attachments_down: 'Downloading attachments',
    profile_sync_diverged_hint: 'The profile ran on two devices. Pick which files to keep; the other version is replaced.',
    settings_sync_enabled: 'Background sync',
    settings_sync_enabled_hint: 'Poll the storage on the interval above. Off pauses sync without leaving the vault.',
    settings_sync_probe: 'Check storage',
    settings_sync_probe_empty: 'Storage is empty — create a new vault here.',
    settings_sync_probe_vault: 'A vault was found — join it with its passphrase.',
    settings_sync_probe_foreign: 'Storage is not empty and has no vault. Pick an empty folder.',
    settings_sync_passphrase: 'Vault passphrase',
    settings_sync_passphrase_placeholder: 'At least 8 characters',
    settings_sync_passphrase_hint: 'Create makes a new vault in an empty storage. Join opens the vault already there. Other devices enter the same passphrase.',
    settings_sync_create: 'Create vault',
    settings_sync_join: 'Join vault',
    settings_sync_created: 'Vault created. Sync is enabled.',
    settings_sync_joined: 'Joined the vault. Sync is enabled.',
    settings_sync_join_own_lock: 'Passwords or Chat keys on this device are behind the lock it has now: one set here, or that of a vault it was connected to before. Turn the lock off on this device, then connect to the vault again.',
    settings_sync_now: 'Sync now',
    settings_sync_running: 'Syncing…',
    settings_sync_done: 'Sync finished.',
    settings_sync_phase_collect: 'Collecting changes',
    settings_sync_phase_push: 'Uploading',
    settings_sync_phase_pull: 'Downloading',
    settings_sync_phase_apply: 'Applying',
    settings_sync_phase_profiles_up: 'Uploading profile files',
    settings_sync_phase_profiles_down: 'Downloading profile files',
    settings_sync_phase_compact: 'Compacting log',
    settings_sync_phase_gc: 'Cleaning unused blobs',
    settings_sync_phase_done: 'Done',
    settings_sync_phase_error: 'Failed',
    settings_sync_progress: '{phase} · {percent}%',
    settings_sync_progress_files: '{phase} · {current}/{total} · {percent}%',
    settings_sync_leave: 'Leave vault',
    settings_sync_leave_confirm: 'This device forgets the vault key and its sync position. Nothing is deleted from the storage or from local notes.',
    settings_sync_left: 'Left the vault.',
    settings_sync_change_passphrase: 'Change passphrase',
    settings_sync_change_passphrase_hint:
      'Only the passphrase is rewrapped; the vault master key stays the same. Anyone holding an older copy of manifest.json can still open the vault with the old passphrase. If it may have leaked, create a new vault instead.',
    settings_sync_passphrase_old: 'Current passphrase',
    settings_sync_passphrase_new: 'New passphrase',
    settings_sync_passphrase_changed: 'Passphrase changed. Other devices keep working; new devices use the new one.',
    settings_sync_status: 'Status',
    settings_sync_status_joined: 'joined',
    settings_sync_status_not_joined: 'not joined',
    settings_sync_vault_id: 'Vault',
    settings_sync_device_id: 'This device',
    settings_sync_peers: 'Other devices',
    settings_sync_storage_devices: 'Devices in storage',
    settings_sync_device_unnamed: 'Unnamed',
    settings_sync_last_applied: 'Received last cycle',
    settings_sync_last_run: 'Last sync',
    settings_sync_last_error: 'Last error',
    settings_sync_last_warning: 'Warning',
    settings_sync_conflicts: 'Notes with conflicts',
    settings_sync_conflicts_hint: 'Both devices edited the same lines. Your text is kept; open the note and choose which version to keep.',
    // SSH
    ssh_scope_all: 'All',
    ssh_scope_global: 'Global',
    ssh_scope_workspace: 'Workspace',
    ssh_scope_profile: 'Profile',
    ssh_field_scope: 'Scope',
    ssh_totp_prompt: 'Enter TOTP code for',
    ssh_totp_placeholder: '123456',
    ssh_btn_connect: 'Connect',
    ssh_open_panel: 'Open SSH panel',
    ssh_profile_no_linked: 'No SSH connections linked to this profile',
    secret_stored_placeholder: 'Stored — leave empty to keep',
    secret_clear: 'Remove stored value',
    terminal_col_status: 'Status',
    // UI Inspector / Design QA
    hotkey_section: 'Hotkeys',
    hotkey_hint: 'A shortcut follows the physical key, not the letter of the current keyboard layout.',
    hotkey_group_edit: 'Editing',
    hotkey_group_editor: 'Editor',
    hotkey_group_app: 'Application',
    hotkey_edit_undo: 'Undo',
    hotkey_edit_redo: 'Redo',
    hotkey_editor_bold: 'Bold',
    hotkey_editor_italic: 'Italic',
    hotkey_editor_link: 'Link',
    hotkey_editor_find: 'Find',
    hotkey_editor_save: 'Save',
    hotkey_app_palette: 'Command palette',
    hotkey_app_inspector: 'UI inspector',
    hotkey_change: 'Change',
    hotkey_reset: 'Reset',
    hotkey_record: 'Press a shortcut…',
    hotkey_unbound: 'Not set',
    hotkey_need_modifier: 'Add Ctrl, Alt, or Cmd',
    hotkey_conflict: 'Already used by {name}',
    inspector_toggle: 'UI Inspector',
    inspector_grid: 'Grid',
    inspector_grid_off: 'Off',
    inspector_zoom: 'Zoom',
    inspector_rulers: 'Rulers',
    inspector_guides: 'Guides',
    inspector_outline: 'Outline all',
    inspector_inspect: 'Inspect',
    inspector_screenshot: 'Screenshot',
    inspector_presets: 'Presets',
    inspector_preset_save: 'Save',
    inspector_preset_delete: 'Delete',
    inspector_preset_name: 'Preset name',
    inspector_custom_grid: 'Custom grid',
    inspector_columns: 'Columns',
    inspector_gutter: 'Gutter',
    inspector_margin: 'Margin',
    inspector_viewport: 'Viewport',
    inspector_hotkey_hint: 'Toggle with {keys}',
    inspector_developer_tools: 'Developer Tools',
    // System tray
    settings_tray_section: 'System tray',
    settings_tray_minimize: 'Minimize to tray',
    settings_tray_minimize_hint: 'When you minimize the window, hide it to the system tray instead of the taskbar.',
    settings_tray_close: 'Close to tray',
    settings_tray_close_hint: 'Closing the window (×) hides it to the tray instead of quitting the app.',
    settings_tray_start_hidden: 'Start hidden in tray',
    settings_tray_start_hidden_hint: 'Launch the app straight into the tray without showing the window.',
    tray_show: 'Show {app}',
    tray_hide: 'Hide window',
    tray_quit: 'Quit',
    tray_tooltip: '{app} — {n} running',
    // Start error screen (degraded start, shown instead of the shell)
    start_error_title: 'The app did not start',
    start_error_db_foreign: 'The data file {path} was not created by {app} and will not be opened.',
    start_error_db_dev_schema: 'The data file {path} was created by another build of {app} and will not be opened.',
    start_error_untouched: 'Nothing was changed: the file is exactly as it was.',
    start_error_fix:
      'Close the app, move this file to another folder together with every file beside it whose name starts with app.db- and start the app again. A new data file will be created.',
    start_error_close: 'Close',
    start_error_no_answer: 'The app did not answer the window. Nothing was changed; try again.',
    start_error_retry: 'Try again',
    start_error_wrong_ui_phone: 'This window was given the phone interface of {app}: the app was built with the pages of the Android build.',
    start_error_wrong_ui_desktop: 'This device was given the desktop interface of {app}: the app was built with the pages of the desktop build.',
    start_error_wrong_ui_fix: 'Nothing was changed. Build the app again, or install a release build.',
    // A screen that failed to show, and the address without a page
    screen_failed: 'This screen could not be shown.',
    screen_to_start: 'To the start screen',
    // The switches of the modules (Space)
    modules_section: 'Modules',
    modules_hint: 'What this device shows. A module that is off keeps its data, and its data keeps syncing.',
    modules_last: 'At least one module stays on.',
    modules_first_title: 'What do you want to use?',
    modules_first_hint: 'Choose what this device shows. You can change it at any time in Settings → Modules.',
    modules_first_all: 'Use everything',
    modules_first_continue: 'Continue',
    settings_lock_recovery_new_title: 'Your new recovery key',
  },

  ru: {
    note_template_none: 'Без шаблона',
    note_template_label: 'Шаблон',
    ctx_kind_workspace: 'Воркспейс',
    ctx_kind_profile: 'Профиль',
    ctx_kind_proxy: 'Прокси',
    ctx_kind_ssh: 'SSH',
    ctx_kind_totp: 'TOTP',
    ctx_kind_password: 'Пароль',
    ctx_kind_note: 'Заметка',
    ctx_available_in_space: 'доступно в Veydan Space',
    ctx_action_open: 'Открыть',
    ...moduleTranslations.ru,
    // Nav
    nav_profiles: 'Профили',
    nav_settings: 'Настройки',
    nav_back_to: 'Назад: {name}',
    common_close: 'Закрыть',
    common_cancel: 'Отмена',
    common_delete: 'Удалить',
    common_confirm: 'ОК',
    window_minimize: 'Свернуть',
    window_maximize: 'Развернуть',
    window_restore: 'Восстановить',
    settings_theme_dark: 'Тёмная',
    settings_theme_light: 'Светлая',
    theme_toggle: 'Переключить тему',
    settings_section_theme: 'Тема',
    settings_theme_chrome: 'Рамка',
    settings_theme_accent: 'Акцент',
    settings_theme_bg: 'Фон',
    settings_theme_reset: 'Сбросить',
    workspaces_delete_confirm: 'Удалить воркспейс "{name}"?',
    workspace_tab_notes: 'Заметки',
    // Kanban
    kanban_col_new: 'Новые',
    kanban_col_warmup: 'Прогрев',
    kanban_col_active: 'Рабочие',
    kanban_col_dead: 'Мёртвые',
    kanban_filter_country: 'Все страны',
    table_col_status: 'Статус',
    table_col_column: 'Колонка',
    // Profiles page
    profiles_title: 'Профили',
    profiles_new: '+ Новый профиль',
    profiles_search: 'Поиск профилей...',
    profiles_filter_all: 'Все',
    profiles_filter_running: 'Запущены',
    profiles_filter_stopped: 'Остановлены',
    profiles_empty: 'Профилей пока нет.',
    profiles_empty_create: 'Создать первый профиль',
    profiles_no_match: 'Профили не найдены.',
    profiles_no_proxy: 'Без прокси',
    profiles_last_launch: 'Последний запуск',
    profiles_btn_launch: 'Запустить',
    profiles_btn_stop: 'Стоп',
    profiles_btn_edit: 'Изменить',
    profiles_btn_clone: 'Клон',
    profile_section_privacy: 'Приватность',
    profile_field_ua: 'User-Agent (опционально)',
    profile_field_ua_placeholder: 'Оставьте пустым для использования пресета',
    profile_field_languages: 'Языки',
    proxy_col_ip: 'Последний IP',
    cancel: 'Отмена',
    proxy_btn_checking: 'Проверка…',
    proxy_last_ip: 'Последний IP:',
    // Settings page
    settings_title: 'Настройки',
    settings_group_general: 'Основное',
    settings_group_notes: 'Заметки',
    settings_group_security: 'Безопасность',
    settings_group_data: 'Данные',
    settings_group_about: 'О программе',
    settings_section_language: 'Язык',
    settings_language_label: 'Язык интерфейса',
    update_banner_available: 'Доступна версия {version}',
    update_banner_install: 'Установить',
    update_banner_later: 'Позже',
    settings_update_section: 'Обновления приложения',
    settings_update_check: 'Проверить обновления',
    settings_update_checking: 'Проверка…',
    settings_update_up_to_date: 'Установлена последняя версия.',
    settings_update_available: 'Доступно обновление: {version}',
    settings_update_install: 'Установить и перезапустить',
    settings_update_downloading: 'Загрузка обновления…',
    settings_update_installing: 'Установка… приложение перезапустится',
    settings_update_unsupported:
      'Автообновление недоступно для установок deb/rpm. Скачайте новую версию со страницы релизов.',
    settings_update_open_releases: 'Открыть страницу релизов',
    settings_update_failed: 'Не удалось проверить обновления. Проверьте соединение и попробуйте ещё раз.',
    settings_data_section: 'Демо и данные',
    settings_data_hint: 'Загрузить демо-контент для записи или очистить локальные данные без переустановки.',
    settings_demo_locale: 'Язык демо-контента',
    settings_demo_locale_en: 'English',
    settings_demo_locale_ru: 'Русский',
    settings_demo_load: 'Загрузить демо-данные',
    settings_demo_loading: 'Загрузка…',
    settings_demo_confirm: 'Данные этих разделов будут заменены демо-набором: {list}. Блокировка приложения станет паролем «demo»; её ключ восстановления не показывается — если оставите демо, создайте новый в «Настройки → Блокировка приложения». Продолжить?',
    settings_demo_done: 'Демо-данные загружены.',
    settings_clear_data: 'Очистить данные приложения',
    settings_clear_clearing: 'Очистка…',
    settings_clear_confirm: 'Удалить все данные этих разделов: {list}? Это нельзя отменить. Блокировка приложения и её пароль останутся прежними.',
    settings_clear_done: 'Данные приложения очищены.',
    dev_sync_link: 'Для разработчиков',
    dev_sync_title: 'Лог синхронизации',
    dev_sync_back: 'Настройки',
    dev_sync_enable: 'Лог синхронизации',
    dev_sync_enable_hint: 'Пишет шаги каждого цикла: старт, пропуск, повтор и ошибки. По умолчанию выключен.',
    dev_sync_clear: 'Очистить',
    dev_sync_copy: 'Копировать',
    dev_sync_copied: 'Скопировано',
    dev_sync_empty: 'Пока пусто. Включите лог и дождитесь цикла синхронизации.',
    settings_section_about: 'О программе',
    settings_about_version: 'Версия',
    settings_about_copyright: 'Copyright © 2026 Veydan Project',
    settings_about_license: 'Лицензия',
    settings_about_license_note:
      'Открытый исходный код (source-available): можно свободно читать, собирать, запускать, изменять и распространять — но нельзя использовать для создания продукта, конкурирующего с {app}. Это не OSI open source.',
    settings_about_link_license: 'Полный текст лицензии',
    settings_about_link_summary: 'Краткое описание (5 языков)',
    settings_about_link_thirdparty: 'Лицензии зависимостей',
    settings_about_repo: 'Исходный код',
    settings_about_link_repo: 'Открыть на GitHub',
    settings_bug_report: 'Сообщить об ошибке',
    settings_bug_hint: 'Опишите, что случилось, и отправьте текст на этот адрес.',
    settings_bug_copy: 'Скопировать',
    settings_bug_copied: 'Скопировано',
    settings_bug_mail: 'Открыть почту',
    app_name: '{app}',
    settings_bug_subject: 'Ошибка в {app}',
    settings_bug_os: 'ОС',
    settings_bug_language: 'Язык',
    settings_bug_date: 'Дата',
    settings_bug_what: 'Что случилось:',
    settings_bug_steps: 'Шаги:',
    settings_bug_expected: 'Ожидалось:',
    back_profiles: '← Профили',
    loading: 'Загрузка…',
    export_btn_json: 'Сохранить JSON',
    export_btn_zip: 'Сохранить ZIP',
    export_btn_json_loading: 'Экспорт…',
    export_btn_zip_loading: 'Сохранение…',
    // Password generator
    pwgen_btn_toggle_theme: 'Переключить тему',
    // TOTP
    totp_btn_close: 'Закрыть',
    totp_filter_workspace: 'Воркспейс',
    totp_field_tags: 'Теги',
    totp_profile_badge: 'Профиль',
    pw_field_profile: 'Профиль',
    pw_field_workspace: 'Воркспейс',
    pw_unlock_shared: 'Введите пароль блокировки, заданный на другом устройстве.',
    pw_locked: 'Разблокируйте, чтобы показать',
    pw_unlock: 'Разблокировать',
    pw_mismatch: 'Хранилище паролей нельзя открыть текущим паролем блокировки.',
    pw_reset: 'Сбросить хранилище паролей',
    pw_reset_confirm: 'Удалить все сохранённые пароли? Это нельзя отменить.',
    pw_err_locked: 'Хранилище заблокировано.',
    pw_err_mismatch: 'Хранилище паролей нельзя открыть текущим паролем блокировки.',
    pw_err_decrypt: 'Не удалось расшифровать запись.',
    pw_notes: 'Заметки',
    pw_new_note: 'Новая заметка',
    pw_note_search: 'Найти заметку…',
    notes_sync_never: 'ещё не было',
    note_sync_conflict_remote: 'Версия с другого устройства',
    note_sync_conflict_remote_short: 'Другое устройство',
    note_sync_conflict_mine: 'Моя версия',
    note_sync_conflict_keep: 'Оставить мою версию',
    note_sync_conflict_take: 'Взять версию с другого устройства',
    note_sync_conflict_review: 'Разобрать по строкам',
    note_sync_conflict_both: 'Обе',
    note_sync_conflict_all_mine: 'Всё моё',
    note_sync_conflict_all_theirs: 'Всё чужое',
    ctx_action_copy_url: 'Копировать URL',
    // Command palette
    cmd_title: 'Палитра команд',
    cmd_placeholder: 'Введите команду…',
    cmd_empty: 'Команды не найдены',
    cmd_app_settings: 'Открыть настройки',
    cmd_app_lock: 'Заблокировать {app}',
    // Attachment policy (settings)
    notes_areas_color: 'Выбрать цвет',
    notes_areas_placeholder: 'Название области',
    notes_areas_add: 'Добавить',
    notes_untitled: 'Без названия',
    notes_tags_add: 'Добавить тег',
    notes_tags_create: 'Создать "{name}"',
    notes_tag_rename: 'Переименовать',
    notes_tag_delete: 'Удалить',
    notes_tag_save: 'Сохранить',
    // Settings – notes storage
    // Settings – browser capture rules
    // Notes – quick capture
    settings_lock_section: 'Блокировка приложения',
    settings_lock_hint: 'ПИН или пароль нужен, чтобы открыть {app}. Хранится только Argon2-хэш.',
    settings_lock_status_off: 'Выкл',
    settings_lock_kind: 'Блокировать',
    settings_lock_current: 'Текущий ПИН или пароль',
    settings_lock_new_pin: 'Новый ПИН',
    settings_lock_new_password: 'Новый пароль',
    settings_lock_confirm_pin: 'Повторите ПИН',
    settings_lock_confirm_password: 'Повторите пароль',
    settings_lock_min_len: 'Не меньше 4 символов',
    settings_lock_pin_digits: 'Только цифры',
    settings_lock_hint_label: 'Подсказка (необязательно)',
    settings_lock_hint_note: 'Не пишите сюда сам ПИН или пароль.',
    settings_lock_hint_ph: 'Показывается после неверной попытки',
    settings_lock_warning: 'Без ПИН или пароля любой, у кого есть это устройство, откроет {app} и прочитает его данные. {app} не может сбросить забытый ПИН или пароль: сохраните ключ восстановления, который появится после включения блокировки, иначе данные под блокировкой будут потеряны навсегда.',
    settings_lock_enable: 'Включить блокировку',
    settings_lock_change: 'Сохранить изменения',
    settings_lock_disable: 'Отключить блокировку',
    settings_lock_timeout: 'Автоблокировка через',
    settings_lock_timeout_never: 'Никогда',
    settings_lock_timeout_min: '{n} мин',
    settings_lock_recovery: 'Ключ восстановления',
    settings_lock_recovery_has: 'Создан',
    settings_lock_recovery_none: 'Не создан',
    settings_lock_recovery_new: 'Создать новый ключ',
    settings_lock_recovery_new_hint: 'Введите текущий ПИН или пароль. Прежний ключ перестанет работать.',
    lock_kind_pin: 'ПИН',
    lock_kind_password: 'Пароль',
    lock_title_pin: 'Введите ПИН',
    lock_title_password: 'Введите пароль',
    lock_unlock: 'Разблокировать',
    lock_wrong: 'Неверный ПИН или пароль',
    lock_hint_prefix: 'Подсказка: {hint}',
    lock_forgot: 'Забыли ПИН или пароль?',
    lock_recovery_title: 'Ваш ключ восстановления',
    lock_recovery_file_title: 'Ключ восстановления {app}',
    lock_recovery_intro: 'Показывается один раз. С этим ключом можно задать новый ПИН или пароль, не потеряв данные под блокировкой. Храните его в надёжном месте вне {app}.',
    lock_recovery_copy: 'Копировать ключ',
    lock_recovery_copied: 'Скопировано',
    lock_recovery_save_file: 'Сохранить в файл',
    lock_recovery_share: 'Поделиться',
    lock_recovery_ack: 'Я сохранил ключ в надёжном месте',
    lock_recovery_done: 'Готово',
    lock_recovery_invalid: 'Ключ восстановления не подходит.',
    lock_recover_title: 'Восстановить доступ',
    lock_recover_step: 'Шаг {n} из 3',
    lock_recover_step1: 'Введите ключ восстановления',
    lock_recover_step1_hint: 'Ключ из 24 символов, который вы сохранили при включении блокировки. Регистр и дефисы не важны.',
    lock_recover_paste: 'Вставить',
    lock_recover_continue: 'Продолжить',
    lock_recover_step2: 'Задайте новый ПИН или пароль',
    lock_recover_step3: 'Сохраните новый ключ восстановления',
    lock_recover_step3_hint: 'Ключ, который вы только что ввели, больше не работает.',
    lock_recover_none: 'Для этой блокировки нет ключа восстановления. Вернуть доступ можно только очисткой данных {app} на этом устройстве.',
    lock_recover_back: 'Назад ко входу',
    // Settings – backup
    settings_backup_browse: 'Обзор',
    settings_backup_save: 'Сохранить',
    settings_backup_never: 'Никогда',
    settings_backup_cancel: 'Отмена',
    // Settings – sync (beta)
    settings_sync_section: 'Синхронизация',
    settings_sync_beta: 'Бета',
    settings_sync_hint: 'Синхронизация данных между устройствами через сквозное шифрованное хранилище. Хранилище видит только шифртекст: папка, которую синкает любой облачный клиент, S3-совместимый бакет или WebDAV.',
    settings_sync_backend: 'Хранилище',
    settings_sync_backend_folder: 'Папка',
    settings_sync_backend_s3: 'S3-совместимое',
    settings_sync_backend_webdav: 'WebDAV',
    settings_sync_folder: 'Папка сейфа',
    settings_sync_folder_placeholder: 'Выберите папку, которую синхронизирует облачный клиент',
    settings_sync_folder_hint: 'Путь может отличаться на разных устройствах: сейф определяется манифестом, а не путём.',
    settings_sync_s3_endpoint: 'Адрес (endpoint)',
    settings_sync_s3_region: 'Регион',
    settings_sync_s3_bucket: 'Бакет',
    settings_sync_s3_prefix: 'Префикс',
    settings_sync_s3_access_key: 'Ключ доступа',
    settings_sync_s3_secret_key: 'Секретный ключ',
    settings_sync_s3_path_style: 'Path-style адресация',
    settings_sync_s3_path_style_hint: 'endpoint/bucket/key — нужно для MinIO и большинства self-hosted серверов.',
    settings_sync_webdav_url: 'URL коллекции',
    settings_sync_webdav_username: 'Пользователь',
    settings_sync_webdav_password: 'Пароль',
    settings_sync_interval: 'Интервал опроса',
    settings_sync_unit_seconds: 'секунды',
    settings_sync_unit_minutes: 'минуты',
    settings_sync_interval_presets: 'Быстро:',
    settings_sync_interval_remote_warn: 'Интервал меньше минуты предназначен для тестов: на S3/WebDAV это частые запросы и возможные расходы.',
    settings_sync_profile_files: 'Синхронизировать файлы профилей браузера',
    settings_sync_profile_files_hint: 'Cookies, сессии и история каждого профиля. Выкл.: настройки всё равно едут, занятый профиль виден на других устройствах.',
    settings_sync_device_name: 'Имя устройства',
    settings_sync_device_name_hint: 'Показывается на других ваших устройствах.',
    settings_sync_device_name_profiles_hint: 'Показывается на других устройствах, пока профиль запущен здесь.',
    settings_sync_interval_sec: '{n} с',
    settings_sync_interval_min: '{n} мин',
    settings_sync_profile_conflicts: 'Файлы профилей изменены с обеих сторон',
    settings_sync_gc: 'Блобы хранилища',
    settings_sync_gc_value: '{total} хранится, {removed} удалено при последней очистке {when}',
    settings_sync_gc_hint: 'Неиспользуемые блобы удаляются раз в сутки после 24-часовой отсрочки.',
    settings_sync_lf_gc: 'Объекты больших файлов',
    settings_sync_lf_section: 'Передача больших файлов',
    settings_sync_lf_hint: 'Только это устройство. Действует для новых файлов; уже загруженные сохраняют свой размер чанка.',
    settings_sync_lf_chunk: 'Размер чанка, МиБ',
    settings_sync_lf_chunk_hint: '{min}–{max}. Больше чанк — меньше запросов, больше памяти.',
    settings_sync_lf_parallelism: 'Параллельных чанков',
    settings_sync_lf_parallelism_hint: '{min}–{max} чанков одновременно.',
    settings_sync_lf_resume: 'Возобновлять прерванные передачи',
    settings_sync_lf_resume_hint: 'Хранит частичные загрузки на диске и продолжает с последнего целого чанка.',
    settings_sync_lf_ram: 'Оценка пиковой памяти на передачу: около {mib} МиБ.',
    settings_sync_lf_chunk_warn: 'Смена размера чанка не перекодирует файлы, уже лежащие в сейфе.',
    settings_sync_phase_attachments_up: 'Отправка вложений',
    settings_sync_phase_attachments_down: 'Загрузка вложений',
    profile_sync_diverged_hint: 'Профиль запускали на двух устройствах. Выберите, какие файлы оставить; другая версия будет заменена.',
    settings_sync_enabled: 'Фоновая синхронизация',
    settings_sync_enabled_hint: 'Опрашивать хранилище с заданным интервалом. Выключение приостанавливает синк, не покидая сейф.',
    settings_sync_probe: 'Проверить хранилище',
    settings_sync_probe_empty: 'Хранилище пустое — можно создать новый сейф.',
    settings_sync_probe_vault: 'Найден сейф — подключитесь к нему с его парольной фразой.',
    settings_sync_probe_foreign: 'Хранилище не пустое и сейфа в нём нет. Выберите пустую папку.',
    settings_sync_passphrase: 'Парольная фраза сейфа',
    settings_sync_passphrase_placeholder: 'Не менее 8 символов',
    settings_sync_passphrase_hint: '«Создать» делает новый сейф в пустом хранилище. «Подключиться» открывает уже существующий. Другие устройства вводят ту же фразу.',
    settings_sync_create: 'Создать сейф',
    settings_sync_join: 'Подключиться',
    settings_sync_created: 'Сейф создан. Синхронизация включена.',
    settings_sync_joined: 'Подключено к сейфу. Синхронизация включена.',
    settings_sync_join_own_lock: 'Пароли или ключ модуля «Чат» на этом устройстве закрыты его нынешней блокировкой: заданной здесь или блокировкой сейфа, к которому устройство было подключено раньше. Отключите блокировку на этом устройстве и подключитесь к сейфу снова.',
    settings_sync_now: 'Синхронизировать',
    settings_sync_running: 'Синхронизация…',
    settings_sync_done: 'Синхронизация завершена.',
    settings_sync_phase_collect: 'Сбор изменений',
    settings_sync_phase_push: 'Отправка',
    settings_sync_phase_pull: 'Загрузка',
    settings_sync_phase_apply: 'Применение',
    settings_sync_phase_profiles_up: 'Отправка файлов профилей',
    settings_sync_phase_profiles_down: 'Загрузка файлов профилей',
    settings_sync_phase_compact: 'Сжатие лога',
    settings_sync_phase_gc: 'Очистка неиспользуемых файлов',
    settings_sync_phase_done: 'Готово',
    settings_sync_phase_error: 'Ошибка',
    settings_sync_progress: '{phase} · {percent}%',
    settings_sync_progress_files: '{phase} · {current}/{total} · {percent}%',
    settings_sync_leave: 'Покинуть сейф',
    settings_sync_leave_confirm: 'Устройство забудет ключ сейфа и позицию синка. В хранилище и в локальных заметках ничего не удаляется.',
    settings_sync_left: 'Сейф покинут.',
    settings_sync_change_passphrase: 'Сменить парольную фразу',
    settings_sync_change_passphrase_hint:
      'Меняется только парольная фраза, мастер-ключ хранилища остаётся прежним. Старая копия manifest.json по-прежнему открывается старой фразой. Если она могла утечь — создайте новое хранилище.',
    settings_sync_passphrase_old: 'Текущая фраза',
    settings_sync_passphrase_new: 'Новая фраза',
    settings_sync_passphrase_changed: 'Фраза изменена. Подключённые устройства продолжают работать, новые используют новую фразу.',
    settings_sync_status: 'Статус',
    settings_sync_status_joined: 'подключено',
    settings_sync_status_not_joined: 'не подключено',
    settings_sync_vault_id: 'Сейф',
    settings_sync_device_id: 'Это устройство',
    settings_sync_peers: 'Другие устройства',
    settings_sync_storage_devices: 'Устройства в хранилище',
    settings_sync_device_unnamed: 'Без имени',
    settings_sync_last_applied: 'Получено за цикл',
    settings_sync_last_run: 'Последний синк',
    settings_sync_last_error: 'Последняя ошибка',
    settings_sync_last_warning: 'Предупреждение',
    settings_sync_conflicts: 'Заметки с конфликтами',
    settings_sync_conflicts_hint: 'Оба устройства правили одни и те же строки. Ваш текст сохранён; откройте заметку и выберите, какую версию оставить.',
    // SSH
    ssh_scope_all: 'Все',
    ssh_scope_global: 'Глобальные',
    ssh_scope_workspace: 'Воркспейс',
    ssh_scope_profile: 'Профиль',
    ssh_field_scope: 'Область',
    ssh_totp_prompt: 'Введите TOTP-код для',
    ssh_totp_placeholder: '123456',
    ssh_btn_connect: 'Подключить',
    ssh_open_panel: 'Открыть SSH панель',
    ssh_profile_no_linked: 'Нет привязанных SSH-подключений',
    secret_stored_placeholder: 'Сохранено — оставьте пустым, чтобы не менять',
    secret_clear: 'Удалить сохранённое значение',
    terminal_col_status: 'Статус',
    // UI Inspector / Design QA
    hotkey_section: 'Горячие клавиши',
    hotkey_hint: 'Сочетание срабатывает по позиции клавиши, а не по букве текущей раскладки.',
    hotkey_group_edit: 'Правка текста',
    hotkey_group_editor: 'Редактор',
    hotkey_group_app: 'Приложение',
    hotkey_edit_undo: 'Отменить',
    hotkey_edit_redo: 'Повторить',
    hotkey_editor_bold: 'Жирный',
    hotkey_editor_italic: 'Курсив',
    hotkey_editor_link: 'Ссылка',
    hotkey_editor_find: 'Найти',
    hotkey_editor_save: 'Сохранить',
    hotkey_app_palette: 'Палитра команд',
    hotkey_app_inspector: 'Инспектор интерфейса',
    hotkey_change: 'Изменить',
    hotkey_reset: 'Сбросить',
    hotkey_record: 'Нажмите сочетание…',
    hotkey_unbound: 'Не задано',
    hotkey_need_modifier: 'Нужен Ctrl, Alt или Cmd',
    hotkey_conflict: 'Уже занято: {name}',
    inspector_toggle: 'UI Инспектор',
    inspector_grid: 'Сетка',
    inspector_grid_off: 'Выкл',
    inspector_zoom: 'Масштаб',
    inspector_rulers: 'Линейки',
    inspector_guides: 'Направляющие',
    inspector_outline: 'Обводка всего',
    inspector_inspect: 'Инспект',
    inspector_screenshot: 'Скриншот',
    inspector_presets: 'Пресеты',
    inspector_preset_save: 'Сохранить',
    inspector_preset_delete: 'Удалить',
    inspector_preset_name: 'Название пресета',
    inspector_custom_grid: 'Кастомная сетка',
    inspector_columns: 'Колонки',
    inspector_gutter: 'Отступ',
    inspector_margin: 'Поля',
    inspector_viewport: 'Вьюпорт',
    inspector_hotkey_hint: 'Включить через {keys}',
    inspector_developer_tools: 'Инструменты разработчика',
    // System tray
    settings_tray_section: 'Системный трей',
    settings_tray_minimize: 'Сворачивать в трей',
    settings_tray_minimize_hint: 'При сворачивании окна прятать его в системный трей вместо панели задач.',
    settings_tray_close: 'Закрывать в трей',
    settings_tray_close_hint: 'Закрытие окна (×) прячет его в трей, а не завершает приложение.',
    settings_tray_start_hidden: 'Запускать свёрнутым в трей',
    settings_tray_start_hidden_hint: 'Запускать приложение сразу в трее, не показывая окно.',
    tray_show: 'Показать {app}',
    tray_hide: 'Скрыть окно',
    tray_quit: 'Выход',
    tray_tooltip: '{app} — запущено: {n}',
    // Start error screen (degraded start, shown instead of the shell)
    start_error_title: 'Приложение не запустилось',
    start_error_db_foreign: 'Файл данных {path} создан не {app} и не будет открыт.',
    start_error_db_dev_schema: 'Файл данных {path} создан другой сборкой {app} и не будет открыт.',
    start_error_untouched: 'Ничего не изменено: файл остался таким, каким был.',
    start_error_fix:
      'Закройте приложение, переместите этот файл в другую папку вместе со всеми файлами рядом с ним, имена которых начинаются с app.db-, и запустите приложение снова. Будет создан новый файл данных.',
    start_error_close: 'Закрыть',
    start_error_no_answer: 'Приложение не ответило окну. Ничего не изменено; попробуйте ещё раз.',
    start_error_retry: 'Повторить',
    start_error_wrong_ui_phone: 'Этому окну достался телефонный интерфейс {app}: приложение собрано со страницами сборки для Android.',
    start_error_wrong_ui_desktop: 'Этому устройству достался десктопный интерфейс {app}: приложение собрано со страницами десктопной сборки.',
    start_error_wrong_ui_fix: 'Ничего не изменено. Соберите приложение заново или установите релизную сборку.',
    // A screen that failed to show, and the address without a page
    screen_failed: 'Не удалось показать этот экран.',
    screen_to_start: 'На стартовый экран',
    // The switches of the modules (Space)
    modules_section: 'Модули',
    modules_hint: 'Что показывает это устройство. Выключенный модуль сохраняет свои данные, и они продолжают синхронизироваться.',
    modules_last: 'Хотя бы один модуль остаётся включённым.',
    modules_first_title: 'Чем вы будете пользоваться?',
    modules_first_hint: 'Выберите, что будет на этом устройстве. Изменить выбор можно в любой момент: Настройки → Модули.',
    modules_first_all: 'Всё сразу',
    modules_first_continue: 'Продолжить',
    settings_lock_recovery_new_title: 'Новый ключ восстановления',
  },
} as const;

export type TranslationKey = keyof typeof translations.en;

type Dict = Record<string, string>;

/** A language's file beside a dictionary (`locales/<code>.json`): the desktop keys and the phone layer. */
export type LocaleFile = { desktop: Dict; mobile: Dict };

/**
 * The language of the system, when the user has not chosen one: the first of
 * `navigator.languages` the UI has, by its full tag (`pt-BR`) or by its
 * language (`de-AT` → `de`, `pt-PT` → `pt-BR`). Chinese other than the
 * simplified script stays out: Traditional readers get English, not a script
 * they did not ask for.
 */
export function systemLocale(tags: readonly string[]): Locale {
  for (const raw of tags) {
    const tag = raw.replace('_', '-').toLowerCase();
    const exact = LANGUAGES.find((l) => l.code.toLowerCase() === tag);
    if (exact) return exact.code;
    const [lang, ...rest] = tag.split('-');
    if (lang === 'zh') {
      if (rest.some((r) => r === 'tw' || r === 'hk' || r === 'mo' || r === 'hant')) continue;
      return 'zh-CN';
    }
    // `in` is the old code of Indonesian that Java and older Android still report.
    const base = lang === 'in' ? 'id' : lang;
    const same = LANGUAGES.find((l) => l.code.split('-')[0].toLowerCase() === base);
    if (same) return same.code;
  }
  return 'en';
}

const STORAGE_KEY = 'vb_locale';

function loadLocale(): Locale {
  if (typeof localStorage !== 'undefined') {
    const saved = localStorage.getItem(STORAGE_KEY);
    if (isLocale(saved)) return saved;
  }
  if (typeof navigator !== 'undefined') {
    return systemLocale(navigator.languages?.length ? navigator.languages : [navigator.language ?? 'en']);
  }
  return 'en';
}

/** Whether the user (or another device, through sync) has chosen the language; a guess from the system is not a choice. */
export function localeChosen(): boolean {
  try {
    return isLocale(localStorage.getItem(STORAGE_KEY));
  } catch {
    return false;
  }
}

// The files of each language, fetched when the language is first shown: the
// core's here, the modules' through the virtual module (their entry/i18n.ts).
const coreLocales = import.meta.glob<LocaleFile>('./locales/*.json', { import: 'default' });
const loaded = writable<Record<Locale, LocaleFile>>({});
const loading = new Map<Locale, Promise<void>>();

/** The languages fetched so far besides English and Russian (the phone layer reads them too). */
export const loadedLocales: Readable<Record<Locale, LocaleFile>> = { subscribe: loaded.subscribe };

/**
 * Fetch a language's files once. Until they are here `t()` shows English; the
 * start of the page waits for the language it opens with
 * (routes/+layout.svelte), so only a switch in Settings may show English for
 * a moment.
 */
export function loadLocaleFiles(code: Locale): Promise<void> {
  if (BUILT_IN.includes(code) || !isLocale(code) || get(loaded)[code]) return Promise.resolve();
  let pending = loading.get(code);
  if (!pending) {
    const name = `/${code}.json`;
    const loaders = [coreLocales, ...moduleLocales].flatMap((files) =>
      Object.entries(files).flatMap(([file, load]) => (file.endsWith(name) ? [load] : [])),
    );
    pending = Promise.all(loaders.map((load) => load()))
      .then((files) => {
        const desktop: Dict = {};
        const mobile: Dict = {};
        for (const f of files) {
          Object.assign(desktop, f.desktop);
          Object.assign(mobile, f.mobile);
        }
        loaded.update((all) => ({ ...all, [code]: { desktop, mobile } }));
      })
      .catch((e) => {
        loading.delete(code);
        console.error(`the files of the language ${code} did not load`, e);
      });
    loading.set(code, pending);
  }
  return pending;
}

const current = writable<Locale>(loadLocale());

/**
 * The language of the UI. Setting it remembers the choice on this device and
 * fetches the language's files; the backend's copy (`ui_locale`, which syncs
 * between computers) is written by the language picker alone.
 */
export const locale = {
  subscribe: current.subscribe,
  set(value: Locale) {
    const code = isLocale(value) ? value : 'en';
    try {
      localStorage.setItem(STORAGE_KEY, code);
    } catch {}
    void loadLocaleFiles(code);
    current.set(code);
  },
};

current.subscribe((code) => {
  if (typeof document === 'undefined') return;
  document.documentElement.lang = localeTag(code);
});

/** The text of a key in a language: the language's own, else English, else the key. */
function lookup(all: Record<Locale, LocaleFile>, code: Locale, key: string): string {
  const own = BUILT_IN.includes(code) ? (translations as unknown as Record<Locale, Dict>)[code] : all[code]?.desktop;
  return own?.[key] ?? (translations.en as Dict)[key] ?? key;
}

export const t = derived([current, loaded], ([$locale, $loaded]) => {
  return (key: TranslationKey, vars?: Record<string, string>): string => {
    let text = lookup($loaded, $locale, key);
    if (vars) {
      for (const [k, v] of Object.entries(vars)) {
        // split/join replaces ALL occurrences without regex-escaping concerns
        text = text.split(`{${k}}`).join(v);
      }
    }
    // The name of the product (13.5): a string names it as `{app}`, filled
    // here for every caller, the modules' included.
    return text.split('{app}').join(product.name);
  };
});

/** A key whose count has plural forms: `<key>_one` and `<key>_few` exist beside it. */
export type CountKey = {
  [K in TranslationKey]: `${K}_one` extends TranslationKey ? (`${K}_few` extends TranslationKey ? K : never) : never;
}[TranslationKey];

const pluralRules: Partial<Record<Locale, Intl.PluralRules>> = {};

/**
 * The key of a count's plural form (CLDR, Intl.PluralRules): `<key>_one`
 * (English 1; Russian, Ukrainian 1, 21, 31…; Polish 1 alone), `<key>_few`
 * (Russian, Ukrainian, Polish 2–4, 22–24…) or `<key>` itself for the rest
 * ("many" and "other"). A language without "few" repeats the plain form in
 * `_few`; one without "one" (Chinese, Japanese…) never reads `_one`.
 */
export function countKey<K extends CountKey>(key: K, n: number, loc: Locale): K | `${K}_one` | `${K}_few` {
  const form = (pluralRules[loc] ??= new Intl.PluralRules(localeTag(loc))).select(n);
  if (form === 'one') return `${key}_one`;
  if (form === 'few') return `${key}_few`;
  return key;
}

/** The product's tagline (About) in a language, English where products.json has none. */
export function taglineOf(code: Locale): string {
  const tagline = product.tagline as Record<string, string>;
  return tagline[code] ?? tagline.en;
}

/** The merged dictionary as `t()` reads it (the test compares it with the dictionary before the split). */
export { translations as dictionary };
