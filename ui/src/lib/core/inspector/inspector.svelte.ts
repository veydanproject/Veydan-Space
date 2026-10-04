// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

const PRESETS_KEY = 'rb_inspector_presets';
const MAX_PRESETS = 10;

export type GridSize = 4 | 8 | 12 | 'off' | 'custom';
export type ZoomLevel = 90 | 100 | 110 | 125;

export interface CustomGrid {
  columns: number;
  gutter: number;
  margin: number;
}

export interface Guide {
  id: number;
  axis: 'h' | 'v';
  position: number;
}

export interface InspectorPreset {
  name: string;
  gridSize: GridSize;
  zoom: ZoomLevel;
  showRulers: boolean;
  showGuides: boolean;
  customGrid: CustomGrid;
}

function loadPresets(): InspectorPreset[] {
  if (typeof localStorage === 'undefined') return [];
  try {
    const raw = localStorage.getItem(PRESETS_KEY);
    return raw ? JSON.parse(raw) : [];
  } catch {
    return [];
  }
}

function savePresetsToStorage(presets: InspectorPreset[]) {
  if (typeof localStorage !== 'undefined') {
    localStorage.setItem(PRESETS_KEY, JSON.stringify(presets));
  }
}

let guideCounter = 0;

class InspectorApp {
  enabled = $state(false);
  gridSize = $state<GridSize>('off');
  gridColor = $state('#6366f1');
  zoom = $state<ZoomLevel>(100);
  showRulers = $state(false);
  showGuides = $state(false);
  inspectMode = $state(false);
  outlineAll = $state(false);
  guides = $state<Guide[]>([]);
  customGrid = $state<CustomGrid>({ columns: 12, gutter: 16, margin: 24 });
  presets = $state<InspectorPreset[]>(loadPresets());

  toggle() {
    this.enabled = !this.enabled;
    if (!this.enabled) {
      this.inspectMode = false;
    }
  }

  addGuide(axis: 'h' | 'v', position: number) {
    this.guides = [...this.guides, { id: ++guideCounter, axis, position }];
  }

  moveGuide(id: number, position: number) {
    this.guides = this.guides.map((g) => (g.id === id ? { ...g, position } : g));
  }

  removeGuide(id: number) {
    this.guides = this.guides.filter((g) => g.id !== id);
  }

  clearGuides() {
    this.guides = [];
  }

  savePreset(name: string) {
    const preset: InspectorPreset = {
      name,
      gridSize: this.gridSize,
      zoom: this.zoom,
      showRulers: this.showRulers,
      showGuides: this.showGuides,
      customGrid: { ...this.customGrid },
    };
    const existing = this.presets.findIndex((p) => p.name === name);
    if (existing >= 0) {
      const next = [...this.presets];
      next[existing] = preset;
      this.presets = next;
    } else {
      this.presets = [...this.presets.slice(0, MAX_PRESETS - 1), preset];
    }
    savePresetsToStorage(this.presets);
  }

  loadPreset(name: string) {
    const preset = this.presets.find((p) => p.name === name);
    if (!preset) return;
    this.gridSize = preset.gridSize;
    this.zoom = preset.zoom;
    this.showRulers = preset.showRulers;
    this.showGuides = preset.showGuides;
    this.customGrid = { ...preset.customGrid };
  }

  deletePreset(name: string) {
    this.presets = this.presets.filter((p) => p.name !== name);
    savePresetsToStorage(this.presets);
  }
}

export const inspectorApp = new InspectorApp();
