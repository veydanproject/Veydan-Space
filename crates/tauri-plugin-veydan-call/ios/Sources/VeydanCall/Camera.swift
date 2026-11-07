// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import AVFoundation
import CoreMedia
import CoreVideo
import Foundation
import Tauri
import UIKit

/**
 * `veydan_call_camera_push` of the plugin's Rust side (src/lib.rs): one
 * frame, tightly packed, in `format` (0: NV12), `rotation` in degrees
 * clockwise, `timestampUs` of the capture's clock. False when nothing took
 * it.
 *
 * Declared by its symbol: the Rust function is in the same binary, and a
 * Swift package has no header to declare it in. Its arguments are a
 * pointer and plain integers and its answer a bool, which the calling
 * conventions of Swift and of C pass alike on arm64.
 */
@_silgen_name("veydan_call_camera_push")
private func veydanCallCameraPush(
  _ data: UnsafePointer<UInt8>?, _ len: Int, _ width: Int32, _ height: Int32,
  _ rotation: Int32, _ format: UInt32, _ timestampUs: Int64
) -> Bool

/** The layout `veydanCallCameraPush` is told: the camera's own `420YpCbCr8BiPlanarVideoRange`. */
private let formatNv12: UInt32 = 0

/**
 * The camera of the phone, as frames for the engine of calls (the twin of
 * Camera.kt).
 *
 * `AVCaptureSession` gives NV12 buffers (`420YpCbCr8BiPlanarVideoRange`:
 * Y, then U and V interleaved, the limited range the engine's I420 has)
 * on a queue of its own; the rows of both planes are copied without their
 * padding (the layout the engine's `PushedFrame` names) and handed to the
 * Rust side of the plugin, with the degrees the frame is to be turned to
 * stand upright and the time it was taken. The engine turns and converts
 * it; the camera is not asked to.
 *
 * It is opened at the format nearest the size asked for, of its shape
 * (`CameraFormats.pick`), at 30 frames a second when the format has it;
 * a late frame is dropped, so the newest wins when the engine is slow. A
 * camera does not run while the app is in the background (iOS stops the
 * session, and starts it again when the app comes back).
 *
 * `start`, `switch` and `stop` come on the main thread; the session is
 * configured on the camera's queue, where the frames come too.
 */
final class Camera: NSObject, AVCaptureVideoDataOutputSampleBufferDelegate {
  static let shared = Camera()

  static let front = "front"
  static let back = "back"

  private let session = AVCaptureSession()
  private let output = AVCaptureVideoDataOutput()
  private let queue = DispatchQueue(label: "net.veydan.call.camera")
  /** On `queue`. */
  private var input: AVCaptureDeviceInput?

  /** What the camera does and did so far, read from any thread. */
  private let lock = NSLock()
  private var running = false
  private var facing = Camera.front
  private var wanted = (width: 640, height: 360)
  private var frames: Int64 = 0
  private var dropped: Int64 = 0
  private var lastWidth = 0
  private var lastHeight = 0
  private var lastRotation = 0
  private var packNs: UInt64 = 0
  /** How the phone is held, kept from the main thread for the camera's queue. */
  private var held = CameraRotation.Held.portrait
  private var orientationObserver: NSObjectProtocol?

  /** The packed frame, reused from one to the next; on `queue`. */
  private var packed = [UInt8]()

  private override init() {
    super.init()
    // The camera leaves the audio session alone: it is the call's.
    session.automaticallyConfiguresApplicationAudioSession = false
  }

  private func locked<T>(_ work: () -> T) -> T {
    lock.lock()
    defer { lock.unlock() }
    return work()
  }

  static func granted() -> Bool {
    AVCaptureDevice.authorizationStatus(for: .video) == .authorized
  }

  /**
   * Opens the camera `facing` at about `width`×`height`. On the main
   * thread. The frames start a moment later, when the camera is up.
   */
  func start(facing: String, width: Int, height: Int) throws {
    guard Camera.granted() else { throw CallError("the camera is not allowed") }
    locked {
      self.facing = facing
      wanted = (min(max(width, 160), 1920), min(max(height, 120), 1080))
      running = true
    }
    followOrientation()
    queue.async {
      self.configure()
      if !self.session.isRunning { self.session.startRunning() }
    }
  }

  /** The other camera, with the same size. */
  func switchFacing() {
    let on = locked { () -> Bool in
      guard running else { return false }
      facing = facing == Camera.back ? Camera.front : Camera.back
      return true
    }
    if on { queue.async { self.configure() } }
  }

  func stop() {
    let was = locked { () -> Bool in
      let was = running
      running = false
      return was
    }
    guard was else { return }
    if let observer = orientationObserver {
      NotificationCenter.default.removeObserver(observer)
    }
    orientationObserver = nil
    UIDevice.current.endGeneratingDeviceOrientationNotifications()
    queue.async {
      self.session.stopRunning()
      self.session.beginConfiguration()
      if let input = self.input { self.session.removeInput(input) }
      self.input = nil
      self.session.commitConfiguration()
      let (frames, dropped) = self.locked { (self.frames, self.dropped) }
      callLog("camera closed after \(frames) frames (\(dropped) dropped)")
    }
  }

  /** `{ running, facing, frames, dropped, width, height, rotation, packUs }`, as the Kotlin side's. */
  func describe() -> JsonObject {
    locked {
      [
        "running": running,
        "facing": facing,
        "frames": Int(frames),
        "dropped": Int(dropped),
        "width": lastWidth,
        "height": lastHeight,
        "rotation": lastRotation,
        // What copying one frame out of the camera's buffer costs on its queue, microseconds, the last one.
        "packUs": Int(packNs / 1000),
      ]
    }
  }

  /** How the phone is held follows the device's orientation from now on. On the main thread. */
  private func followOrientation() {
    let device = UIDevice.current
    if orientationObserver == nil {
      device.beginGeneratingDeviceOrientationNotifications()
      orientationObserver = NotificationCenter.default.addObserver(
        forName: UIDevice.orientationDidChangeNotification, object: nil, queue: .main
      ) { [weak self] _ in
        self?.orientationChanged()
      }
    }
    orientationChanged()
  }

  private func orientationChanged() {
    let now: CameraRotation.Held
    switch UIDevice.current.orientation {
    case .portrait: now = .portrait
    case .portraitUpsideDown: now = .portraitUpsideDown
    case .landscapeLeft: now = .landscapeLeft
    case .landscapeRight: now = .landscapeRight
    // Face up, face down, unknown: the last known way.
    default: return
    }
    locked { held = now }
  }

  /** The input for the camera wanted now, at its format. On `queue`. */
  private func configure() {
    let (facing, wanted, running) = locked { (self.facing, self.wanted, self.running) }
    guard running else { return }
    session.beginConfiguration()
    defer { session.commitConfiguration() }
    if let old = input {
      session.removeInput(old)
      input = nil
    }
    let position: AVCaptureDevice.Position = facing == Camera.back ? .back : .front
    guard let device = AVCaptureDevice.default(.builtInWideAngleCamera, for: .video, position: position) else {
      callLog("the camera would not open: no \(facing) camera")
      return
    }
    do {
      let newInput = try AVCaptureDeviceInput(device: device)
      // The format of the device decides the size, not a preset of the session.
      session.sessionPreset = .inputPriority
      guard session.canAddInput(newInput) else {
        callLog("the camera would not open: the session takes no \(facing) camera")
        return
      }
      session.addInput(newInput)
      input = newInput
      if !session.outputs.contains(output) {
        output.videoSettings = [kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange]
        output.alwaysDiscardsLateVideoFrames = true
        output.setSampleBufferDelegate(self, queue: queue)
        if session.canAddOutput(output) { session.addOutput(output) }
      }
      try chooseFormat(of: device, wanted: wanted)
      callLog("camera \(facing) open, asked for \(wanted.width)x\(wanted.height)")
    } catch {
      callLog("the camera would not open: \(error)")
    }
  }

  /**
   * The device's format for `wanted`: of the formats that give the
   * camera's own NV12 at 30 frames a second (any format when none does),
   * the one `CameraFormats.pick` takes; 30 frames a second when it has
   * them.
   */
  private func chooseFormat(of device: AVCaptureDevice, wanted: (width: Int, height: Int)) throws {
    let all = device.formats
    let fitting = all.filter { format in
      CMFormatDescriptionGetMediaSubType(format.formatDescription) == kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange
        && format.videoSupportedFrameRateRanges.contains { $0.maxFrameRate >= 30 }
    }
    let formats = fitting.isEmpty ? all : fitting
    let sizes = formats.map { format -> (width: Int, height: Int) in
      let d = CMVideoFormatDescriptionGetDimensions(format.formatDescription)
      return (Int(d.width), Int(d.height))
    }
    guard let index = CameraFormats.pick(sizes, wanted: wanted) else { return }
    let format = formats[index]
    try device.lockForConfiguration()
    defer { device.unlockForConfiguration() }
    device.activeFormat = format
    let thirty = CMTime(value: 1, timescale: 30)
    if format.videoSupportedFrameRateRanges.contains(where: { $0.minFrameRate <= 30 && $0.maxFrameRate >= 30 }) {
      device.activeVideoMinFrameDuration = thirty
      device.activeVideoMaxFrameDuration = thirty
    }
  }

  // MARK: AVCaptureVideoDataOutputSampleBufferDelegate

  /**
   * One buffer into tight NV12 and to the Rust side. The rows of a plane
   * may be wider than the picture; the output has no padding. A frame
   * nobody takes (no engine) is counted as dropped.
   */
  func captureOutput(_ output: AVCaptureOutput, didOutput sampleBuffer: CMSampleBuffer, from connection: AVCaptureConnection) {
    guard let pixels = CMSampleBufferGetImageBuffer(sampleBuffer),
      CVPixelBufferGetPixelFormatType(pixels) == kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange,
      CVPixelBufferGetPlaneCount(pixels) == 2
    else { return }
    let started = DispatchTime.now().uptimeNanoseconds
    CVPixelBufferLockBaseAddress(pixels, .readOnly)
    defer { CVPixelBufferUnlockBaseAddress(pixels, .readOnly) }
    let w = CVPixelBufferGetWidth(pixels)
    let h = CVPixelBufferGetHeight(pixels)
    let cw = (w + 1) / 2
    let ch = (h + 1) / 2
    let size = w * h + 2 * cw * ch
    guard let yBase = CVPixelBufferGetBaseAddressOfPlane(pixels, 0),
      let uvBase = CVPixelBufferGetBaseAddressOfPlane(pixels, 1),
      CVPixelBufferGetHeightOfPlane(pixels, 1) >= ch
    else { return }
    let yStride = CVPixelBufferGetBytesPerRowOfPlane(pixels, 0)
    let uvStride = CVPixelBufferGetBytesPerRowOfPlane(pixels, 1)
    guard yStride >= w, uvStride >= 2 * cw else { return }
    if packed.count != size { packed = [UInt8](repeating: 0, count: size) }
    let (held, front) = locked { (self.held, self.facing == Camera.front) }
    let rotation = CameraRotation.degrees(held, front: front)
    let time = CMSampleBufferGetPresentationTimeStamp(sampleBuffer)
    let timestampUs = time.isValid ? Int64(CMTimeGetSeconds(time) * 1_000_000) : 0
    let taken = packed.withUnsafeMutableBytes { (out: UnsafeMutableRawBufferPointer) -> Bool in
      let dst = out.baseAddress!
      // Y: in one copy when the rows have no padding, row by row otherwise.
      if yStride == w {
        memcpy(dst, yBase, w * h)
      } else {
        for row in 0..<h { memcpy(dst + row * w, yBase + row * yStride, w) }
      }
      // UV interleaved, a row of `cw` pairs.
      let uvDst = dst + w * h
      if uvStride == 2 * cw {
        memcpy(uvDst, uvBase, 2 * cw * ch)
      } else {
        for row in 0..<ch { memcpy(uvDst + row * 2 * cw, uvBase + row * uvStride, 2 * cw) }
      }
      let bytes = dst.assumingMemoryBound(to: UInt8.self)
      let elapsed = DispatchTime.now().uptimeNanoseconds - started
      locked {
        lastWidth = w
        lastHeight = h
        lastRotation = rotation
        frames += 1
        packNs = elapsed
      }
      return veydanCallCameraPush(bytes, size, Int32(w), Int32(h), Int32(rotation), formatNv12, timestampUs)
    }
    if !taken { locked { dropped += 1 } }
  }
}
