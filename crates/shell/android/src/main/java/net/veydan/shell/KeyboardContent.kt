// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

package net.veydan.shell

import android.view.View
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.InputConnection
import androidx.core.view.inputmethod.EditorInfoCompat
import androidx.core.view.inputmethod.InputConnectionCompat

/**
 * Pictures a keyboard inserts (Gboard's clipboard, stickers, GIFs). A
 * keyboard offers them only to a field that names the types it takes, and
 * the WebView names none for a `textarea`: Gboard says "does not support
 * image pasting here". The WebView of every product overrides
 * `onCreateInputConnection` with this (wry's `RustWebView`, through
 * `WRY_RUSTWEBVIEW_CLASS_EXTENSION` in `.cargo/config.toml`): the field
 * names the types of pictures, and what the keyboard commits goes to the view's
 * `OnReceiveContentListener` (`ActivityDelegate.receivePictures`), which
 * takes it only while the page asks for pictures and gives anything else
 * back, so the keyboard says it was not taken.
 */
object KeyboardContent {
  private val PICTURES = arrayOf("image/*")

  @JvmStatic
  fun connect(view: View, outAttrs: EditorInfo, connection: InputConnection?): InputConnection? {
    if (connection == null) return null
    EditorInfoCompat.setContentMimeTypes(outAttrs, PICTURES)
    return InputConnectionCompat.createWrapper(view, connection, outAttrs)
  }
}
