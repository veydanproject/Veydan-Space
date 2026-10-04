// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Demo data for the browser preview (`pnpm dev` without Tauri). Enabled by
// `localStorage['messenger.demo'] = '1'`. Never used inside the app.

import type {
  ChatMode, MessengerChat, MessengerContact, MessengerGroup, MessengerGroupInvite, MessengerGroupMember, MessengerIdentity,
  MessengerMessage, MessengerProfile,
} from './api';

const ME = 'ab'.repeat(32);
const hex = (c: string) => c.repeat(64).slice(0, 64);
const npub = (pk: string, tag: string) => `npub1${tag}${pk}`.slice(0, 63);

function profile(pk: string, name: string, about: string, nip05: string | null): MessengerProfile {
  return {
    pubkey: pk, npub: npub(pk, name.toLowerCase().replace(/[^a-z]/g, '')), name: name.toLowerCase(), display_name: name, about,
    picture: null, banner: null, website: null, nip05, lud16: null, nip05_verified: !!nip05, event_created_at: 1, fetched_at: 1,
  };
}

export interface DemoData {
  identity: MessengerIdentity;
  ownProfile: MessengerProfile;
  contacts: MessengerContact[];
  chats: MessengerChat[];
  messages: Record<string, MessengerMessage[]>;
  groups: MessengerGroup[];
  invites: MessengerGroupInvite[];
}

export function demoEnabled(): boolean {
  try { return typeof localStorage !== 'undefined' && localStorage.getItem('messenger.demo') === '1'; }
  catch { return false; }
}

export function buildDemo(): DemoData {
  const now = Math.floor(Date.now() / 1000);
  const people: [string, string, string, string | null, ChatMode, boolean][] = [
    [hex('1a'), 'Алиса Морозова', 'Дизайнер. Пишу редко, но по делу.', 'alice@veydan.net', 'full_chat', true],
    [hex('2b'), 'Борис', 'Бэкенд, реле, инфраструктура.', null, 'full_chat', true],
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
    created_at: at, edited_at: null, deleted: false, failure_reason: null, media: null, ...extra,
  });

  for (const [pk, name, about, nip05, mode, canSend] of people) {
    const p = profile(pk, name, about, nip05);
    const isContact = mode === 'full_chat' || mode === 'request_sent' || mode === 'removed_by_peer';
    if (isContact) {
      contacts.push({ pubkey: pk, npub: p.npub, nickname: null, note: null, followed: name === 'Алиса Морозова', profile: p, created_at: now - 86400 * 9, updated_at: now - 3600 });
    }
    const id = `dm:${pk}`;
    chats.push({ id, kind: 'dm', peer_pubkey: pk, peer_npub: p.npub, title: name, picture: null, is_contact: isContact, is_muted: name === 'Борис', unread: 0, last_message_at: null, last_preview: null, pinned: name === 'Алиса Морозова', archived: false, mode, can_send: canSend });
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
    msg(a, alice, now - day * 2 - 3790, 'И ещё: закреплённые чаты сверху.'),
    msg(a, ME, now - day * 2 - 3600, 'Согласен по всем трём пунктам. Сделаю сегодня.', { reply_to: { id: m1.id, sender_pubkey: alice, text: m1.text } }),
    msg(a, ME, now - day - 7200, 'Готово, обновил. Посмотри, когда будет минута.', { edited_at: now - day - 7100 }),
    msg(a, alice, now - day - 7000, null, { deleted: true }),
    msg(a, alice, now - day - 6900, 'Смотрю.'),
    msg(a, alice, now - 5400, 'Макет экрана', { content_type: 'media', media: { name: 'chat-list-v3.png', mime: 'image/png', size: 842_113, kind: 'image' } }),
    msg(a, ME, now - 5000, null, { content_type: 'media', media: { name: 'Техническое задание (черновик).pdf', mime: 'application/pdf', size: 2_412_004, kind: 'file', local_path: '/home/dev/spec.pdf' } }),
    msg(a, alice, now - 2400, null, { content_type: 'media', media: { name: 'voice.weba', mime: 'audio/webm', size: 48_200, kind: 'voice', duration_ms: 17_400, waveform: [20, 60, 120, 200, 240, 180, 90, 140, 220, 255, 190, 110, 60, 40, 90, 170, 230, 210, 150, 80, 50, 100, 180, 240, 200, 130, 70, 40, 60, 120, 190, 230, 170, 100, 60, 90, 150, 210, 180, 120, 70, 40, 30, 60, 110, 160, 120, 60] } }),
    msg(a, ME, now - 1800, 'Отлично выглядит. Беру в работу 👍'),
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
    msg(tm, ME, now - day - 2700, 'Да, именно так.', { reply_to: { id: t1.id, sender_pubkey: boris, text: t1.text } }),
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

  const unread: Record<string, number> = { [b]: 2, [v]: 1, [sq]: 2 };
  for (const c of chats) {
    const list = messages[c.id].filter((x) => x.content_type !== 'system');
    const last = list[list.length - 1];
    if (last) {
      c.last_message_at = last.created_at;
      c.last_preview = last.deleted ? null : last.content_type === 'media' ? `📎 ${last.text ?? (last.media?.name as string)}` : last.text;
    }
    c.unread = unread[c.id] ?? 0;
  }

  const ownProfile = profile(ME, 'Виталий', 'Строю Veydan Space.', null);
  return {
    identity: { npub: ownProfile.npub, pubkey: ME, created_at: now - day * 30 },
    ownProfile, contacts, chats, messages, groups, invites,
  };
}
