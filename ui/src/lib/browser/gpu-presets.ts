// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

export interface GpuPreset {
  os: 'windows' | 'macos' | 'linux';
  label: string;
  vendor: string;
  renderer: string;
}

export const GPU_PRESETS: GpuPreset[] = [
  // Windows
  { os: 'windows', label: 'NVIDIA GTX 1050 Ti (Win)', vendor: 'NVIDIA Corporation', renderer: 'NVIDIA GeForce GTX 1050 Ti, or similar' },
  { os: 'windows', label: 'NVIDIA GTX 1080 (Win)',    vendor: 'NVIDIA Corporation', renderer: 'NVIDIA GeForce GTX 1080, or similar' },
  { os: 'windows', label: 'NVIDIA RTX 3060 (Win)',    vendor: 'NVIDIA Corporation', renderer: 'NVIDIA GeForce RTX 3060, or similar' },
  { os: 'windows', label: 'NVIDIA RTX 3080 (Win)',    vendor: 'NVIDIA Corporation', renderer: 'NVIDIA GeForce RTX 3080, or similar' },
  { os: 'windows', label: 'AMD RX 580 (Win)',         vendor: 'AMD',                renderer: 'AMD Radeon RX 580, or similar' },
  { os: 'windows', label: 'Intel UHD 630 (Win)',      vendor: 'Intel',              renderer: 'Intel(R) UHD Graphics 630' },
  { os: 'windows', label: 'Intel HD 620 (Win)',       vendor: 'Intel',              renderer: 'Intel(R) HD Graphics 620' },
  // macOS
  { os: 'macos',   label: 'Apple M1',                 vendor: 'Apple',              renderer: 'Apple M1, or similar' },
  { os: 'macos',   label: 'Apple M2',                 vendor: 'Apple',              renderer: 'Apple M2, or similar' },
  { os: 'macos',   label: 'Intel Iris Plus (Mac)',    vendor: 'Intel Inc.',         renderer: 'Intel(R) Iris(TM) Plus Graphics' },
  // Linux
  { os: 'linux',   label: 'NVIDIA GTX 1060 (Lin)',    vendor: 'NVIDIA Corporation', renderer: 'NVIDIA GeForce GTX 1060/PCIe/SSE2' },
  { os: 'linux',   label: 'AMD RX 5500 XT (Lin)',     vendor: 'X.Org',              renderer: 'AMD Radeon RX 5500 XT (navi14, LLVM 15.0.7, DRM 3.47)' },
  { os: 'linux',   label: 'Mesa llvmpipe (Lin)',      vendor: 'Mesa/X.org',         renderer: 'llvmpipe (LLVM 15.0.7, 256 bits)' },
];

/** Preset OS identifier → GPU OS string */
export function presetToGpuOs(fingerprintPreset: string): 'windows' | 'macos' | 'linux' {
  if (fingerprintPreset.startsWith('win')) return 'windows';
  if (fingerprintPreset === 'macos') return 'macos';
  return 'linux';
}

export function gpuPresetByValues(vendor: string | null | undefined, renderer: string | null | undefined): GpuPreset | undefined {
  if (!vendor || !renderer) return undefined;
  return GPU_PRESETS.find(p => p.vendor === vendor && p.renderer === renderer);
}

/** Возвращает дефолтный GPU для пресета ОС, или undefined для linux */
export function defaultGpuForPreset(fingerprintPreset: string): GpuPreset | undefined {
  const os = presetToGpuOs(fingerprintPreset);
  if (os === 'linux') return undefined;
  const defaults: Record<string, string> = {
    windows: 'Intel(R) UHD Graphics 630',
    macos: 'Apple M1, or similar',
  };
  const renderer = defaults[os];
  return GPU_PRESETS.find(g => g.os === os && g.renderer === renderer);
}
