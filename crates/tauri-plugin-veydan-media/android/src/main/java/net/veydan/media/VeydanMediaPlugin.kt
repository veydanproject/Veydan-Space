// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

package net.veydan.media

import android.Manifest
import android.app.Activity
import android.content.ActivityNotFoundException
import android.content.ClipData
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.provider.MediaStore
import android.provider.Settings
import android.util.Log
import android.webkit.WebView
import androidx.activity.result.ActivityResult
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.result.contract.ActivityResultContracts
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat
import androidx.core.content.FileProvider
import app.tauri.PermissionState
import app.tauri.annotation.ActivityCallback
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.Permission
import app.tauri.annotation.PermissionCallback
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSArray
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.io.File
import java.util.concurrent.Executors

@InvokeArg
class PageArgs {
  var cursor: String? = null
  var limit: Int = 60
  /** `image`, `video`, or both when absent. */
  var kind: String? = null
  var album: String? = null
}

@InvokeArg
class ThumbArgs {
  var uri: String? = null
  var side: Int = 256
}

@InvokeArg
class ReadableArgs {
  var uris: Array<String>? = null
}

@InvokeArg
class CaptureArgs {
  /** `photo` or `video`. */
  var kind: String? = null
}

@InvokeArg
class PickArgs {
  var multiple: Boolean = true
  /** `image`, `video`, `media`. */
  var kind: String? = null
}

@InvokeArg
class ConvertArgs {
  var src: String? = null
  var dst: String? = null
  var maxPixels: Long = 50_000_000
  var quality: Int = 92
}

@InvokeArg
class ForgetArgs {
  var uri: String? = null
}

@InvokeArg
class ShareArgs {
  /** `send` (the system's list of apps), `sms`, `mail`. */
  var via: String? = null
  var text: String? = null
  /** The subject of a letter. */
  var subject: String? = null
}

@InvokeArg
class SavePictureArgs {
  /** The PNG, as base64. */
  var data: String? = null
  /** Its file name in the gallery. */
  var name: String? = null
}

private const val TAG = "VeydanMedia"
private const val CAMERA = "camera"
private const val IMAGES = "images"
private const val VIDEO = "video"
private const val SELECTED = "selected"
private const val STORAGE = "storage"
private const val WRITE = "write"
private const val PREFS = "veydan-media"
private const val ASKED = "asked"

/**
 * The phone's gallery for the sheet of the paperclip (ROO-110 1b): the
 * access to photos and videos, pages of the newest of them and the albums,
 * thumbnails as files, the system camera, the system's Photo Picker and
 * the photos of HEIC written as JPEG. Called from the app's Rust code only;
 * nothing here is open to the page.
 *
 * Nothing is asked of the user before a tap: the access is only read until
 * the page asks for it (`request`), the camera's permission until the tile
 * of the camera is tapped (`capture`).
 */
@TauriPlugin(
  permissions = [
    Permission(strings = [MediaPermissions.IMAGES], alias = IMAGES),
    Permission(strings = [MediaPermissions.VIDEO], alias = VIDEO),
    Permission(strings = [MediaPermissions.USER_SELECTED], alias = SELECTED),
    Permission(strings = [MediaPermissions.STORAGE], alias = STORAGE),
    // Android 9 and older only (maxSdkVersion 28 in the products): the public Pictures of «Сохранить».
    Permission(strings = [Manifest.permission.WRITE_EXTERNAL_STORAGE], alias = WRITE),
    Permission(strings = [Manifest.permission.CAMERA], alias = CAMERA),
  ]
)
class VeydanMediaPlugin(private val activity: Activity) : Plugin(activity) {
  private val gallery = Gallery(activity)
  private val thumbs = Thumbs(activity)
  // MediaStore, the decoder and the disk off the thread that called.
  private val work = Executors.newFixedThreadPool(3)
  private val captureDir = File(activity.cacheDir, "attach-capture")
  /** The file the camera app is writing into, while it is open. */
  private var capturing: File? = null

  override fun load(webView: WebView) {
    // What a run before left: thumbnails and shots never imported.
    thumbs.clear { work.execute(it) }
    work.execute { captureDir.listFiles()?.forEach { it.delete() } }
  }

  private fun background(invoke: Invoke, job: () -> JSObject) {
    work.execute {
      try {
        invoke.resolve(job())
      } catch (e: Exception) {
        Log.w(TAG, "media command failed: ${e.javaClass.simpleName}: ${e.message}")
        invoke.reject(e.message ?: e.javaClass.simpleName)
      }
    }
  }

  // ─── Access ──────────────────────────────────────────────────────────────

  private fun granted(permission: String) =
    ContextCompat.checkSelfPermission(activity, permission) == PackageManager.PERMISSION_GRANTED

  private fun accessNow(): MediaAccess = MediaPermissions.access(Build.VERSION.SDK_INT, ::granted)

  private fun accessAnswer(): JSObject = JSObject().put("access", accessNow().word)

  /** What the grid may show now; asks nothing. Read again whenever the page looks (a change in the settings is seen without a restart). */
  @Command
  fun access(invoke: Invoke) {
    invoke.resolve(accessAnswer())
  }

  private fun askedAliases(): Array<String> = when {
    Build.VERSION.SDK_INT >= MediaPermissions.SDK_SELECTED -> arrayOf(IMAGES, VIDEO, SELECTED)
    Build.VERSION.SDK_INT >= MediaPermissions.SDK_MEDIA -> arrayOf(IMAGES, VIDEO)
    else -> arrayOf(STORAGE)
  }

  /**
   * The user tapped «Разрешить доступ» (or «Выбрать ещё»): the system's
   * question, when it may still be shown; else the app's page of settings,
   * where the access is given by hand. Answers the access after it.
   */
  @Command
  fun request(invoke: Invoke) {
    val prefs = activity.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
    val access = accessNow()
    val rationale = MediaPermissions.asked(Build.VERSION.SDK_INT).any {
      ActivityCompat.shouldShowRequestPermissionRationale(activity, it)
    }
    if (!MediaPermissions.mayAsk(access, prefs.getBoolean(ASKED, false), rationale)) {
      if (access == MediaAccess.DENIED) openSettings()
      invoke.resolve(accessAnswer())
      return
    }
    prefs.edit().putBoolean(ASKED, true).apply()
    requestPermissionForAliases(askedAliases(), invoke, "accessAnswered")
  }

  @PermissionCallback
  fun accessAnswered(invoke: Invoke) {
    invoke.resolve(accessAnswer())
  }

  private fun openSettings() {
    try {
      activity.startActivity(
        Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.fromParts("package", activity.packageName, null))
      )
    } catch (e: ActivityNotFoundException) {
      Log.w(TAG, "no page of settings: ${e.message}")
    }
  }

  // ─── The grid ────────────────────────────────────────────────────────────

  /** A page of the newest photos and videos after `cursor`: `{ items, cursor }`, no cursor after the last. */
  @Command
  fun recent(invoke: Invoke) {
    val args = invoke.parseArgs(PageArgs::class.java)
    background(invoke) {
      if (accessNow() == MediaAccess.DENIED) return@background JSObject().put("items", JSArray())
      val page = gallery.page(args.cursor, args.limit, args.kind, args.album)
      val items = JSArray()
      for (it in page.items) {
        items.put(
          JSObject()
            .put("id", it.id.toString())
            .put("kind", if (it.video) "video" else "image")
            .put("path", it.uri)
            .put("width", it.width)
            .put("height", it.height)
            .apply { if (it.video) put("durationMs", it.durationMs) }
        )
      }
      JSObject().put("items", items).apply { page.next?.let { put("cursor", it) } }
    }
  }

  @Command
  fun albums(invoke: Invoke) {
    background(invoke) {
      val list = JSArray()
      if (accessNow() != MediaAccess.DENIED) {
        for (a in gallery.albums()) list.put(JSObject().put("id", a.id).put("title", a.title).put("count", a.count))
      }
      JSObject().put("albums", list)
    }
  }

  /**
   * The file of the thumbnail of an item of the grid (`{ path }`); the app
   * hands it to the page. An item out of the access now: `{ gone }`, the
   * files deleted of it, for the app to forget their addresses.
   */
  @Command
  fun thumb(invoke: Invoke) {
    val args = invoke.parseArgs(ThumbArgs::class.java)
    val uri = args.uri
    if (uri == null || !uri.startsWith("content://media/")) {
      invoke.reject("not an item of the gallery")
      return
    }
    background(invoke) {
      try {
        JSObject().put("path", thumbs.of(uri, args.side).absolutePath)
      } catch (e: NoAccess) {
        val gone = JSArray()
        for (f in e.gone) gone.put(f.absolutePath)
        JSObject().put("gone", gone)
      }
    }
  }

  /**
   * Which of the items chosen in the grid the app may still read
   * (`{ readable }`), each asked of MediaStore by its own row, whatever album
   * or page the grid shows: a partial access the user changed lets out only
   * what it no longer holds. The thumbnails kept of the others are deleted
   * (`{ gone }`, for the app to forget their addresses).
   */
  @Command
  fun readable(invoke: Invoke) {
    val uris = invoke.parseArgs(ReadableArgs::class.java).uris?.toList() ?: emptyList()
    background(invoke) {
      val ok = stillReadable(uris) { thumbs.readable(Uri.parse(it)) }
      val kept = ok.toSet()
      val gone = JSArray()
      for (uri in uris) {
        if (uri in kept || !uri.startsWith("content://media/")) continue
        for (f in thumbs.drop(uri)) gone.put(f.absolutePath)
      }
      val readable = JSArray()
      for (uri in ok) readable.put(uri)
      JSObject().put("readable", readable).put("gone", gone)
    }
  }

  // ─── The camera ──────────────────────────────────────────────────────────

  /**
   * The system's camera app takes a photo or a video into
   * `cacheDir/attach-capture/`, handed over through the app's FileProvider.
   * The app declares CAMERA (calls), so the camera app is not even started
   * without it (SecurityException): it is asked for first, on this tap.
   * Answers `{ path }`, the `content://` address of the shot, or no path
   * when it was given up; `camera_denied`, `no_camera_app` refuse.
   */
  @Command
  fun capture(invoke: Invoke) {
    if (getPermissionState(CAMERA) == PermissionState.GRANTED) {
      startCamera(invoke)
    } else {
      requestPermissionForAlias(CAMERA, invoke, "cameraAnswered")
    }
  }

  @PermissionCallback
  fun cameraAnswered(invoke: Invoke) {
    if (getPermissionState(CAMERA) == PermissionState.GRANTED) startCamera(invoke) else invoke.reject("camera_denied")
  }

  private fun startCamera(invoke: Invoke) {
    val video = invoke.parseArgs(CaptureArgs::class.java).kind == "video"
    captureDir.mkdirs()
    val file = File(captureDir, "${System.currentTimeMillis()}.${if (video) "mp4" else "jpg"}")
    val uri = FileProvider.getUriForFile(activity, "${activity.packageName}.fileprovider", file)
    val intent = Intent(if (video) MediaStore.ACTION_VIDEO_CAPTURE else MediaStore.ACTION_IMAGE_CAPTURE)
      .putExtra(MediaStore.EXTRA_OUTPUT, uri)
      .addFlags(Intent.FLAG_GRANT_WRITE_URI_PERMISSION or Intent.FLAG_GRANT_READ_URI_PERMISSION)
    // The grant reaches the camera app through the clip on every version.
    intent.clipData = ClipData.newRawUri("", uri)
    capturing = file
    try {
      startActivityForResult(invoke, intent, "captured")
    } catch (e: ActivityNotFoundException) {
      capturing = null
      invoke.reject("no_camera_app")
    }
  }

  @ActivityCallback
  fun captured(invoke: Invoke, result: ActivityResult) {
    val file = capturing
    capturing = null
    if (file == null) {
      invoke.resolve(JSObject())
      return
    }
    if (result.resultCode == Activity.RESULT_OK && file.isFile && file.length() > 0) {
      val uri = FileProvider.getUriForFile(activity, "${activity.packageName}.fileprovider", file)
      invoke.resolve(JSObject().put("path", uri.toString()))
    } else {
      file.delete()
      // A camera app that kept the video to itself names where it put it (its gallery: not ours to delete).
      val own = result.data?.data?.takeIf { result.resultCode == Activity.RESULT_OK && it.scheme == "content" }
      invoke.resolve(if (own != null) JSObject().put("path", own.toString()) else JSObject())
    }
  }

  /** A shot was imported (copied into the messenger's folder): its file in the cache goes. Only a file of `attach-capture`. */
  @Command
  fun forgetCapture(invoke: Invoke) {
    val uri = invoke.parseArgs(ForgetArgs::class.java).uri?.let(Uri::parse)
    val name = uri?.lastPathSegment
    if (uri != null && uri.authority == "${activity.packageName}.fileprovider" && name != null &&
      uri.pathSegments.dropLast(1).lastOrNull() == captureDir.name && !name.contains('/') && !name.startsWith(".")
    ) {
      File(captureDir, name).delete()
    }
    invoke.resolve()
  }

  // ─── The system's Photo Picker ───────────────────────────────────────────

  /** The system's Photo Picker (no permission needed): `{ paths }`, none when given up. */
  @Command
  fun pickSystem(invoke: Invoke) {
    val args = invoke.parseArgs(PickArgs::class.java)
    val type = when (args.kind) {
      "image" -> ActivityResultContracts.PickVisualMedia.ImageOnly
      "video" -> ActivityResultContracts.PickVisualMedia.VideoOnly
      else -> ActivityResultContracts.PickVisualMedia.ImageAndVideo
    }
    val request = PickVisualMediaRequest.Builder().setMediaType(type).build()
    val intent = if (args.multiple) {
      ActivityResultContracts.PickMultipleVisualMedia().createIntent(activity, request)
    } else {
      ActivityResultContracts.PickVisualMedia().createIntent(activity, request)
    }
    try {
      startActivityForResult(invoke, intent, if (args.multiple) "pickedMany" else "pickedOne")
    } catch (e: ActivityNotFoundException) {
      invoke.reject("no_picker")
    }
  }

  private fun answerPaths(invoke: Invoke, uris: List<Uri>) {
    val paths = JSArray()
    for (u in uris) paths.put(u.toString())
    invoke.resolve(JSObject().put("paths", paths))
  }

  @ActivityCallback
  fun pickedMany(invoke: Invoke, result: ActivityResult) {
    answerPaths(invoke, ActivityResultContracts.PickMultipleVisualMedia().parseResult(result.resultCode, result.data))
  }

  @ActivityCallback
  fun pickedOne(invoke: Invoke, result: ActivityResult) {
    answerPaths(invoke, listOfNotNull(ActivityResultContracts.PickVisualMedia().parseResult(result.resultCode, result.data)))
  }

  // ─── HEIC → JPEG ─────────────────────────────────────────────────────────

  /** The photo at `src` as a JPEG at `dst` (Pictures.kt); the app gives paths in its own folder. */
  @Command
  fun convert(invoke: Invoke) {
    val args = invoke.parseArgs(ConvertArgs::class.java)
    val src = args.src
    val dst = args.dst
    if (src == null || dst == null) {
      invoke.reject("no paths")
      return
    }
    background(invoke) {
      Pictures.toJpeg(File(src), File(dst), args.maxPixels, args.quality)
      JSObject()
    }
  }

  // ─── Sharing a text ──────────────────────────────────────────────────────

  /**
   * A text into another app: the system's list of apps (`send`), the SMS
   * app with the text and no addressee (`sms`), a letter with a subject
   * (`mail`). The user picks the app and the addressee there; nothing comes
   * back. Refused with `no_app` when the phone has none for it.
   */
  @Command
  fun shareText(invoke: Invoke) {
    val args = invoke.parseArgs(ShareArgs::class.java)
    val text = args.text
    if (text.isNullOrEmpty()) {
      invoke.reject("no text")
      return
    }
    val intent = when (args.via) {
      "send" -> Intent.createChooser(
        Intent(Intent.ACTION_SEND).setType("text/plain").putExtra(Intent.EXTRA_TEXT, text),
        null,
      )
      "sms" -> Intent(Intent.ACTION_SENDTO, Uri.parse("smsto:")).putExtra("sms_body", text)
      "mail" -> {
        val subject = args.subject.orEmpty()
        // Both in the address and as extras: some mail apps read only one of them.
        Intent(Intent.ACTION_SENDTO, Uri.parse("mailto:?subject=${Uri.encode(subject)}&body=${Uri.encode(text)}"))
          .putExtra(Intent.EXTRA_SUBJECT, subject)
          .putExtra(Intent.EXTRA_TEXT, text)
      }
      else -> {
        invoke.reject("unknown way")
        return
      }
    }
    activity.runOnUiThread {
      try {
        activity.startActivity(intent)
        invoke.resolve(JSObject())
      } catch (e: ActivityNotFoundException) {
        invoke.reject("no_app")
      }
    }
  }

  // ─── Save a picture ──────────────────────────────────────────────────────

  private fun mayWritePictures() =
    Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q || granted(Manifest.permission.WRITE_EXTERNAL_STORAGE)

  /**
   * A PNG of the app (a QR code, ROO-297) into the gallery, Pictures/Veydan.
   * Android 10+ asks nothing; an older one asks to write the storage, at
   * this tap only, and is refused with `storage_denied` when the user says no.
   */
  @Command
  fun savePicture(invoke: Invoke) {
    if (!mayWritePictures()) {
      requestPermissionForAlias(WRITE, invoke, "savePictureAnswered")
      return
    }
    writePicture(invoke)
  }

  @PermissionCallback
  fun savePictureAnswered(invoke: Invoke) {
    if (!mayWritePictures()) {
      invoke.reject("storage_denied")
      return
    }
    writePicture(invoke)
  }

  private fun writePicture(invoke: Invoke) {
    val args = invoke.parseArgs(SavePictureArgs::class.java)
    val data = args.data
    val name = args.name
    if (data.isNullOrEmpty() || name.isNullOrEmpty()) {
      invoke.reject("no picture")
      return
    }
    background(invoke) {
      Pictures.saveToGallery(activity, android.util.Base64.decode(data, android.util.Base64.DEFAULT), name)
      JSObject()
    }
  }
}
