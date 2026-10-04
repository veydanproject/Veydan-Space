// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

/** Open state of the notes' "more" sheet on the phone: the bottom bar's item opens it, the shell mounts it. */
class NotesMobileUi {
  moreOpen = $state(false);
}

export const notesMobileUi = new NotesMobileUi();
