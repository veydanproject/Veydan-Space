// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

package net.veydan.push

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Log
import java.io.File
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/**
 * The keys the push handler opens messages with, kept for the process a
 * push starts, which cannot open the app's vault.
 *
 * The app hands them over as a bundle; they are sealed with a key that
 * lives in the phone's own key store and never leaves it, and put in a
 * file no backup takes. What is in the bundle is the app's business: this
 * side only keeps it, gives it back, and throws it away when told.
 */
internal object Keys {
  private const val ALIAS = "veydan.push.keys"
  private const val FILE = "push/keys.bin"
  private const val IV_LEN = 12
  private const val TAG_BITS = 128

  private fun file(context: Context) = File(context.noBackupFilesDir, FILE)

  /** Whether the app has handed keys over (they may still fail to open). */
  fun present(context: Context): Boolean = file(context).exists()

  /** Seals and keeps the bundle; a bundle kept before is replaced. */
  fun store(context: Context, bundle: ByteArray) {
    val cipher = Cipher.getInstance("AES/GCM/NoPadding")
    cipher.init(Cipher.ENCRYPT_MODE, key(create = true))
    val sealed = cipher.iv + cipher.doFinal(bundle)
    val target = file(context)
    target.parentFile?.mkdirs()
    // Written whole or not at all: a push may read while the app writes.
    val fresh = File(target.parentFile, "keys.bin.new")
    fresh.writeBytes(sealed)
    if (!fresh.renameTo(target)) {
      fresh.delete()
      throw IllegalStateException("the keys could not be kept")
    }
  }

  fun clear(context: Context) {
    file(context).delete()
  }

  /**
   * The bundle, or null when there is none or it cannot be opened: a key
   * store that changed under it (the app's data restored elsewhere, the
   * phone not unlocked yet since it started) is treated as no keys, and
   * what cannot be opened is thrown away.
   */
  fun read(context: Context): ByteArray? {
    val target = file(context)
    if (!target.exists()) return null
    return try {
      val sealed = target.readBytes()
      val cipher = Cipher.getInstance("AES/GCM/NoPadding")
      cipher.init(Cipher.DECRYPT_MODE, key(create = false), GCMParameterSpec(TAG_BITS, sealed, 0, IV_LEN))
      cipher.doFinal(sealed, IV_LEN, sealed.size - IV_LEN)
    } catch (e: Exception) {
      Log.w(PushState.TAG, "the kept keys could not be opened (${e.javaClass.simpleName}); forgotten")
      target.delete()
      null
    }
  }

  private fun key(create: Boolean): SecretKey {
    val store = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
    (store.getKey(ALIAS, null) as? SecretKey)?.let { return it }
    if (!create) throw IllegalStateException("no key in the key store")
    val generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore")
    generator.init(
      KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
        .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
        .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
        .setKeySize(256)
        // Pushes come with the screen off; nothing here asks the user.
        .build()
    )
    return generator.generateKey()
  }
}
