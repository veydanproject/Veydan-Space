/**
 * The address of an entry is text from the vault, possibly synced from elsewhere: only http, https and mailto
 * may be opened. A bare domain gets https://; any other scheme (javascript:, data:, file:, …) gives null.
 */
export function safeExternalUrl(raw: string | null | undefined): string | null {
  const url = (raw ?? '').trim();
  if (!url || /[\u0000-\u001f\u007f]/.test(url)) return null;
  const scheme = /^([a-z][a-z0-9+.-]*):/i.exec(url);
  if (scheme) {
    const name = scheme[1].toLowerCase();
    // `host:8080` is a bare domain with a port, not a scheme.
    if (/^\d+(\/|$)/.test(url.slice(scheme[0].length)) && name !== 'http' && name !== 'https' && name !== 'mailto') {
      return safeExternalUrl(`https://${url}`);
    }
    if (name === 'mailto') return url.length > 'mailto:'.length ? url : null;
    if (name !== 'http' && name !== 'https') return null;
    try {
      return new URL(url).host ? url : null;
    } catch {
      return null;
    }
  }
  if (/^\/\//.test(url) || /\s/.test(url)) return null;
  try {
    const parsed = new URL(`https://${url}`);
    return parsed.host.includes('.') || parsed.hostname === 'localhost' ? `https://${url}` : null;
  } catch {
    return null;
  }
}
