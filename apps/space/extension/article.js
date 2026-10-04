// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Injected on demand (tabs.executeScript). Picks the main content block,
// converts it to Markdown and collects inline images as base64.
// Resolves to { title, markdown, images: [{ src, name, data_b64 }] }.

(async () => {
  const NOISE_SELECTOR =
    'script,style,noscript,template,iframe,svg,canvas,form,button,input,select,textarea,' +
    'nav,aside,footer,header,[hidden],[aria-hidden="true"],[role="navigation"],[role="banner"],' +
    '[role="complementary"],[role="contentinfo"]';
  const NOISE_NAME = /(^|[\s_-])(comment|share|social|sidebar|related|promo|advert|ads?|cookie|newsletter|menu|nav|breadcrumb|popup|modal|subscribe)([\s_-]|$)/i;
  const MAX_IMAGES = 20;
  const MAX_IMAGE_BYTES = 3 * 1024 * 1024;

  const text = (el) => (el.textContent || '').replace(/\s+/g, ' ').trim();

  function isNoise(el) {
    const name = `${el.className || ''} ${el.id || ''}`;
    return typeof name === 'string' && NOISE_NAME.test(name);
  }

  // Score paragraphs onto their ancestors, readability-style.
  function pickRoot() {
    const explicit = document.querySelector('article, [role="main"], main');
    const scores = new Map();
    for (const p of document.querySelectorAll('p, pre')) {
      const len = text(p).length;
      if (len < 40) continue;
      const parent = p.parentElement;
      if (!parent) continue;
      scores.set(parent, (scores.get(parent) || 0) + len);
      const grand = parent.parentElement;
      if (grand) scores.set(grand, (scores.get(grand) || 0) + len / 2);
    }
    let best = null;
    let bestScore = 0;
    for (const [el, score] of scores) {
      if (isNoise(el)) continue;
      if (score > bestScore) { best = el; bestScore = score; }
    }
    if (explicit && best && explicit.contains(best) && text(explicit).length < text(best).length * 3) return explicit;
    return best || explicit || document.body;
  }

  function clean(root) {
    const copy = root.cloneNode(true);
    for (const el of copy.querySelectorAll(NOISE_SELECTOR)) el.remove();
    for (const el of copy.querySelectorAll('div,section,ul,ol,span,p')) {
      if (isNoise(el) && text(el).length < 800) el.remove();
    }
    return copy;
  }

  const images = [];
  const imageBySrc = new Map();

  function imageName(src, mime) {
    let base = '';
    try { base = decodeURIComponent(new URL(src, location.href).pathname.split('/').pop() || ''); } catch {}
    base = base.replace(/[^\w.\-]+/g, '_').slice(0, 80);
    if (!/\.[a-z0-9]{2,5}$/i.test(base)) {
      const ext = (mime.split('/')[1] || 'png').replace('jpeg', 'jpg').replace(/\+.*$/, '');
      base = `${base || 'image'}-${images.length + 1}.${ext}`;
    }
    return base;
  }

  function registerImage(img) {
    const raw = img.currentSrc || img.getAttribute('src') || '';
    if (!raw || raw.startsWith('data:')) return raw;
    let abs;
    try { abs = new URL(raw, location.href).href; } catch { return raw; }
    if (!imageBySrc.has(abs) && images.length < MAX_IMAGES) {
      const entry = { src: abs, name: '', data_b64: '' };
      images.push(entry);
      imageBySrc.set(abs, entry);
    }
    return abs;
  }

  const escapeCell = (s) => s.replace(/\|/g, '\\|');

  function convertChildren(node, ctx) {
    let out = '';
    for (const child of node.childNodes) out += convert(child, ctx);
    return out;
  }

  function convertList(el, ctx, ordered) {
    let out = '\n';
    let i = 1;
    for (const li of el.children) {
      if (li.tagName !== 'LI') continue;
      const marker = ordered ? `${i++}. ` : '- ';
      const body = convertChildren(li, { ...ctx, depth: ctx.depth + 1 }).trim().replace(/\n/g, '\n' + ' '.repeat(marker.length));
      out += `${'  '.repeat(ctx.depth)}${marker}${body}\n`;
    }
    return out + '\n';
  }

  function convertTable(el) {
    const rows = [...el.querySelectorAll('tr')].map((tr) => [...tr.children].map((td) => escapeCell(text(td))));
    if (rows.length === 0) return '';
    const width = Math.max(...rows.map((r) => r.length));
    const line = (cells) => `| ${Array.from({ length: width }, (_, i) => cells[i] || '').join(' | ')} |`;
    const [head, ...body] = rows;
    return `\n${line(head)}\n${line(Array(width).fill('---'))}\n${body.map(line).join('\n')}\n\n`;
  }

  function convert(node, ctx) {
    if (node.nodeType === Node.TEXT_NODE) {
      const t = node.nodeValue || '';
      return ctx.pre ? t : t.replace(/\s+/g, ' ');
    }
    if (node.nodeType !== Node.ELEMENT_NODE) return '';
    const el = node;
    const tag = el.tagName.toLowerCase();
    const inner = () => convertChildren(el, ctx);

    switch (tag) {
      case 'h1': case 'h2': case 'h3': case 'h4': case 'h5': case 'h6': {
        const level = Math.min(6, Number(tag[1]) + (ctx.headingShift || 0));
        return `\n${'#'.repeat(level)} ${inner().trim()}\n\n`;
      }
      case 'p': return `\n${inner().trim()}\n\n`;
      case 'br': return ctx.pre ? '\n' : '  \n';
      case 'hr': return '\n---\n\n';
      case 'strong': case 'b': { const t = inner().trim(); return t ? `**${t}**` : ''; }
      case 'em': case 'i': { const t = inner().trim(); return t ? `*${t}*` : ''; }
      case 'del': case 's': { const t = inner().trim(); return t ? `~~${t}~~` : ''; }
      case 'code': {
        if (ctx.pre) return inner();
        const t = el.textContent || '';
        return t.includes('`') ? `\`\` ${t} \`\`` : `\`${t}\``;
      }
      case 'pre': {
        const code = el.querySelector('code');
        const lang = ((code || el).className.match(/(?:language|lang)-([\w+-]+)/) || [])[1] || '';
        const body = (el.textContent || '').replace(/\n$/, '');
        return `\n\`\`\`${lang}\n${body}\n\`\`\`\n\n`;
      }
      case 'blockquote': {
        const body = inner().trim().split('\n').map((l) => `> ${l}`).join('\n');
        return `\n${body}\n\n`;
      }
      case 'ul': return convertList(el, ctx, false);
      case 'ol': return convertList(el, ctx, true);
      case 'a': {
        const href = el.getAttribute('href') || '';
        const t = inner().trim();
        if (!href || href.startsWith('javascript:')) return t;
        let abs = href;
        try { abs = new URL(href, location.href).href; } catch {}
        return t ? `[${t}](${abs})` : '';
      }
      case 'img': {
        const src = registerImage(el);
        if (!src) return '';
        const alt = (el.getAttribute('alt') || '').replace(/[\[\]]/g, ' ').trim();
        return `![${alt}](${src})`;
      }
      case 'figure': {
        const img = el.querySelector('img');
        const cap = el.querySelector('figcaption');
        const body = img ? convert(img, ctx) : inner().trim();
        return `\n${body}${cap ? `\n*${text(cap)}*` : ''}\n\n`;
      }
      case 'table': return convertTable(el);
      case 'li': return inner();
      case 'div': case 'section': case 'article': case 'main': case 'body': {
        const body = inner();
        return /\n\s*$/.test(body) ? body : `${body}\n`;
      }
      default: return inner();
    }
  }

  async function fetchImage(entry) {
    try {
      const res = await fetch(entry.src, { credentials: 'omit', cache: 'force-cache' });
      if (!res.ok) return;
      const blob = await res.blob();
      if (!blob.type.startsWith('image/') || blob.size === 0 || blob.size > MAX_IMAGE_BYTES) return;
      const buf = new Uint8Array(await blob.arrayBuffer());
      let bin = '';
      for (let i = 0; i < buf.length; i += 0x8000) bin += String.fromCharCode.apply(null, buf.subarray(i, i + 0x8000));
      entry.name = imageName(entry.src, blob.type);
      entry.data_b64 = btoa(bin);
    } catch {
      // Cross-origin without CORS: the Markdown keeps the original URL
    }
  }

  const root = clean(pickRoot());
  const title = text(document.querySelector('h1')) || document.title || location.hostname;
  let markdown = convert(root, { depth: 0, pre: false, headingShift: 0 })
    .replace(/[ \t]+\n/g, '\n')
    .replace(/\n{3,}/g, '\n\n')
    .trim();
  // Drop a leading heading that duplicates the title
  const escapedTitle = title.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  markdown = markdown.replace(new RegExp('^#{1,6} ' + escapedTitle + '\\n+'), '');

  await Promise.all(images.map(fetchImage));
  return { title, markdown, images: images.filter((i) => i.data_b64) };
})();
