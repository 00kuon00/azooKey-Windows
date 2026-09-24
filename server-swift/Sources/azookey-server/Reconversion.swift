import KanaKanjiConverterModule
import Foundation
import SwiftUtils

/// 再変換で読みを推定するときに扱う表層形の長さの上限（文字数）
let maxReconversionSurfaceLength = 64
/// 完全一致の読みが複数あるとき、原文が候補に出るかを試す数
let maxReconversionReadingTrials = 4

/// 漢字（々〆〇ヶを含む）か
func isKanjiForReading(_ character: Character) -> Bool {
    character.unicodeScalars.contains { scalar in
        switch scalar.value {
        case 0x3005...0x3007, 0x30F6, 0x3400...0x4DBF, 0x4E00...0x9FFF, 0xF900...0xFAFF, 0x20000...0x2FA1F:
            return true
        default:
            return false
        }
    }
}

/// 辞書（`*.loudstxt3`）を表層形から引く。
/// 辞書は読みで引く形なので、表層形から引くには全体を走査する。
/// ファイルは初めて引いたときに読み込んでメモリに置く（システム辞書で約 20MB）
struct ReadingIndex {
    private var files: [Data] = []

    init(files: [Data]) {
        self.files = files
    }

    /// `directoryURL` 直下の `*.loudstxt3` を読み込む
    static func loadFiles(in directoryURL: URL) -> [Data] {
        let urls = (try? FileManager.default.contentsOfDirectory(at: directoryURL, includingPropertiesForKeys: nil)) ?? []
        return urls
            .filter { $0.pathExtension == "loudstxt3" }
            .compactMap { try? Data(contentsOf: $0) }
    }

    /// `surface` の部分文字列のうち、漢字を含み辞書に語として載っているものを集める。
    /// キーは部分文字列の範囲（文字単位の `start..<end`）、値は読み（カタカナ）と重み
    func lookup(_ surface: [Character]) -> [Range<Int>: [(reading: String, value: Float)]] {
        // 表層形を UTF-8 にし、各文字の始まりのバイト位置を持つ
        var bytes: [UInt8] = []
        var charStarts: [Int] = []
        for character in surface {
            charStarts.append(bytes.count)
            bytes.append(contentsOf: String(character).utf8)
        }
        let charIndexAtByte = Dictionary(uniqueKeysWithValues: (charStarts + [bytes.count]).enumerated().map { ($1, $0) })
        let kanjiPrefixCount = surface.reduce(into: [0]) { $0.append($0.last! + (isKanjiForReading($1) ? 1 : 0)) }

        var result: [Range<Int>: [(reading: String, value: Float)]] = [:]
        for file in files {
            file.withUnsafeBytes { (buffer: UnsafeRawBufferPointer) in
                Self.scan(buffer) { reading, value, word in
                    // 語が表層形のどこかに文字の区切りで一致するか
                    guard !word.isEmpty, word.count <= bytes.count else { return }
                    for start in charStarts where bytes[start] == word[word.startIndex] {
                        let end = start + word.count
                        guard end <= bytes.count, let endChar = charIndexAtByte[end] else { continue }
                        guard bytes[start..<end].elementsEqual(word) else { continue }
                        let startChar = charIndexAtByte[start]!
                        // かなだけの語は読みをそのまま使えるので要らない
                        guard kanjiPrefixCount[endChar] > kanjiPrefixCount[startChar] else { continue }
                        result[startChar..<endChar, default: []].append((reading(), value))
                    }
                }
            }
        }
        return result
    }

    /// loudstxt3 の全行を走査する。形式は変換エンジンの `LOUDS.parseBinary` と同じ:
    /// 先頭に UInt16 の区画数と UInt32 の区画の開始位置、各区画は UInt16 の行数・行ごとに 10 バイト
    /// （lcid・rcid・mid が UInt16、重みが Float32）・タブ区切りの文字列（先頭が読み、続いて行ごとの語。空なら読みと同じ）
    private static func scan(
        _ buffer: UnsafeRawBufferPointer,
        _ body: (_ reading: () -> String, _ value: Float, _ word: UnsafeRawBufferPointer.SubSequence) -> Void
    ) {
        guard buffer.count >= 2 else { return }
        let slotCount = Int(buffer.loadUnaligned(fromByteOffset: 0, as: UInt16.self))
        let headerEnd = 2 + slotCount * 4
        guard buffer.count >= headerEnd else { return }
        for slot in 0..<slotCount {
            let start = Int(buffer.loadUnaligned(fromByteOffset: 2 + slot * 4, as: UInt32.self))
            let end = slot == slotCount - 1
                ? buffer.count
                : Int(buffer.loadUnaligned(fromByteOffset: 2 + (slot + 1) * 4, as: UInt32.self))
            guard start >= headerEnd, start + 2 <= end, end <= buffer.count else { continue }
            let rowCount = Int(buffer.loadUnaligned(fromByteOffset: start, as: UInt16.self))
            let textStart = start + 2 + rowCount * 10
            guard rowCount > 0, textStart <= end else { continue }

            let fields = buffer[textStart..<end].split(separator: UInt8(ascii: "\t"), omittingEmptySubsequences: false)
            guard fields.count >= 2 else { continue }
            let readingBytes = fields[0]
            var decodedReading: String?
            let reading = {
                if let decodedReading { return decodedReading }
                let string = String(decoding: readingBytes, as: UTF8.self)
                decodedReading = string
                return string
            }
            for row in 0..<min(rowCount, fields.count - 1) {
                let value = buffer.loadUnaligned(fromByteOffset: start + 2 + row * 10 + 6, as: Float32.self)
                body(reading, value, fields[row + 1])
            }
        }
    }
}

/// 表層形から読み（ひらがな）の候補を推定する。良い順に並ぶ。推定できなければ空。
/// - 漢字を含まなければ、カタカナをひらがなにしたものだけ
/// - 表層形全体が辞書の語なら、その読みを重みの順に（最大 `maxReconversionReadingTrials` 件）
/// - そうでなければ、辞書の語とそれ以外の 1 文字ずつに分ける。分け方は、読みの分からない漢字が少なく、
///   区切りが少なく、重みの合計が大きいものを選ぶ
func inferReadings(for surface: String, index: ReadingIndex) -> [String] {
    let characters = Array(surface)
    guard !characters.isEmpty, characters.count <= maxReconversionSurfaceLength else {
        return []
    }
    guard characters.contains(where: isKanjiForReading) else {
        return [surface.toHiragana()]
    }

    let matches = index.lookup(characters)
    if let exact = matches[0..<characters.count] {
        var readings: [String] = []
        for entry in exact.sorted(by: { $0.value > $1.value }) {
            let reading = entry.reading.toHiragana()
            if !readings.contains(reading) {
                readings.append(reading)
            }
        }
        return Array(readings.prefix(maxReconversionReadingTrials))
    }

    // best[i] = 先頭 i 文字の最良の分け方（読みの分からない漢字の数, 区切りの数, -重みの合計, 読み）
    typealias Path = (unknownKanji: Int, segments: Int, cost: Float, reading: String)
    func isBetter(_ lhs: Path, than rhs: Path) -> Bool {
        (lhs.unknownKanji, lhs.segments, lhs.cost) < (rhs.unknownKanji, rhs.segments, rhs.cost)
    }
    var best: [Path?] = Array(repeating: nil, count: characters.count + 1)
    best[0] = (0, 0, 0, "")
    for start in 0..<characters.count {
        guard let path = best[start] else { continue }
        func relax(_ end: Int, _ candidate: Path) {
            if let current = best[end], !isBetter(candidate, than: current) { return }
            best[end] = candidate
        }
        let character = characters[start]
        relax(start + 1, (
            path.unknownKanji + (isKanjiForReading(character) ? 1 : 0),
            path.segments + 1,
            path.cost,
            path.reading + String(character).toHiragana()
        ))
        for end in (start + 1)...characters.count {
            guard let entries = matches[start..<end],
                  let entry = entries.max(by: { $0.value < $1.value }) else { continue }
            relax(end, (path.unknownKanji, path.segments + 1, path.cost - entry.value, path.reading + entry.reading.toHiragana()))
        }
    }
    guard let path = best[characters.count], path.unknownKanji < characters.filter(isKanjiForReading).count else {
        // 漢字の読みが一つも分からないときは再変換しない
        return []
    }
    return [path.reading]
}

// システム辞書の逆引き。初めて再変換したときに読み込む
@MainActor var systemReadingFiles: [Data]?

/// 再変換に使う辞書（システム辞書とユーザー辞書）
@MainActor func reconversionReadingIndex() -> ReadingIndex {
    if systemReadingFiles == nil {
        systemReadingFiles = ReadingIndex.loadFiles(in: systemDictionaryURL.appendingPathComponent("louds", isDirectory: true))
    }
    // ユーザー辞書は小さく、作り直されることがあるので毎回読む
    let userFiles = ReadingIndex.loadFiles(in: userDictionaryURL)
    return ReadingIndex(files: systemReadingFiles! + userFiles)
}

/// 変換器が返す候補の文字列（`GetComposedText` と同じ作り方）
@MainActor func candidateTexts(for text: ComposingText) -> [String] {
    let converted = converter.requestCandidates(text, options: getOptions(context: (config["context"] as? String) ?? ""))
    return converted.mainResults.map { constructCandidateString(candidate: $0, hiragana: text.convertTarget) }
}

/// 確定済みの文字列 `surface` を読みに戻し、入力中の文字列をその読みにする。
/// 読みの候補が複数あるときは、候補に `surface` が出る最初の読みを使う（無ければ最も良い読み）。
/// 返り値は入力中の文字列（ひらがな）。読みが推定できなければ空で、入力中の文字列も空になる
@MainActor func startReconversion(surface: String) -> String {
    clear_text()
    let readings = inferReadings(for: surface, index: reconversionReadingIndex())
    guard let first = readings.first else {
        return ""
    }
    var chosen = first
    if readings.count > 1 {
        for reading in readings {
            var text = ComposingText()
            text.insertAtCursorPosition(reading, inputStyle: .direct)
            if candidateTexts(for: text).contains(surface) {
                chosen = reading
                break
            }
        }
    }
    composingText.insertAtCursorPosition(chosen, inputStyle: .direct)
    return composingText.convertTarget
}

@_silgen_name("StartReconversion")
@MainActor public func start_reconversion(surface: UnsafePointer<CChar>) -> UnsafeMutablePointer<CChar> {
    return _strdup(startReconversion(surface: String(cString: surface)))!
}
