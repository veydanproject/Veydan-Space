// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

package net.veydan.shell

/**
 * The decisions of ShareActivity that need no phone (crates/shell/src/shares.rs
 * is the Rust side): what of a share is taken, how its files are named in the
 * inbox, which folders of the inbox are stale. On the JVM in src/test.
 */
object ShareRules {
  /** The folder of the app's cache the copies of shared files wait in; the Rust side reads the same name. */
  const val INBOX = "share-inbox"

  /** More files than one share takes: the rest are left where they were. */
  const val MAX_FILES = 50

  /**
   * A file larger than this is refused before it fills the cache: the
   * messenger sends nothing larger (`MAX_SEND_BYTES` of its runtime, 1 GiB).
   */
  const val MAX_FILE_BYTES = 1L shl 30

  /** A folder of the inbox older than this was never taken (the app died first): deleted at the next share. */
  const val STALE_MS = 24L * 60 * 60 * 1000

  /** The longest text taken, in characters: what a chat sends. */
  const val MAX_TEXT = 64 * 1024

  /**
   * The name a shared file is kept and shown under: the last segment of
   * what its source calls it, without control characters, never a hidden
   * file nor a step out of its folder; `file-<n>` with the extension of
   * `mime` (`ext`) when the source gives no usable name.
   */
  fun fileName(display: String?, mime: String?, index: Int, ext: (String) -> String?): String {
    val last = display?.split('/', '\\')?.lastOrNull { it.isNotBlank() }?.filter { !it.isISOControl() }?.trim() ?: ""
    if (last.isNotEmpty() && last != "." && last != ".." && !last.startsWith('.')) return last.take(255)
    val suffix = mime?.let(ext)?.takeIf { it.isNotBlank() }?.let { ".$it" } ?: ""
    return "file-${index + 1}$suffix"
  }

  /** The mime of a file: its source's, else the share's own when that names one kind, else none. */
  fun mimeOf(ofFile: String?, ofShare: String?): String? {
    val own = ofFile?.takeIf { it.isNotBlank() && !it.endsWith("/*") && it != "*/*" }
    if (own != null) return own
    return ofShare?.takeIf { it.isNotBlank() && !it.endsWith("/*") && it != "*/*" }
  }

  /** The name of the inbox folder of a share that came at `now`: a hex stamp the Rust side checks (`[0-9a-f]+`). */
  fun folderName(now: Long): String = java.lang.Long.toHexString(now)

  /** Whether an inbox folder named `name` (a stamp of `folderName`) is stale at `now`; a name that is no stamp is. */
  fun stale(name: String, now: Long): Boolean {
    val made = name.toLongOrNull(16) ?: return true
    return now - made > STALE_MS || made > now + STALE_MS
  }

  /** The text of a share, cut to `MAX_TEXT`, none when blank. */
  fun textOf(text: CharSequence?): String? {
    val s = text?.toString()?.trim() ?: return null
    if (s.isEmpty()) return null
    if (s.length <= MAX_TEXT) return s
    var end = MAX_TEXT
    if (Character.isHighSurrogate(s[end - 1])) end -= 1
    return s.substring(0, end)
  }

  /** The subject of a share, none when blank or the same as the text. */
  fun subjectOf(subject: CharSequence?, text: String?): String? {
    val s = subject?.toString()?.trim()?.takeIf { it.isNotEmpty() } ?: return null
    return if (s == text) null else s.take(1024)
  }
}
