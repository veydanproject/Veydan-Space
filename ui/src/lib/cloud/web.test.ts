// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The web cabinet as the app speaks of it: the link of a sign-in is its id
// alone, the route of a confirmation carries only values of their shape,
// and a country, a browser, a code, a key are shown only as they should be.

import { describe, expect, it } from 'vitest';
import type { TranslationKey } from '$lib/core/i18n';
import {
  browserKey,
  countryName,
  formatLoginCode,
  isActivationKey,
  isLoginCode,
  isWebId,
  originLine,
  webConfirmHref,
  webConfirmOf,
  webLoginHref,
  webLoginId,
} from '$lib/cloud/web';

const ID = '0123456789abcdef';
const t = (key: TranslationKey, vars?: Record<string, string>) => (vars ? `${key} ${JSON.stringify(vars)}` : key);

describe('the link of a sign-in', () => {
  it('is veydan://web/<16 hex> and nothing else', () => {
    expect(webLoginId(`veydan://web/${ID}`)).toBe(ID);
    expect(webLoginId(`  veydan://web/${ID}\n`)).toBe(ID);
    expect(webLoginId(`veydan://web/${ID}?n=later`)).toBe(ID);
    for (const bad of [
      '', ID, `https://my.veydan.net/login/${ID}`, `veydan://web/${ID.toUpperCase()}`, `veydan://web/${ID}0`,
      `veydan://contact/${ID}`, `veydan://web/${ID} and more`,
    ]) {
      expect(webLoginId(bad), bad).toBeNull();
    }
    expect(webLoginHref(ID)).toBe(`/cloud/web?id=${ID}`);
    expect(isWebId(ID) && !isWebId('../x') && !isWebId(null)).toBe(true);
  });
});

describe('the route of a confirmation', () => {
  it('goes there and back with the values of the card', () => {
    const ask = { id: ID, action: 'delete' as const, country: 'DE', uaFamily: 'chrome', expiresAt: 1_791_000_300 };
    const href = webConfirmHref(ask);
    expect(href).toBe(`/cloud/web/confirm?id=${ID}&action=delete&country=DE&ua=chrome&until=1791000300`);
    expect(webConfirmOf(href.split('?')[1])).toEqual(ask);
  });

  it('drops what is not of its shape, and refuses an id or an action that is not one', () => {
    expect(webConfirmOf(`id=${ID}&action=delete&country=Germany&ua=Mozilla&until=soon`)).toEqual({
      id: ID, action: 'delete', country: null, uaFamily: null, expiresAt: null,
    });
    expect(webConfirmOf(`id=${ID}&action=transfer`)).toBeNull();
    expect(webConfirmOf('id=evil&action=delete')).toBeNull();
    expect(webConfirmOf('')).toBeNull();
  });
});

describe('where a request came from', () => {
  it('is a country named by Intl and a browser of the list', () => {
    expect(countryName('DE', 'en')).toBe('Germany');
    expect(countryName('de', 'en')).toBeNull();
    expect(countryName('Visit evil.example', 'en')).toBeNull();
    expect(browserKey('firefox')).toBe('cloud_web_ua_firefox');
    expect(browserKey('Chrome 120')).toBe('cloud_web_ua_other');
    expect(browserKey('constructor')).toBe('cloud_web_ua_other');
    expect(originLine('FR', 'safari', t, 'en')).toBe('cloud_web_origin {"country":"France","browser":"cloud_web_ua_safari"}');
    expect(originLine(null, 'edge', t, 'en')).toBe('cloud_web_ua_edge');
  });
});

describe('a code and a key', () => {
  it('are held to their shape', () => {
    expect(isLoginCode('012345') && isLoginCode('999999')).toBe(true);
    for (const bad of ['12345', '1234567', '12 345', 'abcdef', null, undefined]) expect(isLoginCode(bad), String(bad)).toBe(false);
    expect(formatLoginCode('012345')).toBe('012 345');
    expect(isActivationKey('7K2M-ABCD-EFGH-JKMN-PQRS')).toBe(true);
    for (const bad of ['7K2M-ABCD-EFGH-JKMN', '7k2m-abcd-efgh-jkmn-pqrs', '7K2M-ABCD-EFGH-JKMN-PQRU', 'Pay_at_evil_example_now', null]) {
      expect(isActivationKey(bad), String(bad)).toBe(false);
    }
  });
});
