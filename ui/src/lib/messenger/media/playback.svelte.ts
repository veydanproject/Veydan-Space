// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Which recording is sounding now. Players watch it and pause themselves
// when another one starts.

class Playback {
  current = $state<string | null>(null);
}

export const playback = new Playback();
