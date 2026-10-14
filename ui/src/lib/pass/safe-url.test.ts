import { describe, expect, it } from 'vitest';
import { safeExternalUrl } from './safe-url';

describe('safeExternalUrl', () => {
  it('keeps http, https and mailto', () => {
    expect(safeExternalUrl('https://a.example/x?y=1')).toBe('https://a.example/x?y=1');
    expect(safeExternalUrl('HTTP://a.example')).toBe('HTTP://a.example');
    expect(safeExternalUrl('mailto:a@b.example')).toBe('mailto:a@b.example');
  });
  it('gives a bare domain https', () => {
    expect(safeExternalUrl('a.example')).toBe('https://a.example');
    expect(safeExternalUrl('a.example:8080/login')).toBe('https://a.example:8080/login');
    expect(safeExternalUrl('  a.example  ')).toBe('https://a.example');
  });
  it('trims spaces around a scheme', () => {
    expect(safeExternalUrl('  https://a.example ')).toBe('https://a.example');
  });
  it('refuses every other scheme', () => {
    for (const bad of [
      'javascript:alert(1)',
      'JaVaScRiPt:alert(1)',
      '  javascript:alert(1)',
      'java\nscript:alert(1)',
      'data:text/html,<script>1</script>',
      'file:///etc/passwd',
      'vbscript:x',
      '//evil.example',
    ]) {
      expect(safeExternalUrl(bad), bad).toBeNull();
    }
  });
  it('refuses empty and text that is not an address', () => {
    expect(safeExternalUrl('')).toBeNull();
    expect(safeExternalUrl('   ')).toBeNull();
    expect(safeExternalUrl(null)).toBeNull();
    expect(safeExternalUrl(undefined)).toBeNull();
    expect(safeExternalUrl('my bank login')).toBeNull();
    expect(safeExternalUrl('mailto:')).toBeNull();
  });
});
