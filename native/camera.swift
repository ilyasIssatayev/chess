import Foundation
import AVFoundation
import CoreMedia
import CoreVideo

// Versioned binary protocol: newline-delimited JSON header followed by tight BGRA.
// Camera timestamps are sample presentation timestamps, never inference times.
final class Capture: NSObject, AVCaptureVideoDataOutputSampleBufferDelegate {
    let session = AVCaptureSession()
    let output = FileHandle.standardOutput
    var anchor: CMTime?
    var sequence: UInt64 = 0
    var dropped: UInt64 = 0
    func captureOutput(_ output: AVCaptureOutput, didDrop sampleBuffer: CMSampleBuffer, from connection: AVCaptureConnection) { dropped += 1 }
    func captureOutput(_ output: AVCaptureOutput, didOutput sampleBuffer: CMSampleBuffer, from connection: AVCaptureConnection) {
        guard let pixel = CMSampleBufferGetImageBuffer(sampleBuffer) else { return }
        let pts = CMSampleBufferGetPresentationTimeStamp(sampleBuffer)
        guard pts.isValid && !pts.isIndefinite else { return }
        if anchor == nil { anchor = pts }
        let elapsed = CMTimeGetSeconds(CMTimeSubtract(pts, anchor!))
        guard elapsed.isFinite && elapsed >= 0 else { return }
        CVPixelBufferLockBaseAddress(pixel, .readOnly)
        defer { CVPixelBufferUnlockBaseAddress(pixel, .readOnly) }
        guard let base = CVPixelBufferGetBaseAddress(pixel) else { return }
        let width = CVPixelBufferGetWidth(pixel), height = CVPixelBufferGetHeight(pixel)
        let stride = CVPixelBufferGetBytesPerRow(pixel)
        guard width <= 1920 && height <= 1080 && stride >= width * 4 else { return }
        var bytes = Data(capacity: width * height * 4)
        for row in 0..<height { bytes.append(base.advanced(by: row * stride).assumingMemoryBound(to: UInt8.self), count: width * 4) }
        let header: [String: Any] = ["version": 1, "sequence": sequence, "capture_time_us": Int64((elapsed * 1_000_000).rounded()), "width": width, "height": height, "bytes": bytes.count, "dropped": dropped, "timestamp_source": "avfoundation_presentation_time"]
        sequence += 1
        do {
            var line = try JSONSerialization.data(withJSONObject: header, options: [.sortedKeys]); line.append(10)
            try self.output.write(contentsOf: line)
            try self.output.write(contentsOf: bytes)
        } catch { exit(0) }
    }
    func start() throws {
        session.beginConfiguration()
        session.sessionPreset = .hd1280x720
        let cameras = AVCaptureDevice.DiscoverySession(deviceTypes: [.builtInWideAngleCamera], mediaType: .video, position: .unspecified).devices
        guard let camera = cameras.first else { throw NSError(domain: "No built-in camera is available", code: 1) }
        let input = try AVCaptureDeviceInput(device: camera)
        guard session.canAddInput(input) else { throw NSError(domain: "Camera input unavailable", code: 2) }
        session.addInput(input)
        let video = AVCaptureVideoDataOutput()
        video.videoSettings = [kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA]
        video.alwaysDiscardsLateVideoFrames = true
        video.setSampleBufferDelegate(self, queue: DispatchQueue(label: "chess.camera.frames"))
        guard session.canAddOutput(video) else { throw NSError(domain: "Camera output unavailable", code: 3) }
        session.addOutput(video)
        session.commitConfiguration()
        session.startRunning()
    }
}
func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data((message + "\n").utf8)); exit(1)
}
let permission = DispatchSemaphore(value: 0)
var granted = false
AVCaptureDevice.requestAccess(for: .video) { value in granted = value; permission.signal() }
guard permission.wait(timeout: .now() + 55) == .success && granted else {
    fail("Camera permission denied. Enable Chess Camera Recorder in System Settings > Privacy & Security > Camera, then restart capture.")
}
let capture = Capture()
do { try capture.start() } catch { fail(error.localizedDescription) }
RunLoop.main.run()
