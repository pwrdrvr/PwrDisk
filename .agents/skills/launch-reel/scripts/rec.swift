// rec WID OUT.mov STOPFILE MAXSECONDS: record one window with
// ScreenCaptureKit to ProRes 422 LT, up to 60 fps, cursor shown.
// Prints "rec: start <epoch>" at the first frame so markers line up.
// The output has a variable frame rate: SCK sends a frame only when the
// window changes.
import Foundation
import AppKit
import ScreenCaptureKit
import AVFoundation
import CoreMedia

// A command-line tool must bring up its window-server connection first, or
// ScreenCaptureKit asserts in CGS_REQUIRE_INIT.
_ = NSApplication.shared
_ = CGMainDisplayID()

let a = CommandLine.arguments
let wid = CGWindowID(a[1])!
let out = URL(fileURLWithPath: a[2])
let stopPath = a[3]
let maxS = Double(a[4])!

final class Rec: NSObject, SCStreamOutput {
    let writer: AVAssetWriter
    let input: AVAssetWriterInput
    var started = false
    var frames = 0
    init(url: URL, w: Int, h: Int) throws {
        try? FileManager.default.removeItem(at: url)
        writer = try AVAssetWriter(outputURL: url, fileType: .mov)
        input = AVAssetWriterInput(mediaType: .video, outputSettings: [
            AVVideoCodecKey: AVVideoCodecType.proRes422LT,
            AVVideoWidthKey: w, AVVideoHeightKey: h,
        ])
        input.expectsMediaDataInRealTime = true
        writer.add(input)
    }
    func stream(_ s: SCStream, didOutputSampleBuffer sb: CMSampleBuffer, of type: SCStreamOutputType) {
        guard type == .screen, sb.isValid,
              let att = CMSampleBufferGetSampleAttachmentsArray(sb, createIfNecessary: false) as? [[SCStreamFrameInfo: Any]],
              let raw = att.first?[.status] as? Int, SCFrameStatus(rawValue: raw) == .complete
        else { return }
        if !started {
            writer.startWriting()
            writer.startSession(atSourceTime: sb.presentationTimeStamp)
            started = true
            print(String(format: "rec: start %.3f", Date().timeIntervalSince1970)); fflush(stdout)
        }
        if input.isReadyForMoreMediaData { input.append(sb); frames += 1 }
    }
}

final class Flag: @unchecked Sendable { var done = false }
let flag = Flag()
Task {
    do {
        let content = try await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: true)
        guard let w = content.windows.first(where: { $0.windowID == wid }) else { print("rec: no window"); flag.done = true; return }
        let scale = Int(NSScreen.main?.backingScaleFactor ?? 1)
        let pw = Int(w.frame.width) * scale, ph = Int(w.frame.height) * scale
        let cfg = SCStreamConfiguration()
        cfg.width = pw; cfg.height = ph
        cfg.showsCursor = true
        cfg.minimumFrameInterval = CMTime(value: 1, timescale: 60)
        cfg.pixelFormat = kCVPixelFormatType_32BGRA
        cfg.queueDepth = 8
        let rec = try Rec(url: out, w: pw, h: ph)
        let stream = SCStream(filter: SCContentFilter(desktopIndependentWindow: w), configuration: cfg, delegate: nil)
        try stream.addStreamOutput(rec, type: .screen, sampleHandlerQueue: DispatchQueue(label: "rec"))
        try await stream.startCapture()
        let start = Date()
        while Date().timeIntervalSince(start) < maxS && !FileManager.default.fileExists(atPath: stopPath) {
            try await Task.sleep(nanoseconds: 100_000_000)
        }
        try await stream.stopCapture()
        // An unstarted writer raises on finish; say why instead of crashing.
        guard rec.started else { print("rec: no complete frame arrived"); flag.done = true; return }
        rec.input.markAsFinished()
        await rec.writer.finishWriting()
        print("rec: \(rec.frames) frames, \(pw)x\(ph), status \(rec.writer.status.rawValue) \(rec.writer.error.map { "\($0)" } ?? "")")
    } catch { print("rec error: \(error)") }
    flag.done = true
}
// Keep the main run loop turning: ScreenCaptureKit and AppKit expect it.
while !flag.done { RunLoop.main.run(until: Date().addingTimeInterval(0.1)) }
