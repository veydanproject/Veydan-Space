// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

/** Open state of the notes' "more" sheet on the phone: the bottom bar's item opens it, the shell mounts it. */
class NotesMobileUi {
  moreOpen = $state(false);
  /**
   * The address of the list shown last (`/notes` and its filter): a wide
   * window keeps that list beside an open note, whose own address has no filter.
   */
  listHref = '/notes';
}

export const notesMobileUi = new NotesMobileUi();
