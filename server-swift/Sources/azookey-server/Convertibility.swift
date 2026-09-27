import KanaKanjiConverterModule
import Foundation
import SwiftUtils

/// ユーザー辞書の登録候補（#17）で「登録しなくても変換できる」とみなす候補の順位（上位何件に出るか）
let convertibilityTopCount = 5

/// 変換できるかを調べる語。`reading` が空なら辞書から読みを推定する
struct ConvertibilityQuery: Codable, Equatable, Sendable {
    var word: String
    var reading: String
}

/// 調べた結果。`reading` は調べるのに使った読み（推定できなければ空）
struct ConvertibilityResult: Codable, Equatable, Sendable {
    var word: String
    var reading: String
    var convertible: Bool
}

/// 読みとして変換器に渡せる文字（ひらがなと長音）だけでできているか。
/// 英字の語を推定すると英字のまま返るが、それを入力すると英字がそのまま候補に出て「変換できる」に見えるので除く
func isHiraganaReading(_ reading: String) -> Bool {
    !reading.isEmpty && reading.unicodeScalars.allSatisfy { scalar in
        (0x3041...0x3096).contains(scalar.value) || scalar.value == 0x30FC
    }
}

/// 検査用の変換オプション。Zenzai は重く、並べ替えは文脈で変わるので切り、辞書の順位で判定する
@MainActor func convertibilityOptions() -> ConvertRequestOptions {
    var options = getOptions(context: "")
    options.zenzaiMode = .off
    return options
}

/// `reading` を入力したとき、上位 `convertibilityTopCount` 件に `word` が出るか
@MainActor func appearsInTopCandidates(word: String, reading: String) -> Bool {
    var text = ComposingText()
    text.insertAtCursorPosition(reading, inputStyle: .direct)
    let converted = converter.requestCandidates(text, options: convertibilityOptions())
    return converted.mainResults
        .prefix(convertibilityTopCount)
        .contains { constructCandidateString(candidate: $0, hiragana: reading) == word }
}

typealias ReadingMatches = [Range<Int>: [(reading: String, value: Float)]]

/// 複数の表層形をまとめて逆引きする。辞書の逆引きは全体の走査で 1 回に約 0.07 秒かかるので、語ごとに走査しない。
/// 表層形を NUL でつないで 1 回だけ引き、語ごとの範囲に分け直す（辞書の語は NUL を含まないので、語をまたいだ一致は出ない）
func lookupAll(_ surfaces: [String], index: ReadingIndex) -> [String: ReadingMatches] {
    var joined: [Character] = []
    var starts: [Int] = []
    for surface in surfaces {
        starts.append(joined.count)
        joined += Array(surface)
        joined.append("\u{0}")
    }
    let all = index.lookup(joined)
    var result: [String: ReadingMatches] = [:]
    for (surface, start) in zip(surfaces, starts) {
        let end = start + surface.count
        var matches: ReadingMatches = [:]
        for (range, entries) in all where range.lowerBound >= start && range.upperBound <= end {
            matches[(range.lowerBound - start)..<(range.upperBound - start)] = entries
        }
        result[surface] = matches
    }
    return result
}

/// 語ごとに、登録しなくても変換できるかを調べる。
/// IME の入力中の文字列（`composingText`）・直近の候補・学習には触れない。
/// 最後に変換器の前回の結果を捨てさせ、IME の次の変換にこの検査の結果を持ち越さない
@MainActor func checkConvertibility(_ queries: [ConvertibilityQuery]) -> [ConvertibilityResult] {
    defer { converter.stopComposition() }
    // 読みを推定する語のうち、辞書を引くもの（漢字を含むもの）だけをまとめて引く
    let surfaces = queries
        .filter { $0.reading.isEmpty && $0.word.count <= maxReconversionSurfaceLength && $0.word.contains(where: isKanjiForReading) }
        .map(\.word)
    let matches = surfaces.isEmpty ? [:] : lookupAll(surfaces, index: reconversionReadingIndex())
    return queries.map { query in
        let readings = query.reading.isEmpty
            ? inferReadings(for: query.word, lookup: { _ in matches[query.word] ?? [:] })
            : [query.reading]
        let valid = readings.filter(isHiraganaReading)
        if let reading = valid.first(where: { appearsInTopCandidates(word: query.word, reading: $0) }) {
            return ConvertibilityResult(word: query.word, reading: reading, convertible: true)
        }
        return ConvertibilityResult(word: query.word, reading: valid.first ?? "", convertible: false)
    }
}

/// `queries` は JSON（`[{"word": ..., "reading": ...}]`）。返り値も JSON（`[{"word": ..., "reading": ..., "convertible": ...}]`）
@_silgen_name("CheckConvertibility")
@MainActor public func check_convertibility(queries: UnsafePointer<CChar>) -> UnsafeMutablePointer<CChar> {
    let data = Data(String(cString: queries).utf8)
    let decoded = (try? JSONDecoder().decode([ConvertibilityQuery].self, from: data)) ?? []
    let encoded = (try? JSONEncoder().encode(checkConvertibility(decoded))) ?? Data("[]".utf8)
    return _strdup(String(decoding: encoded, as: UTF8.self))!
}
