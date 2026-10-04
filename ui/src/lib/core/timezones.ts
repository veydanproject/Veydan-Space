// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

export interface TimezoneOption {
  value: string;
  label: string;
  region: 'europe' | 'america' | 'asia' | 'africa' | 'pacific' | 'australia';
}

export const TIMEZONES: TimezoneOption[] = [
  // Europe
  { value: 'Europe/London',     label: 'London (GMT+0/+1)',     region: 'europe' },
  { value: 'Europe/Paris',      label: 'Paris (GMT+1/+2)',      region: 'europe' },
  { value: 'Europe/Berlin',     label: 'Berlin (GMT+1/+2)',     region: 'europe' },
  { value: 'Europe/Amsterdam',  label: 'Amsterdam (GMT+1/+2)',  region: 'europe' },
  { value: 'Europe/Warsaw',     label: 'Warsaw (GMT+1/+2)',     region: 'europe' },
  { value: 'Europe/Madrid',     label: 'Madrid (GMT+1/+2)',     region: 'europe' },
  { value: 'Europe/Rome',       label: 'Rome (GMT+1/+2)',       region: 'europe' },
  { value: 'Europe/Stockholm',  label: 'Stockholm (GMT+1/+2)',  region: 'europe' },
  { value: 'Europe/Helsinki',   label: 'Helsinki (GMT+2/+3)',   region: 'europe' },
  { value: 'Europe/Kiev',       label: 'Kyiv (GMT+2/+3)',       region: 'europe' },
  { value: 'Europe/Istanbul',   label: 'Istanbul (GMT+3)',      region: 'europe' },
  { value: 'Europe/Moscow',     label: 'Moscow (GMT+3)',        region: 'europe' },
  // America
  { value: 'America/New_York',     label: 'New York (GMT-5/-4)',    region: 'america' },
  { value: 'America/Chicago',      label: 'Chicago (GMT-6/-5)',     region: 'america' },
  { value: 'America/Denver',       label: 'Denver (GMT-7/-6)',      region: 'america' },
  { value: 'America/Los_Angeles',  label: 'Los Angeles (GMT-8/-7)', region: 'america' },
  { value: 'America/Toronto',      label: 'Toronto (GMT-5/-4)',     region: 'america' },
  { value: 'America/Vancouver',    label: 'Vancouver (GMT-8/-7)',   region: 'america' },
  { value: 'America/Sao_Paulo',    label: 'São Paulo (GMT-3)',      region: 'america' },
  { value: 'America/Mexico_City',  label: 'Mexico City (GMT-6/-5)', region: 'america' },
  // Asia
  { value: 'Asia/Tokyo',      label: 'Tokyo (GMT+9)',        region: 'asia' },
  { value: 'Asia/Seoul',      label: 'Seoul (GMT+9)',        region: 'asia' },
  { value: 'Asia/Shanghai',   label: 'Shanghai (GMT+8)',     region: 'asia' },
  { value: 'Asia/Singapore',  label: 'Singapore (GMT+8)',    region: 'asia' },
  { value: 'Asia/Hong_Kong',  label: 'Hong Kong (GMT+8)',    region: 'asia' },
  { value: 'Asia/Bangkok',    label: 'Bangkok (GMT+7)',      region: 'asia' },
  { value: 'Asia/Jakarta',    label: 'Jakarta (GMT+7)',      region: 'asia' },
  { value: 'Asia/Kolkata',    label: 'Kolkata (GMT+5:30)',   region: 'asia' },
  { value: 'Asia/Dubai',      label: 'Dubai (GMT+4)',        region: 'asia' },
  { value: 'Asia/Almaty',     label: 'Almaty (GMT+6)',       region: 'asia' },
  // Australia / Pacific
  { value: 'Australia/Sydney',    label: 'Sydney (GMT+10/+11)',   region: 'australia' },
  { value: 'Australia/Melbourne', label: 'Melbourne (GMT+10/+11)', region: 'australia' },
  { value: 'Pacific/Auckland',    label: 'Auckland (GMT+12/+13)', region: 'pacific' },
  // Africa
  { value: 'Africa/Cairo',         label: 'Cairo (GMT+2/+3)',     region: 'africa' },
  { value: 'Africa/Johannesburg',  label: 'Johannesburg (GMT+2)', region: 'africa' },
  // UTC
  { value: 'UTC', label: 'UTC (GMT+0)', region: 'europe' },
];

/** Country code (2-letter ISO) → expected timezone region */
const COUNTRY_REGION: Record<string, TimezoneOption['region']> = {
  GB: 'europe', DE: 'europe', FR: 'europe', NL: 'europe', BE: 'europe',
  IT: 'europe', ES: 'europe', PT: 'europe', SE: 'europe', NO: 'europe',
  DK: 'europe', FI: 'europe', PL: 'europe', CZ: 'europe', AT: 'europe',
  CH: 'europe', HU: 'europe', RO: 'europe', GR: 'europe', TR: 'europe',
  RU: 'europe', UA: 'europe',
  US: 'america', CA: 'america', MX: 'america', BR: 'america', AR: 'america',
  CL: 'america', CO: 'america', PE: 'america',
  JP: 'asia', CN: 'asia', KR: 'asia', SG: 'asia', HK: 'asia', TH: 'asia',
  ID: 'asia', IN: 'asia', AE: 'asia', SA: 'asia', KZ: 'asia',
  AU: 'australia', NZ: 'pacific',
  EG: 'africa', ZA: 'africa', NG: 'africa', KE: 'africa',
};

export function getExpectedRegion(countryCode: string | null | undefined): TimezoneOption['region'] | null {
  if (!countryCode) return null;
  return COUNTRY_REGION[countryCode.toUpperCase()] ?? null;
}

export function getTimezoneRegion(timezone: string | null | undefined): TimezoneOption['region'] | null {
  if (!timezone) return null;
  const tz = TIMEZONES.find(t => t.value === timezone);
  return tz?.region ?? null;
}
