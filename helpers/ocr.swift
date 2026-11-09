// MacroFlow OCR helper.
// Reads text from an image using Apple's Vision framework and prints it, one line per row.
//
//   macroflow-ocr <image.png> [fast]
//
// It's compiled automatically by build.rs (needs the Xcode Command Line Tools).

import CoreGraphics
import Foundation
import ImageIO
import Vision

let args = CommandLine.arguments
guard args.count >= 2 else {
    FileHandle.standardError.write("usage: macroflow-ocr <image> [fast]\n".data(using: .utf8)!)
    exit(2)
}

let url = URL(fileURLWithPath: args[1])
let fast = args.count >= 3 && args[2] == "fast"

guard let source = CGImageSourceCreateWithURL(url as CFURL, nil),
      let image = CGImageSourceCreateImageAtIndex(source, 0, nil)
else {
    FileHandle.standardError.write("Couldn't read the image.\n".data(using: .utf8)!)
    exit(1)
}

let request = VNRecognizeTextRequest()
request.recognitionLevel = fast ? .fast : .accurate
// Language correction "fixes" numbers and codes into words, so turn it off.
request.usesLanguageCorrection = false

let handler = VNImageRequestHandler(cgImage: image, options: [:])
do {
    try handler.perform([request])
} catch {
    FileHandle.standardError.write("Text recognition failed: \(error)\n".data(using: .utf8)!)
    exit(1)
}

let observations = (request.results as? [VNRecognizedTextObservation]) ?? []

// Top-to-bottom, then left-to-right (Vision's origin is bottom-left).
let sorted = observations.sorted { a, b in
    if abs(a.boundingBox.midY - b.boundingBox.midY) > 0.03 {
        return a.boundingBox.midY > b.boundingBox.midY
    }
    return a.boundingBox.minX < b.boundingBox.minX
}

let lines = sorted.compactMap { $0.topCandidates(1).first?.string }
print(lines.joined(separator: "\n"))
