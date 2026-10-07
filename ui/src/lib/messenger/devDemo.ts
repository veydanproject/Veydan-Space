// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Demo data for the browser preview (`pnpm dev` without Tauri). Enabled by
// `localStorage['messenger.demo'] = '1'`. Never used inside the app.

import type {
  CardView, ChatMode, Color, MessageStatus, MessengerChat, MessengerContact, MessengerGroup, MessengerGroupInvite, MessengerGroupMember, MessengerIdentity,
  MessengerMessage, MessengerProfile, MessengerTransfer, SocialLink, SocialView, Span, Style,
} from './api';

const ME = 'ab'.repeat(32);
const hex = (c: string) => c.repeat(64).slice(0, 64);
const npub = (pk: string, tag: string) => `npub1${tag}${pk}`.slice(0, 63);

function profile(pk: string, name: string, about: string, nip05: string | null, extra: Partial<MessengerProfile> = {}): MessengerProfile {
  return {
    pubkey: pk, npub: npub(pk, name.toLowerCase().replace(/[^a-z]/g, '')), name: name.toLowerCase(), display_name: name, about: demoStrip(about) || null,
    picture: null, banner: null, website: null, nip05, lud16: null, nip05_verified: !!nip05, event_created_at: 1, fetched_at: 1,
    bio: demoBioParse(about), bio_source: about || null, socials: [], ...extra,
  };
}

// ── What the runtime does in the app, played for the browser preview ──────
// Rough on purpose: the real parser, checks and pictures are in Rust.

const COLORS: readonly Color[] = ['red', 'orange', 'yellow', 'green', 'teal', 'blue', 'purple', 'pink', 'gray'];

/** A bio's marks as spans: `**b**`, `*i*`, `~~s~~`, `\`code\``, `{red}…{/}`, bare https links. */
export function demoBioParse(markup: string): Span[] {
  const src = markup.replace(/\r\n?/g, '\n').trim().slice(0, 2000);
  const out: Span[] = [];
  const st: Style = { bold: false, italic: false, strike: false, code: false, color: null };
  let buf = '';
  const flush = () => {
    if (!buf) return;
    const style = { ...st };
    let last = 0;
    for (const m of buf.matchAll(/https:\/\/[^\s<>"]+/g)) {
      const url = m[0].replace(/[.,;:!?)\]}'"]+$/, '');
      if (m.index! > last) out.push({ kind: 'text', text: buf.slice(last, m.index), style });
      out.push({ kind: 'link', url, text: url, style });
      last = m.index! + url.length;
    }
    if (last < buf.length) out.push({ kind: 'text', text: buf.slice(last), style });
    buf = '';
  };
  const closes = (mark: string, from: number) => src.indexOf(mark, from) > 0;
  for (let i = 0; i < src.length; i++) {
    const ch = src[i];
    if (ch === '\n') { flush(); out.push({ kind: 'break' }); continue; }
    if (st.code) {
      if (ch === '`') { flush(); st.code = false; } else buf += ch;
      continue;
    }
    if (ch === '\\' && i + 1 < src.length) { buf += src[++i]; continue; }
    if (ch === '`' && closes('`', i + 1)) { flush(); st.code = true; continue; }
    const two = src.slice(i, i + 2);
    if (two === '**' || two === '~~') {
      const key = two === '**' ? 'bold' : 'strike';
      if (st[key] || closes(two, i + 2)) { flush(); st[key] = !st[key]; i++; continue; }
    }
    if (ch === '*' && (st.italic || closes('*', i + 1))) { flush(); st.italic = !st.italic; continue; }
    if (ch === '{') {
      if (st.color && src.startsWith('{/}', i)) { flush(); st.color = null; i += 2; continue; }
      const name = /^\{([a-z]+)\}/.exec(src.slice(i))?.[1] as Color | undefined;
      if (!st.color && name && COLORS.includes(name) && closes('{/}', i)) { flush(); st.color = name; i += name.length + 1; continue; }
    }
    buf += ch;
  }
  flush();
  return out;
}

/** The bio without its marks: what other clients get as `about`. */
export function demoStrip(markup: string): string {
  return demoBioParse(markup).map((s) => (s.kind === 'break' ? '\n' : s.text)).join('');
}

const SOCIAL: Record<string, [name: string, prefix: string, url: (h: string) => string]> = {
  telegram: ['Telegram', '@', (h) => `https://t.me/${h}`],
  instagram: ['Instagram', '@', (h) => `https://instagram.com/${h}`],
  tiktok: ['TikTok', '@', (h) => `https://tiktok.com/@${h}`],
  x: ['X', '@', (h) => `https://x.com/${h}`],
  youtube: ['YouTube', '@', (h) => `https://youtube.com/@${h}`],
  vk: ['VK', '', (h) => `https://vk.com/${h}`],
  facebook: ['Facebook', '', (h) => `https://facebook.com/${h}`],
  linkedin: ['LinkedIn', '', (h) => `https://linkedin.com/in/${h}`],
  github: ['GitHub', '', (h) => `https://github.com/${h}`],
  whatsapp: ['WhatsApp', '+', (h) => `https://wa.me/${h}`],
  discord: ['Discord', '', () => ''],
  twitch: ['Twitch', '', (h) => `https://twitch.tv/${h}`],
  mastodon: ['Mastodon', '@', (h) => { const [u, host] = h.split('@'); return `https://${host}/@${u}`; }],
  bluesky: ['Bluesky', '@', (h) => `https://bsky.app/profile/${h}`],
  threads: ['Threads', '@', (h) => `https://threads.net/@${h}`],
  reddit: ['Reddit', 'u/', (h) => `https://reddit.com/user/${h}`],
  other: ['Link', '', (h) => h],
};

/** The platforms a link can be of, as the runtime lists them. */
export const DEMO_PLATFORMS = Object.entries(SOCIAL).map(([id, [name, prefix]]) => ({
  id, name,
  hint: id === 'other' ? 'https://…' : id === 'whatsapp' ? '+1 555 123 4567' : id === 'mastodon' ? '@user@mastodon.social' : id === 'bluesky' ? '@name.bsky.social' : `${prefix}username`,
}));

/** A typed link in its stored form; throws the runtime's codes. */
export function demoSocialLink(link: SocialLink): SocialLink {
  if (!SOCIAL[link.p]) throw new Error('social_unknown_platform');
  let h = link.h.trim();
  if (link.p === 'other') {
    if (!/^https:\/\/[^\s/@]+\.[^\s/@]+(\/\S*)?$/.test(h) || h.length > 512) throw new Error('social_bad_handle');
    return { p: link.p, h };
  }
  if (h.includes('/')) h = h.replace(/\/+$/, '').split('/').pop() ?? '';
  h = h.replace(/^(@|u\/)/, '');
  if (link.p === 'whatsapp') h = h.replace(/[\s().-]/g, '').replace(/^\+/, '');
  if (link.p === 'bluesky' && !h.includes('.')) h = `${h}.bsky.social`;
  const ok = link.p === 'whatsapp' ? /^\d{7,15}$/ : link.p === 'mastodon' ? /^[A-Za-z0-9_]{1,30}@[a-z0-9.-]+\.[a-z]{2,}$/ : /^[A-Za-z0-9._-]{1,64}$/;
  if (!ok.test(h)) throw new Error('social_bad_handle');
  return { p: link.p, h };
}

/** A stored link as the UI shows it. */
export function demoSocialView(link: SocialLink): SocialView {
  const [name, prefix, url] = SOCIAL[link.p] ?? SOCIAL.other;
  return { platform: link.p, name, handle: link.p === 'other' ? link.h.replace(/^https:\/\//, '') : `${prefix}${link.h}`, url: url(link.h) };
}

/** `+` and 7..15 digits, as typed with spaces, dashes, dots or brackets; throws `phone_invalid`. */
export function demoPhone(input: string): string | null {
  const s = input.trim();
  if (!s) return null;
  const digits = s.replace(/[\s().-]/g, '');
  if (!/^\+\d{7,15}$/.test(digits)) throw new Error('phone_invalid');
  return digits;
}

/**
 * A picture for the preview: a JPEG `data:` URL drawn on a canvas, the
 * colours picked by `seed`. Null where there is no canvas (tests).
 */
export function demoPicture(seed: string, w = 256, h = w): string | null {
  try {
    if (typeof document === 'undefined') return null;
    const c = document.createElement('canvas');
    c.width = w; c.height = h;
    const g = c.getContext('2d');
    if (!g) return null;
    let x = 0;
    for (let i = 0; i < seed.length; i++) x = (x * 31 + seed.charCodeAt(i)) >>> 0;
    const hue = x % 360;
    const grad = g.createLinearGradient(0, 0, w, h);
    grad.addColorStop(0, `hsl(${hue} 70% 55%)`);
    grad.addColorStop(1, `hsl(${(hue + 70) % 360} 65% 35%)`);
    g.fillStyle = grad;
    g.fillRect(0, 0, w, h);
    for (let i = 0; i < 5; i++) {
      x = (x * 1103515245 + 12345) >>> 0;
      g.fillStyle = `hsla(${(hue + i * 47) % 360} 80% ${60 + (i % 3) * 10}% / 0.55)`;
      g.beginPath();
      g.arc((x % w), ((x >>> 8) % h), Math.min(w, h) * (0.12 + (i % 3) * 0.08), 0, Math.PI * 2);
      g.fill();
    }
    return c.toDataURL('image/jpeg', 0.85);
  } catch {
    return null;
  }
}

/** A card as the runtime shows it, from a profile of the demo. */
export function demoCard(p: MessengerProfile, phone: string | null, flags: { is_me?: boolean; is_contact?: boolean; blocked?: boolean } = {}): CardView {
  return {
    pubkey: p.pubkey, npub: p.npub, label: p.display_name?.trim() || p.name?.trim() || `${p.npub.slice(0, 12)}…`,
    name: p.name, display_name: p.display_name, bio: p.bio, website: p.website, socials: p.socials, phone,
    avatar: p.picture ? demoPicture(p.picture, 160) : null,
    is_me: flags.is_me ?? false, is_contact: flags.is_contact ?? false, blocked: flags.blocked ?? false,
  };
}

/** Pictures of the demo's people: these URLs "are cached" in the preview. */
export const DEMO_PICTURE_BASE = 'https://node-1.veydan.net/media/';

export interface DemoData {
  identity: MessengerIdentity;
  ownProfile: MessengerProfile;
  contacts: MessengerContact[];
  chats: MessengerChat[];
  messages: Record<string, MessengerMessage[]>;
  groups: MessengerGroup[];
  invites: MessengerGroupInvite[];
  /** Transfers of the files of the demo, in the states they show. */
  transfers: MessengerTransfer[];
}

export function demoEnabled(): boolean {
  try { return typeof localStorage !== 'undefined' && localStorage.getItem('messenger.demo') === '1'; }
  catch { return false; }
}

export function buildDemo(): DemoData {
  const now = Math.floor(Date.now() / 1000);
  const people: [string, string, string, string | null, ChatMode, boolean][] = [
    [hex('1a'), 'Алиса Морозова', '**Дизайнер.** Пишу {purple}редко{/}, но *по делу*.\nПортфолио: https://example.com/alice', 'alice@veydan.net', 'full_chat', true],
    [hex('2b'), 'Борис', 'Бэкенд, {teal}реле{/}, инфраструктура. `wss://node-1.veydan.net`', null, 'full_chat', true],
    [hex('3c'), 'Вера К.', '', null, 'request_received', false],
    [hex('4d'), 'Глеб', '', null, 'request_sent', false],
    [hex('5e'), 'Спам-бот', '', null, 'blocked', false],
    [hex('6f'), 'Дарья', 'На связи по будням.', 'daria@example.org', 'removed_by_peer', false],
  ];
  const contacts: MessengerContact[] = [];
  const chats: MessengerChat[] = [];
  const messages: Record<string, MessengerMessage[]> = {};
  let n = 0;
  const msg = (chat: string, from: string, at: number, text: string | null, extra: Partial<MessengerMessage> = {}): MessengerMessage => ({
    id: `demo${(n++).toString(16).padStart(60, '0')}`, chat_id: chat, direction: from === ME ? 'out' : 'in',
    status: from === ME ? 'sent' : 'received', content_type: 'text', text, sender_pubkey: from, reply_to: null,
    created_at: at, edited_at: null, deleted: false, failure_reason: null, delivered_at: null, read_at: null, seen_by: [], reactions: [], media: null, ...extra,
  });

  const looks: Record<string, Partial<MessengerProfile>> = {
    [hex('1a')]: {
      picture: `${DEMO_PICTURE_BASE}${'a1'.repeat(32)}`, website: 'https://example.com/alice',
      socials: [{ p: 'telegram', h: 'alice_m' }, { p: 'instagram', h: 'alice.designs' }, { p: 'github', h: 'alicem' }].map(demoSocialView),
    },
    [hex('2b')]: { picture: `${DEMO_PICTURE_BASE}${'b2'.repeat(32)}`, socials: [{ p: 'github', h: 'boris-dev' }, { p: 'mastodon', h: 'boris@mastodon.social' }].map(demoSocialView) },
    // Never "fetched": the initials stay.
    [hex('6f')]: { picture: 'https://example.org/missing/daria.jpg' },
  };
  for (const [pk, name, about, nip05, mode, canSend] of people) {
    const p = profile(pk, name, about, nip05, looks[pk]);
    const isContact = mode === 'full_chat' || mode === 'request_sent' || mode === 'removed_by_peer';
    if (isContact) {
      contacts.push({ pubkey: pk, npub: p.npub, nickname: null, note: null, followed: name === 'Алиса Морозова', profile: p, created_at: now - 86400 * 9, updated_at: now - 3600 });
    }
    const id = `dm:${pk}`;
    chats.push({ id, kind: 'dm', peer_pubkey: pk, peer_npub: p.npub, title: name, picture: p.picture, is_contact: isContact, is_muted: name === 'Борис', unread: 0, last_message_at: null, last_preview: null, pinned: name === 'Алиса Морозова', archived: false, mode, can_send: canSend });
    messages[id] = [];
  }

  const day = 86400;
  const [alice, boris, vera, gleb, bot, daria] = people.map((p) => p[0]);
  const a = `dm:${alice}`;
  const m1 = msg(a, alice, now - day * 2 - 4000, 'Привет! Посмотрела макеты мессенджера. В целом очень нравится, но есть пара мыслей по списку чатов.');
  const m2 = msg(a, ME, now - day * 2 - 3900, 'Привет! Давай, рассказывай.');
  messages[a].push(
    msg(a, '', now - day * 2 - 4100, 'request_accepted', { content_type: 'system', direction: 'out' }),
    m1, m2,
    msg(a, alice, now - day * 2 - 3800, 'Первое: время последнего сообщения лучше прижать вправо. Второе: счётчик непрочитанных у заглушённых чатов сделать серым, а не акцентным.'),
    msg(a, alice, now - day * 2 - 3790, 'И ещё: закреплённые чаты сверху.', { reactions: [{ emoji: '👍', count: 2, mine: true }, { emoji: '🔥', count: 1, mine: false }] }),
    msg(a, ME, now - day * 2 - 3600, 'Согласен по всем трём пунктам. Сделаю сегодня.', { reply_to: { id: m1.id, sender_pubkey: alice, text: m1.text } }),
    msg(a, ME, now - day - 7200, 'Готово, обновил. Посмотри, когда будет минута.', { edited_at: now - day - 7100, delivered_at: now - day - 7190, read_at: now - day - 7000 }),
    msg(a, alice, now - day - 7000, null, { deleted: true }),
    msg(a, alice, now - day - 6900, 'Смотрю.'),
    msg(a, alice, now - 5400, 'Макет экрана', { content_type: 'media', media: { name: 'chat-list-v3.png', mime: 'image/png', size: 842_113, kind: 'image' } }),
    msg(a, ME, now - 5000, null, { content_type: 'media', media: { name: 'Техническое задание (черновик).pdf', mime: 'application/pdf', size: 2_412_004, kind: 'file', local_path: '/home/dev/spec.pdf' } }),
    msg(a, alice, now - 2400, null, { content_type: 'media', media: { name: 'voice.weba', mime: 'audio/webm', size: 48_200, kind: 'voice', duration_ms: 17_400, waveform: [20, 60, 120, 200, 240, 180, 90, 140, 220, 255, 190, 110, 60, 40, 90, 170, 230, 210, 150, 80, 50, 100, 180, 240, 200, 130, 70, 40, 60, 120, 190, 230, 170, 100, 60, 90, 150, 210, 180, 120, 70, 40, 30, 60, 110, 160, 120, 60] } }),
    msg(a, ME, now - 1800, 'Отлично выглядит. Беру в работу 👍', { delivered_at: now - 1790 }),
    msg(a, ME, now - 600, 'Это сообщение не ушло: реле было недоступно.', { status: 'failed', failure_reason: 'no relay accepted' }),
    msg(a, ME, now - 60, 'А это ждёт отправки.', { status: 'queued' }),
  );
  const b = `dm:${boris}`;
  messages[b].push(
    msg(b, boris, now - 9000, 'NIP-77 на node-1 включу на этой неделе.'),
    msg(b, ME, now - 8900, 'Хорошо. Код менять не придётся: догон истории сам переключится на Negentropy.'),
    msg(b, boris, now - 300, 'Логи реле за ночь', { content_type: 'media', media: { name: 'relay-2026-09-29.log', mime: 'text/plain', size: 104_857_600, kind: 'file' } }),
    msg(b, boris, now - 200, 'Посмотри, там есть странные всплески около трёх ночи.'),
  );
  const v = `dm:${vera}`;
  messages[v].push(
    msg(v, '', now - 4001, 'request_received', { content_type: 'system', direction: 'out' }),
    msg(v, vera, now - 4000, 'Здравствуйте! Мне дали ваш ключ на конференции, хочу обсудить интеграцию.'),
  );
  const g = `dm:${gleb}`;
  messages[g].push(
    msg(g, ME, now - day * 3, 'Глеб, привет! Это я, напиши, когда появишься.'),
    msg(g, '', now - day * 3 + 1, 'request_sent', { content_type: 'system', direction: 'out' }),
  );
  messages[`dm:${bot}`].push(
    msg(`dm:${bot}`, bot, now - day * 5, 'Вы выиграли приз! Перейдите по ссылке…'),
    msg(`dm:${bot}`, '', now - day * 5 + 60, 'blocked', { content_type: 'system', direction: 'out' }),
  );
  const d = `dm:${daria}`;
  messages[d].push(
    msg(d, daria, now - day * 12, 'Спасибо за помощь с настройкой!'),
    msg(d, ME, now - day * 12 + 100, 'Обращайся.'),
    msg(d, '', now - day * 6, 'contact_left', { content_type: 'system', direction: 'out' }),
  );


  // Groups: one private that I own, one public where I am a member.
  const gid = (c: string) => c.repeat(64).slice(0, 64);
  const member = (pubkey: string, role: MessengerGroupMember['role'], at: number, muted = false): MessengerGroupMember =>
    ({ pubkey, role, muted, joined_at: at, is_me: pubkey === ME });
  const sys = (chat: string, at: number, what: string, actor: string, target: string | null = null, role: string | null = null) =>
    msg(chat, actor, at, what, { content_type: 'system', direction: 'out', media: { actor, target, role } });
  const team: MessengerGroup = {
    id: gid('7a'), chat_id: `group:${gid('7a')}`, kind: 'private', name: 'Команда Veydan', about: 'Рабочие вопросы по мессенджеру и реле.',
    picture: '', relay: 'wss://node-1.veydan.net', owner: ME, membership: 'joined', my_role: 'owner', muted: false, can_post: true, history_for_new: true,
    members: [member(ME, 'owner', now - day * 20), member(boris, 'admin', now - day * 19), member(alice, 'moderator', now - day * 18), member(daria, 'member', now - day * 3, true)],
    banned: [bot], requests: [vera], undecrypted: 0,
    key: { id: 'e0aa8b956aaf838d50ee6ab5d8622273', version: 3, cipher: 'AES-256-GCM', source: 'random', link_epoch: 0, since: now - day * 5, by: ME, reason: 'remove', held: true, status: 'good' },
    link: `veydan://group/${gid('7a')}?t=private&r=wss%3A%2F%2Fnode-1.veydan.net&o=${ME}&n=%D0%9A%D0%BE%D0%BC%D0%B0%D0%BD%D0%B4%D0%B0&m=${ME},${boris}`,
  };
  const square: MessengerGroup = {
    id: gid('8b'), chat_id: `group:${gid('8b')}`, kind: 'public', name: 'Veydan: открытый чат', about: 'Вход по ссылке. Вежливость обязательна.',
    picture: '', relay: 'wss://node-1.veydan.net', owner: boris, membership: 'joined', my_role: 'member', muted: false, can_post: true, history_for_new: true,
    members: [member(boris, 'owner', now - day * 40), member(alice, 'admin', now - day * 39), member(ME, 'member', now - day * 2), member(gleb, 'member', now - day)],
    banned: [], requests: [], undecrypted: 3,
    key: { id: '5a0f3c395349207ca3117c7e8590da77', version: 1, cipher: 'AES-256-GCM', source: 'link', link_epoch: 0, since: now - day * 40, by: boris, reason: 'create', held: true, status: 'deliver' },
    link: `veydan://group/${gid('8b')}?t=public&r=wss%3A%2F%2Fnode-1.veydan.net&o=${boris}&n=Veydan&s=M_ASAN9VOjsg94VjUdQvzIk-0vXXcmv_mjLOLp-neys&e=0`,
  };
  const left: MessengerGroup = {
    ...square, id: gid('9c'), chat_id: `group:${gid('9c')}`, name: 'Старая группа', about: '', membership: 'removed', my_role: null, can_post: false,
    members: [member(boris, 'owner', now - day * 40)], undecrypted: 0, link: null, key: null,
  };
  const groups = [team, square, left];
  for (const g of groups) {
    chats.push({ id: g.chat_id, kind: 'group', peer_pubkey: null, peer_npub: null, title: g.name, picture: null, is_contact: false, is_muted: false, unread: 0, last_message_at: null, last_preview: null, pinned: false, archived: false, mode: 'group', can_send: g.membership === 'joined' });
    messages[g.chat_id] = [];
  }
  const tm = team.chat_id;
  const t1 = msg(tm, boris, now - day - 3000, 'Коллеги, реле обновил. Группы теперь шифруются одним ключом на группу, реле не видит ни автора, ни текста.');
  messages[tm].push(
    sys(tm, now - day * 20, 'group_created', ME),
    sys(tm, now - day * 19, 'group_admitted', ME, boris),
    sys(tm, now - day * 19 + 60, 'group_role', ME, boris, 'admin'),
    sys(tm, now - day * 18, 'group_admitted', boris, alice),
    t1,
    msg(tm, alice, now - day - 2900, 'Отлично. А история для новых участников?'),
    msg(tm, alice, now - day - 2890, 'В приватных по настройке группы, в публичных всегда.'),
    msg(tm, ME, now - day - 2700, 'Да, именно так.', { reply_to: { id: t1.id, sender_pubkey: boris, text: t1.text }, read_at: now - day - 2600, seen_by: [boris, alice] }),
    sys(tm, now - day * 3, 'group_admitted', boris, daria),
    msg(tm, daria, now - day * 3 + 500, 'Всем привет!'),
    sys(tm, now - 7000, 'group_muted', alice, daria),
    msg(tm, boris, now - 900, 'Схема ключей', { content_type: 'media', media: { name: 'keys.png', mime: 'image/png', size: 311_204, kind: 'image' } }),
    msg(tm, alice, now - day * 2, null, { content_type: 'media', media: { name: 'board.jpg', mime: 'image/jpeg', size: 1_204_000, kind: 'image' } }),
    msg(tm, daria, now - day * 2 + 60, null, { content_type: 'media', media: { name: 'sketch.png', mime: 'image/png', size: 402_000, kind: 'image' } }),
    msg(tm, boris, now - day * 2 + 120, 'Спецификация: https://github.com/nostr-protocol/nips/blob/master/29.md'),
    msg(tm, daria, now - day * 2 + 200, null, { content_type: 'media', media: { name: 'voice.weba', mime: 'audio/webm', size: 31_000, kind: 'voice', duration_ms: 9_200, waveform: [40, 120, 200, 160, 90, 60, 140, 220, 180, 100] } }),
    msg(tm, ME, now - 400, 'Принято, смотрю.'),
  );
  messages[tm].sort((x, y) => x.created_at - y.created_at);
  const sq = square.chat_id;
  messages[sq].push(
    sys(sq, now - day * 2, 'group_joined', ME),
    msg(sq, boris, now - day * 2 + 100, 'Добро пожаловать!'),
    sys(sq, now - day, 'group_joined', gleb),
    sys(sq, now - 3300, 'group_joined', vera),
    sys(sq, now - 3200, 'group_joined', daria),
    sys(sq, now - 3100, 'group_left', vera),
    msg(sq, gleb, now - 3000, 'Подскажите, где взять сборку под Android?'),
    msg(sq, alice, now - 2800, 'Пока только тестовая, ссылка в закрепе.'),
  );
  messages[left.chat_id].push(sys(left.chat_id, now - day * 8, 'group_removed', boris, ME));
  const invites: MessengerGroupInvite[] = [{
    invite_id: 'inv1', group_id: gid('ad'), name: 'Книжный клуб', about: 'Читаем по книге в месяц.', picture: '', members: 12,
    peer: alice, direction: 'in', status: 'received', created_at: now - 3600, expires_at: now + day * 6,
  }];


  // Links of every kind, and what the chat makes of them.
  const stranger = gid('ad');
  const borisNpub = contacts.find((c) => c.pubkey === boris)?.npub ?? '';
  messages[a].push(
    msg(a, alice, now - 50, `Заходи к нам: ${square.link}`),
    msg(a, alice, now - 45, `veydan://group/${stranger}?t=private&r=wss%3A%2F%2Fnode-1.veydan.net&o=${alice}&n=${encodeURIComponent('Книжный клуб')}`),
    msg(a, ME, now - 40, `Это Борис, он по реле: veydan://contact/${borisNpub}?n=${encodeURIComponent('Борис')}`),
    msg(a, alice, now - 35, 'Статья про это: https://example.com/blog/nostr-groups?utm=1 и ещё старая http://old.example.org/page.'),
    msg(a, alice, now - 30, 'А это что-то подозрительное: https://аррӏе.com/login'),
    msg(a, alice, now - 25, 'Ссылка из новой версии: veydan://channel/42?n=News'),
    msg(a, alice, now - 20, 'И битая: veydan://group/123'),
    msg(a, alice, now - 15, 'javascript:alert(1) и file:///etc/passwd остаются текстом.'),
    msg(a, ME, now - 10, 'Фото', { content_type: 'media', media: { name: 'one.png', mime: 'image/png', size: 120_000, kind: 'image', batch: 'demo-batch-1' } }),
    msg(a, ME, now - 9, null, { content_type: 'media', media: { name: 'two.png', mime: 'image/png', size: 140_000, kind: 'image', batch: 'demo-batch-1' } }),
    msg(a, ME, now - 8, null, { content_type: 'media', media: { name: 'three.mp4', mime: 'video/mp4', size: 4_140_000, kind: 'video', batch: 'demo-batch-1' } }),
    msg(a, ME, now - 8, null, { content_type: 'media', media: { name: 'Отчёт за сентябрь.pdf', mime: 'application/pdf', size: 612_000, kind: 'file', batch: 'demo-batch-1' } }),
    ...[['backup-2026-09.tar.gz', 48_000_000], ['report.xlsx', 88_000], ['manifest.json', 3_400]].map(([name, size]) =>
      msg(a, ME, now - 6, null, { content_type: 'media', media: { name, mime: 'application/octet-stream', size, kind: 'file', batch: 'demo-batch-4', local_path: '/home/dev/x' } })),
    ...['vite.config.js', 'dev.sh', 'tsconfig.json', 'svelte.config.js'].map((name, i) =>
      msg(a, alice, now - 7, null, { content_type: 'media', media: { name, mime: 'text/plain', size: 1_200 + i * 700, kind: 'file', batch: 'demo-batch-2' } })),
    msg(a, alice, now - 5, null, { content_type: 'media', media: { name: 'IMG_2031.jpg', mime: 'image/jpeg', size: 2_300_000, kind: 'image' } }),
    ...['IMG_2032.jpg', 'IMG_2033.jpg', 'IMG_2034.jpg', 'IMG_2035.jpg', 'IMG_2036.jpg'].map((name, i) =>
      msg(a, alice, now - 4, i === 4 ? 'Поездка, часть первая' : null, { content_type: 'media', media: { name, mime: 'image/jpeg', size: 1_900_000, kind: 'image', batch: 'demo-batch-3' } })),
  );

  // Calls as the runtime writes their lines: answered both ways (one through a relay), declined,
  // not answered by Alice, and two of Boris's that I missed (the last one is new).
  const callLine = (chat: string, at: number, direction: 'in' | 'out', outcome: string | null, duration: number | null, via: string | null, media = 'audio') => {
    const id = `demo-call-${(n++).toString(16)}`;
    return msg(chat, '', at, 'call', {
      id: `sys:call:${id}`, content_type: 'system', direction,
      media: { call_id: id, direction, media, outcome, duration_secs: duration, via, started_at: at },
    });
  };
  messages[a].push(
    callLine(a, now - day * 2 - 3500, 'in', 'missed', null, null),
    callLine(a, now - day * 2 - 3300, 'out', 'ended', 754, 'direct'),
    callLine(a, now - day - 6000, 'in', 'ended', 185, 'relay'),
    callLine(a, now - day - 5000, 'out', 'missed', null, null, 'video'),
    callLine(a, now - 58, 'out', 'declined', null, null),
  );
  messages[b].push(
    callLine(b, now - 7000, 'in', 'busy', null, null),
    callLine(b, now - 160, 'in', 'missed', null, null),
  );
  messages[a].sort((x, y) => x.created_at - y.created_at);
  messages[b].sort((x, y) => x.created_at - y.created_at);

  // Contact cards: Boris's own, with his phone; Alice's of someone not in my contacts (never a phone).
  const borisP = contacts.find((c) => c.pubkey === boris)?.profile;
  const zhenya = profile(hex('c7'), 'Евгений Соколов', '{orange}Фотограф{/} и **путешественник**.\n~~Не~~ отвечаю быстро. Снимки: https://example.net/zhenya', null, {
    picture: `${DEMO_PICTURE_BASE}${'c7'.repeat(32)}`, website: 'https://example.net/zhenya',
    socials: [{ p: 'instagram', h: 'zhenya.photo' }, { p: 'youtube', h: 'zhenyatravels' }, { p: 'x', h: 'zhenya_s' }, { p: 'other', h: 'https://example.net/zhenya/gallery' }].map(demoSocialView),
  });
  if (borisP) {
    messages[b].push(msg(b, boris, now - 150, null, { content_type: 'contact', card: demoCard(borisP, '+79161234567', { is_contact: true }) }));
  }
  messages[a].push(msg(a, alice, now - 3, null, { content_type: 'contact', card: demoCard(zhenya, null) }));

  // Files on their way, in every state a transfer shows (Boris's chat). They
  // stand still until a button moves them; `transfers` are their rows as the
  // runtime would keep them.
  const MB = 1024 * 1024;
  const CHUNK = 4 * MB;
  const nowMs = Date.now();
  const transfers: MessengerTransfer[] = [];
  const parts = (size: number, chunk = CHUNK) => Math.max(1, Math.ceil(size / chunk));
  const row = (over: Partial<MessengerTransfer> & Pick<MessengerTransfer, 'id' | 'direction' | 'message_id' | 'file_name' | 'mime' | 'size'>): MessengerTransfer => ({
    chat_id: b, status: 'running', done_bytes: 0, attempts: 0, failure_reason: null, local_path: null,
    stage: over.direction === 'up' ? 'uploading' : 'downloading', chunks_done: 0, chunks_total: parts(over.size, over.chunk_size),
    chunk_size: CHUNK, rate_bps: 0, eta_secs: null, retry_at_ms: null, ...over,
  });
  let at = now - 140;
  /** A file of mine going up: its placeholder (`status`) and its transfer. */
  const up = (name: string, mime: string, kind: string, size: number, t: Partial<MessengerTransfer>, status: MessageStatus = 'uploading', media: Record<string, unknown> = {}) => {
    const id = `demo-up-${transfers.length}`;
    const m = msg(b, ME, at++, null, {
      id: `local:demo-${transfers.length}`, status, failure_reason: status === 'failed' ? (t.failure_reason ?? null) : null, content_type: 'media',
      media: { name, mime, size, kind, local_path: `/home/dev/${name}`, transfer_id: id, ...media },
    });
    transfers.push(row({ id, direction: 'up', message_id: m.id, file_name: name, mime, size, local_path: `/home/dev/${name}`, ...t }));
    return m;
  };
  /** A file of Boris's: its parts listed, fetched or not. */
  const down = (name: string, mime: string, kind: string, size: number, t: Partial<MessengerTransfer> | null) => {
    const m = msg(b, boris, at++, null, { content_type: 'media', media: { name, mime, size, kind, chunks: Array.from({ length: parts(size) }, () => ({})) } });
    if (t) transfers.push(row({ id: `demo-down-${transfers.length}`, direction: 'down', message_id: m.id, file_name: name, mime, size, ...t }));
    return m;
  };
  const trip = 'demo-batch-up';
  messages[b].push(
    msg(b, ME, at++, 'Все состояния передачи файлов — нажимайте кнопки.'),
    up('IMG_3001.jpg', 'image/jpeg', 'image', 6_800_000, { status: 'queued', stage: 'preparing', chunks_total: 0 }),
    up('design-export.zip', 'application/zip', 'file', 96 * MB, { stage: 'checking', done_bytes: 40 * MB, chunks_done: 7 }),
    up('project-backup.7z', 'application/x-7z-compressed', 'file', 812 * MB, { done_bytes: 330 * MB, chunks_done: 82, rate_bps: Math.round(5.2 * MB), eta_secs: 92 }),
    up('Договор (подписан).pdf', 'application/pdf', 'file', 3_100_000, { stage: 'publishing', done_bytes: 3_100_000, chunks_done: 1 }),
    up('backup-photos.tar', 'application/x-tar', 'file', 250 * MB, { status: 'waiting_retry', done_bytes: 120 * MB, chunks_done: 30, attempts: 2, failure_reason: 'err.network', retry_at_ms: nowMs + 12_000 }),
    up('raw-footage.mov', 'video/quicktime', 'file', 400 * MB, { status: 'paused', done_bytes: 100 * MB, chunks_done: 25 }, 'paused'),
    up('presentation.key', 'application/octet-stream', 'file', 48 * MB, { status: 'failed', done_bytes: 16 * MB, chunks_done: 4, attempts: 5, failure_reason: 'err.network' }, 'failed'),
    up('voice.weba', 'audio/webm', 'voice', 52_000, { status: 'failed', failure_reason: 'err.timeout' }, 'failed', {
      duration_ms: 6_200, waveform: [30, 90, 160, 220, 180, 120, 70, 140, 210, 250, 200, 130, 80, 50, 100, 170, 220, 160, 90, 60],
    }),
    msg(b, ME, at++, null, { content_type: 'media', media: { name: 'trip-1.jpg', mime: 'image/jpeg', size: 2_100_000, kind: 'image', batch: trip, local_path: '/home/dev/trip-1.jpg' } }),
    up('trip-2.jpg', 'image/jpeg', 'image', 2_300_000, { done_bytes: 920_000, chunks_done: 1, chunk_size: 786_432, chunks_total: 3, rate_bps: 410_000, eta_secs: 4 }, 'uploading', { batch: trip }),
    up('trip-3.jpg', 'image/jpeg', 'image', 1_900_000, { status: 'queued', stage: 'queued', chunk_size: 786_432, chunks_total: 3 }, 'uploading', { batch: trip }),
    down('dataset-2026.csv.gz', 'application/gzip', 'file', 820 * MB, null),
    down('archive-2025.zip', 'application/zip', 'file', 800 * MB, { done_bytes: 40 * MB, chunks_done: 10, rate_bps: Math.round(3.1 * MB), eta_secs: 245 }),
    down('IMG_4410.jpg', 'image/jpeg', 'image', 3_400_000, { done_bytes: 1_400_000, rate_bps: 600_000, eta_secs: 4 }),
    down('movie.mkv', 'video/x-matroska', 'video', 300 * MB, { stage: 'assembling', done_bytes: 300 * MB, chunks_done: 34 }),
    down('docs.pdf', 'application/pdf', 'file', 12 * MB, { stage: 'verifying', done_bytes: 12 * MB, chunks_done: 3 }),
    down('logs.tar.gz', 'application/gzip', 'file', 60 * MB, { status: 'failed', done_bytes: 20 * MB, chunks_done: 5, failure_reason: 'err.not_found' }),
  );

  const unread: Record<string, number> = { [b]: 2, [v]: 1, [sq]: 2 };
  for (const c of chats) {
    const list = messages[c.id].filter((x) => x.content_type !== 'system');
    const last = list[list.length - 1];
    if (last) {
      c.last_message_at = last.created_at;
      c.last_preview = last.deleted ? null : last.content_type === 'media' ? `📎 ${last.text ?? (last.media?.name as string)}` : last.card ? `👤 ${last.card.label}` : last.text;
    }
    c.unread = unread[c.id] ?? 0;
  }

  const ownProfile = profile(ME, 'Виталий', 'Строю **Veydan Space**. {blue}Приватность{/} по умолчанию.', null, {
    socials: [{ p: 'telegram', h: 'vitaly_v' }, { p: 'github', h: 'veydanproject' }].map(demoSocialView), website: 'https://veydan.net',
  });
  return {
    identity: { npub: ownProfile.npub, pubkey: ME, created_at: now - day * 30 },
    ownProfile, contacts, chats, messages, groups, invites, transfers,
  };
}
