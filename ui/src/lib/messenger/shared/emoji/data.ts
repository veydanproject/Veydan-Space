// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// A compact emoji set: the ones people actually type in chats. Plain
// Unicode, rendered by the system font; no images, no network.

export interface EmojiGroup {
  id: 'smileys' | 'gestures' | 'hearts' | 'nature' | 'food' | 'activity' | 'objects' | 'symbols';
  icon: string;
  list: string[];
}

const split = (s: string) => s.trim().split(/\s+/);

export const EMOJI: EmojiGroup[] = [
  { id: 'smileys', icon: '😀', list: split(`
    😀 😃 😄 😁 😆 😅 🤣 😂 🙂 🙃 😉 😊 😇 🥰 😍 🤩 😘 😗 😚 😙 😋 😛 😜 🤪 😝 🤑 🤗 🤭 🤫 🤔
    🤐 🤨 😐 😑 😶 😏 😒 🙄 😬 🤥 😌 😔 😪 🤤 😴 😷 🤒 🤕 🤢 🤮 🤧 🥵 🥶 🥴 😵 🤯 🤠 🥳 😎 🤓
    🧐 😕 😟 🙁 ☹️ 😮 😯 😲 😳 🥺 😦 😧 😨 😰 😥 😢 😭 😱 😖 😣 😞 😓 😩 😫 🥱 😤 😡 😠 🤬 😈
    👿 💀 ☠️ 💩 🤡 👻 👽 🤖 😺 😸 😹 😻 😼 😽 🙀 😿 😾 🙈 🙉 🙊`) },
  { id: 'gestures', icon: '👍', list: split(`
    👍 👎 👌 🤌 🤏 ✌️ 🤞 🤟 🤘 🤙 👈 👉 👆 👇 ☝️ ✋ 🤚 🖐️ 🖖 👋 🤝 🙏 ✍️ 👏 🙌 👐 🤲 💪 🦾 🖕
    🤛 🤜 ✊ 👊 🫶 🫡 🫠 🫣 🫢 🤷 🤦 🙋 🙅 🙆 💁 🙇 👀 👁️ 🧠 🫀 👂 👃 👄 💋 👶 🧒 👦 👧 🧑 👨
    👩 🧓 👴 👵`) },
  { id: 'hearts', icon: '❤️', list: split(`
    ❤️ 🧡 💛 💚 💙 💜 🖤 🤍 🤎 💔 ❣️ 💕 💞 💓 💗 💖 💘 💝 💟 ♥️ 💯 💢 💥 💫 💦 💨 🕳️ 💬 💭 💤`) },
  { id: 'nature', icon: '🌿', list: split(`
    🐶 🐱 🐭 🐹 🐰 🦊 🐻 🐼 🐨 🐯 🦁 🐮 🐷 🐸 🐵 🐔 🐧 🐦 🐤 🦆 🦅 🦉 🦇 🐺 🐗 🐴 🦄 🐝 🐛 🦋
    🐌 🐞 🐜 🕷️ 🐢 🐍 🦎 🐙 🦑 🦀 🐠 🐟 🐬 🐳 🦈 🐊 🐘 🦒 🐪 🐎 🐖 🐑 🐕 🐈 🌵 🎄 🌲 🌳 🌴 🌱
    🌿 ☘️ 🍀 🍁 🍂 🍃 🌷 🌹 🌺 🌸 🌼 🌻 🌞 🌝 🌚 🌙 ⭐ 🌟 ✨ ⚡ 🔥 🌈 ☀️ ⛅ ☁️ 🌧️ ⛈️ ❄️ ☃️ 💧 🌊`) },
  { id: 'food', icon: '🍎', list: split(`
    🍏 🍎 🍐 🍊 🍋 🍌 🍉 🍇 🍓 🫐 🍒 🍑 🥭 🍍 🥥 🥝 🍅 🍆 🥑 🥦 🥒 🌶️ 🌽 🥕 🧄 🧅 🥔 🍞 🥐 🥖
    🧀 🥚 🍳 🥞 🥓 🍗 🍖 🌭 🍔 🍟 🍕 🥪 🌮 🌯 🥗 🍝 🍜 🍲 🍣 🍱 🥟 🍤 🍚 🍙 🍦 🍰 🎂 🍮 🍭 🍫
    🍿 🍩 🍪 🥛 ☕ 🍵 🧃 🥤 🍺 🍻 🥂 🍷 🥃 🍸 🍹 🍾`) },
  { id: 'activity', icon: '⚽', list: split(`
    ⚽ 🏀 🏈 ⚾ 🎾 🏐 🏉 🎱 🏓 🏸 🥅 🏒 ⛳ 🏹 🎣 🥊 🥋 🛹 ⛸️ 🎿 🏂 🏋️ 🤸 🏄 🏊 🚴 🧘 🏆 🥇 🥈
    🥉 🏅 🎖️ 🎫 🎪 🎭 🎨 🎬 🎤 🎧 🎼 🎹 🥁 🎷 🎺 🎸 🎻 🎲 ♟️ 🎯 🎳 🎮 🎰 🧩 🚗 🚕 🚌 🚓 🚑 🚒
    🚚 🚜 🏍️ 🚲 ✈️ 🚀 🛸 🚁 ⛵ 🚢 🚂 🚇 🗺️ 🏠 🏢 🏰 🗽 ⛰️ 🏖️ 🏝️`) },
  { id: 'objects', icon: '💡', list: split(`
    ⌚ 📱 💻 ⌨️ 🖥️ 🖨️ 🖱️ 💾 💿 📷 📹 🎥 📞 ☎️ 📺 📻 🧭 ⏰ ⏳ 📡 🔋 🔌 💡 🔦 🕯️ 🧯 💸 💵 💰 💳
    💎 ⚖️ 🧰 🔧 🔨 ⚙️ 🧲 🔫 💣 🔪 🛡️ 🔮 🧿 💊 💉 🧬 🔬 🔭 🧹 🧺 🧻 🧼 🔑 🗝️ 🚪 🛏️ 🛋️ 🎁 🎈 🎉
    🎊 ✉️ 📩 📦 📜 📄 📊 📈 📉 📅 📌 📎 ✂️ 📝 ✏️ 🔍 🔒 🔓 🔐`) },
  { id: 'symbols', icon: '✅', list: split(`
    ✅ ☑️ ✔️ ❌ ❎ ➕ ➖ ➗ ✖️ ❓ ❔ ❗ ❕ ‼️ ⁉️ ⚠️ 🚫 ⛔ 🔞 ♻️ ⚜️ 🔱 📛 🔰 ⭕ 🆗 🆕 🆒 🆓 🆘
    🔴 🟠 🟡 🟢 🔵 🟣 ⚫ ⚪ 🟤 🔺 🔻 🔸 🔹 🔶 🔷 ▶️ ⏸️ ⏹️ ⏺️ ⏭️ ⏮️ 🔀 🔁 🔂 ◀️ 🔼 🔽 ➡️ ⬅️ ⬆️
    ⬇️ ↗️ ↘️ ↙️ ↖️ ↩️ ↪️ 🔄 🔃 🎵 🎶 ™️ ©️ ®️ 💲 #️⃣ *️⃣ 0️⃣ 1️⃣ 2️⃣ 3️⃣ 🏁 🚩 🏳️ 🏴`) },
];

const RECENT_KEY = 'messenger.emoji.recent';
const RECENT_MAX = 24;

export function loadRecent(): string[] {
  try {
    const raw = JSON.parse(localStorage.getItem(RECENT_KEY) ?? '[]');
    return Array.isArray(raw) ? raw.filter((x): x is string => typeof x === 'string').slice(0, RECENT_MAX) : [];
  } catch {
    return [];
  }
}

export function pushRecent(emoji: string): string[] {
  const next = [emoji, ...loadRecent().filter((e) => e !== emoji)].slice(0, RECENT_MAX);
  try { localStorage.setItem(RECENT_KEY, JSON.stringify(next)); } catch { /* private mode */ }
  return next;
}
