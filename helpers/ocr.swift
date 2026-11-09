// MacroFlow OCR helper.
// Reads text from an image using Apple's Vision framework and prints it, one line per row.
//
//   macroflow-ocr <image1.png> [image2.png ...]   (results separated by <<<MACROFLOW-NEXT>>>)
//
// It's compiled automatically by build.rs (needs the Xcode Command Line Tools).

import CoreGraphics
import Foundation
import ImageIO
import Vision

let args = CommandLine.arguments
let paths = args.dropFirst().filter { $0 != "fast" }
guard !paths.isEmpty else {
    FileHandle.standardError.write("usage: macroflow-ocr <image> [image ...]\n".data(using: .utf8)!)
    exit(2)
}

/// Read one image and return its text, top-to-bottom, one line per row.
func read(_ path: String) -> String {
    let url = URL(fileURLWithPath: path)
    guard let source = CGImageSourceCreateWithURL(url as CFURL, nil),
        let image = CGImageSourceCreateImageAtIndex(source, 0, nil)
    else {
        FileHandle.standardError.write("Couldn't read \(path)\n".data(using: .utf8)!)
        return ""
    }

    let request = VNRecognizeTextRequest()
    request.recognitionLevel = .accurate
    // Language correction "fixes" numbers and codes into words, so turn it off.
    request.usesLanguageCorrection = false
    request.minimumTextHeight = 0

    let handler = VNImageRequestHandler(cgImage: image, options: [:])
    do {
        try handler.perform([request])
    } catch {
        FileHandle.standardError.write("Text recognition failed: \(error)\n".data(using: .utf8)!)
        return ""
    }

    let observations = (request.results as? [VNRecognizedTextObservation]) ?? []
    // Top-to-bottom, then left-to-right (Vision's origin is bottom-left).
    let sorted = observations.sorted { a, b in
        if abs(a.boundingBox.midY - b.boundingBox.midY) > 0.03 {
            return a.boundingBox.midY > b.boundingBox.midY
        }
        return a.boundingBox.minX < b.boundingBox.minX
    }
    return sorted.compactMap { $0.topCandidates(1).first?.string }.joined(separator: "\n")
}

var output: [String] = []
for path in paths {
    output.append(read(path))
}
print(output.joined(separator: "\n<<<MACROFLOW-NEXT>>>\n"))
