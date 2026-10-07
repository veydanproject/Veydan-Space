// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

package net.veydan.call

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.hardware.display.DisplayManager
import android.os.Handler
import android.os.Looper
import android.util.Log
import android.util.Size
import android.view.Display
import android.view.Surface
import androidx.camera.core.AspectRatio
import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.ImageProxy
import androidx.camera.core.resolutionselector.AspectRatioStrategy
import androidx.camera.core.resolutionselector.ResolutionSelector
import androidx.camera.core.resolutionselector.ResolutionStrategy
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.core.content.ContextCompat
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.LifecycleRegistry
import app.tauri.plugin.JSObject
import java.util.concurrent.Executors

/**
 * The camera of the phone, as frames for the engine of calls.
 *
 * CameraX gives YUV_420_888 images on a thread of its own; each one is
 * packed into NV21 (Y, then V and U interleaved, no padding: the layout
 * the engine's `PushedFrame` names) and handed to the Rust side of the
 * plugin (`Camera.push`, `Java_net_veydan_call_Camera_push`), with the
 * degrees it is to be turned to stand upright and the time it was taken.
 * The engine turns and converts it; the camera is not asked to.
 *
 * The camera lives on a lifecycle of this object's own, not the activity's:
 * a call goes on with the app's window gone, under the foreground service
 * (type `camera`). It is opened with the size asked for; the camera gives
 * the nearest it has, and the newest frame wins when the engine is slow.
 */
internal object Camera {
  const val FRONT = "front"
  const val BACK = "back"

  private val main = Handler(Looper.getMainLooper())
  private val analysis = Executors.newSingleThreadExecutor()

  /** A lifecycle the camera is bound to; started while the camera runs. */
  private val owner = object : LifecycleOwner {
    val registry = LifecycleRegistry(this)
    override val lifecycle: Lifecycle get() = registry
  }

  private var provider: ProcessCameraProvider? = null
  private var facing = FRONT
  private var wanted = Size(640, 360)
  private var running = false

  /** The use case bound now, whose target rotation follows the display. */
  private var use: ImageAnalysis? = null
  private var displays: DisplayManager? = null
  private val orientation = CameraOrientation()

  /**
   * The phone turned (auto-rotate on; the call's page is not locked to
   * portrait): the frames are to be turned for the orientation the
   * screen has now, or the peer sees the picture on its side. CameraX
   * takes the rotation once, at the making of the use case, and this
   * lifecycle is nobody's activity's; so the display is watched here, and
   * the use case's target follows it (`CameraOrientation` says when).
   */
  private val turned = object : DisplayManager.DisplayListener {
    override fun onDisplayAdded(displayId: Int) {}
    override fun onDisplayRemoved(displayId: Int) {}
    override fun onDisplayChanged(displayId: Int) {
      if (displayId != Display.DEFAULT_DISPLAY) return
      val rotation = displays?.getDisplay(displayId)?.rotation ?: return
      orientation.follow(rotation)?.let { use?.targetRotation = it }
    }
  }

  private fun displayRotation(): Int =
    displays?.getDisplay(Display.DEFAULT_DISPLAY)?.rotation ?: Surface.ROTATION_0

  /** What the camera did so far, for the measurement on the phone. */
  @Volatile private var frames = 0L
  @Volatile private var dropped = 0L
  @Volatile private var lastWidth = 0
  @Volatile private var lastHeight = 0
  @Volatile private var lastRotation = 0
  @Volatile private var packNs = 0L

  fun granted(context: Context): Boolean =
    ContextCompat.checkSelfPermission(context, Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED

  /**
   * Opens the camera `facing` at about `width`×`height`. On the main
   * thread. The frames start a moment later, when the camera is up.
   */
  fun start(context: Context, facing: String, width: Int, height: Int) {
    if (!granted(context)) throw IllegalStateException("the camera is not allowed")
    this.facing = facing
    wanted = Size(width.coerceIn(160, 1920), height.coerceIn(120, 1080))
    val ctx = context.applicationContext
    if (owner.registry.currentState == Lifecycle.State.INITIALIZED) owner.registry.currentState = Lifecycle.State.CREATED
    owner.registry.currentState = Lifecycle.State.STARTED
    running = true
    // Watched once per opening: a start on a camera that runs would
    // otherwise hear every turn twice.
    displays?.unregisterDisplayListener(turned)
    displays = ctx.getSystemService(DisplayManager::class.java)
    displays?.registerDisplayListener(turned, main)
    val future = ProcessCameraProvider.getInstance(ctx)
    future.addListener({
      if (!running) return@addListener
      try {
        provider = future.get()
        bind()
      } catch (e: Exception) {
        Log.w(CallState.TAG, "the camera would not open: ${e.javaClass.simpleName}: ${e.message}")
      }
    }, ContextCompat.getMainExecutor(ctx))
  }

  private fun bind() {
    val provider = provider ?: return
    provider.unbindAll()
    val selector = if (facing == BACK) CameraSelector.DEFAULT_BACK_CAMERA else CameraSelector.DEFAULT_FRONT_CAMERA
    // The shape first, then the size: a bare target size lets CameraX pick
    // any shape it finds nearest (a Pixel gave 1080×1080 for 640×360, four
    // times the pixels to pack and a square picture at the far end).
    val ratio = if (wanted.width * 9 == wanted.height * 16) AspectRatio.RATIO_16_9 else AspectRatio.RATIO_4_3
    val resolution = ResolutionSelector.Builder()
      .setAspectRatioStrategy(AspectRatioStrategy(ratio, AspectRatioStrategy.FALLBACK_RULE_AUTO))
      .setResolutionStrategy(ResolutionStrategy(wanted, ResolutionStrategy.FALLBACK_RULE_CLOSEST_HIGHER_THEN_LOWER))
      .build()
    // The rotation of the display as it is now, not the default CameraX
    // would take; `turned` keeps it current from here on.
    val rotation = displayRotation()
    val use = ImageAnalysis.Builder()
      .setResolutionSelector(resolution)
      .setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST)
      .setOutputImageFormat(ImageAnalysis.OUTPUT_IMAGE_FORMAT_YUV_420_888)
      .setTargetRotation(rotation)
      .build()
    use.setAnalyzer(analysis) { image -> image.use { pack(it) } }
    provider.bindToLifecycle(owner, selector, use)
    this.use = use
    orientation.bound(rotation)
    Log.i(CallState.TAG, "camera $facing open, asked for ${wanted.width}x${wanted.height}, display rotation $rotation")
  }

  /** The other camera, with the same size. */
  fun switch() {
    if (!running) return
    facing = if (facing == BACK) FRONT else BACK
    bind()
  }

  fun stop() {
    if (!running) return
    running = false
    displays?.unregisterDisplayListener(turned)
    displays = null
    use = null
    provider?.unbindAll()
    owner.registry.currentState = Lifecycle.State.CREATED
    Log.i(CallState.TAG, "camera closed after $frames frames ($dropped dropped)")
  }

  /** `{ running, facing, frames, dropped, width, height, rotation, packUs }`. */
  fun describe(): JSObject = JSObject()
    .put("running", running)
    .put("facing", facing)
    .put("frames", frames)
    .put("dropped", dropped)
    .put("width", lastWidth)
    .put("height", lastHeight)
    .put("rotation", lastRotation)
    // What packing one frame into NV21 costs on the camera's thread, microseconds, the last one.
    .put("packUs", packNs / 1000)

  private var nv21 = ByteArray(0)

  /**
   * One image into NV21 and to the Rust side. Row and pixel strides are
   * whatever the camera gives; the output has none. A frame nobody takes
   * (no engine) is counted as dropped by the Rust side.
   */
  private fun pack(image: ImageProxy) {
    val started = System.nanoTime()
    val w = image.width
    val h = image.height
    val cw = (w + 1) / 2
    val ch = (h + 1) / 2
    val size = w * h + 2 * cw * ch
    if (nv21.size != size) nv21 = ByteArray(size)
    val y = image.planes[0]
    val u = image.planes[1]
    val v = image.planes[2]
    // Y: row by row, the stride may be wider than the picture.
    val yb = y.buffer
    if (y.rowStride == w && y.pixelStride == 1) {
      yb.position(0)
      yb.get(nv21, 0, w * h)
    } else {
      for (row in 0 until h) {
        yb.position(row * y.rowStride)
        yb.get(nv21, row * w, w)
      }
    }
    // VU interleaved. The common case on a phone: U and V share one buffer
    // with a pixel stride of 2, V first — a straight copy of the V plane
    // (its last byte alone is missing, and is taken from U's plane).
    val vb = v.buffer
    val ub = u.buffer
    var at = w * h
    if (v.pixelStride == 2 && u.pixelStride == 2 && v.rowStride == u.rowStride && v.rowStride == cw * 2 && vb.capacity() >= 2 * cw * ch - 1) {
      vb.position(0)
      vb.get(nv21, at, 2 * cw * ch - 1)
      ub.position(2 * cw * ch - 2)
      nv21[size - 1] = ub.get()
    } else {
      for (row in 0 until ch) {
        var vi = row * v.rowStride
        var ui = row * u.rowStride
        for (col in 0 until cw) {
          nv21[at++] = vb.get(vi)
          nv21[at++] = ub.get(ui)
          vi += v.pixelStride
          ui += u.pixelStride
        }
      }
    }
    lastWidth = w
    lastHeight = h
    lastRotation = image.imageInfo.rotationDegrees
    frames++
    packNs = System.nanoTime() - started
    try {
      if (!push(nv21, w, h, lastRotation, image.imageInfo.timestamp / 1000)) dropped++
    } catch (e: UnsatisfiedLinkError) {
      dropped++
    }
  }

  /**
   * `Java_net_veydan_call_Camera_push` of the plugin's Rust side: the
   * frame as NV21, `rotation` in degrees clockwise, `timestampUs` of the
   * camera's monotonic clock. False when nothing took it.
   */
  @JvmStatic
  private external fun push(nv21: ByteArray, width: Int, height: Int, rotation: Int, timestampUs: Long): Boolean
}
