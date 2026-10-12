import Testing
import Foundation
import KanaKanjiConverterModule
import SwiftUtils
@testable import azookey_server

// 日英の自動判定（#26）を、会話ログから作った評価セットで測る。
// 評価セットは scripts/english-detection/extract_eval_set.py の出力のフォルダ。ユーザーの発言そのものなので、リポジトリに入れない。
// 環境変数 AZOOKEY_ENGLISH_EVAL にそのフォルダを渡したときだけ走らせる:
//   AZOOKEY_ENGLISH_EVAL=<フォルダ> swift test --filter EnglishEvaluation
// 結果は同じフォルダの report.txt（外れた例を含むので、これもリポジトリに入れない）
// ほかの環境変数: AZOOKEY_ENGLISH_WORDS（同梱の一覧の場所）・AZOOKEY_ENGLISH_MAX_SIZE（使う大きさの上限）・
// AZOOKEY_ENGLISH_MAX_HITS（自分の語彙を作るときの、日本語の文で英単語に取ってしまう回数の上限）
// 変換エンジンの ComposingText はサーバーでは MainActor からしか使わないので、テストも MainActor で走らせる
@MainActor @Suite(.enabled(if: ProcessInfo.processInfo.environment["AZOOKEY_ENGLISH_EVAL"] != nil))
struct EnglishEvaluation {
    let environment = ProcessInfo.processInfo.environment
    let root = URL(filePath: #filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()

    struct Unit: Decodable {
        var ts: String
        var text: String
    }

    /// 文を英字の語と日本語の部分に分ける
    func pieces(_ text: String) -> [(text: String, isEnglish: Bool)] {
        var result: [(text: String, isEnglish: Bool)] = []
        for character in text {
            let isEnglish = character.isASCII && character.isLetter
            if let last = result.last, last.isEnglish == isEnglish {
                result[result.count - 1].text.append(character)
            } else {
                result.append((String(character), isEnglish))
            }
        }
        return result
    }

    func readLines(_ url: URL) throws -> [String] {
        // Windows の Python が書いた改行は \r\n（Swift では 1 文字）
        try String(contentsOf: url, encoding: .utf8).split(whereSeparator: \.isNewline).map(String.init)
    }

    /// 日本語の部分の読み（ひらがな）。辞書の逆引き（再変換と同じ inferReadings）で付ける。読みが付かなければ空。
    /// 時間がかかるので、フォルダの readings.json に残して使い回す
    func readings(for surfaces: [String], cache: URL) throws -> [String: String] {
        var known: [String: String] = [:]
        if let data = try? Data(contentsOf: cache) {
            known = try JSONDecoder().decode([String: String].self, from: data)
        }
        let missing = Array(Set(surfaces.filter { known[$0] == nil }))
        if !missing.isEmpty {
            let index = ReadingIndex(files: ReadingIndex.loadFiles(
                in: root.appendingPathComponent("azooKey_dictionary_storage/Dictionary/louds", isDirectory: true)))
            let withKanji = missing.filter { $0.contains(where: isKanjiForReading) }
            var matches: [String: [Range<Int>: [(reading: String, value: Float)]]] = [:]
            for chunkStart in stride(from: 0, to: withKanji.count, by: 4000) {
                let chunk = Array(withKanji[chunkStart..<min(withKanji.count, chunkStart + 4000)])
                matches.merge(index.lookupMany(chunk)) { $1 }
            }
            for surface in missing {
                known[surface] = inferReadings(for: surface, lookup: { _ in matches[surface] ?? [:] }).first ?? ""
            }
            try JSONEncoder().encode(known).write(to: cache)
        }
        return known
    }

    /// 読みをローマ字にし、変換エンジンで元の読みに戻ることを確かめる。戻らなければ nil
    func roman(_ hiragana: String) -> String? {
        guard !hiragana.isEmpty, let roman = romanize(hiragana: hiragana),
              readRomaji(roman).kana == hiragana else { return nil }
        return roman
    }

    @Test func measure() throws {
        let directory = URL(filePath: environment["AZOOKEY_ENGLISH_EVAL"]!, directoryHint: .isDirectory)
        let wordsURL = environment["AZOOKEY_ENGLISH_WORDS"].map { URL(filePath: $0) }
            ?? root.appendingPathComponent("EnglishWords/english_words.txt")
        let maxSize = environment["AZOOKEY_ENGLISH_MAX_SIZE"].flatMap { Int($0) } ?? 80
        // 重みを変えて試す: AZOOKEY_ENGLISH_WEIGHTS="ownWord=0.9,unknownWord=2.5"
        var weights = EnglishSpanWeights()
        for pair in (environment["AZOOKEY_ENGLISH_WEIGHTS"] ?? "").split(separator: ",") {
            let parts = pair.split(separator: "=")
            guard parts.count == 2, let value = Float(parts[1]) else { continue }
            switch parts[0] {
            case "leftoverLetter": weights.leftoverLetter = value
            case "ownWord": weights.ownWord = value
            case "shortOwnWord": weights.shortOwnWord = value
            case "ownFrequency": weights.ownFrequency = value
            case "bundledWord": weights.bundledWord = value
            case "unknownWord": weights.unknownWord = value
            case "unknownKanaLetter": weights.unknownKanaLetter = value
            case "uppercaseRun": weights.uppercaseRun = value
            case "particleAfter": weights.particleAfter = value
            case "particleBefore": weights.particleBefore = value
            default: Issue.record("知らない重み: \(parts[0])")
            }
        }
        var rules = OwnVocabularyRules()
        if let hits = environment["AZOOKEY_ENGLISH_MAX_HITS"].flatMap({ Int($0) }) { rules.maxJapaneseHits = hits }

        let bundled = loadEnglishWordList(wordsURL)
        #expect(!bundled.isEmpty)
        let decoder = JSONDecoder()
        let units = try readLines(directory.appendingPathComponent("units.jsonl")).map { try decoder.decode(Unit.self, from: Data($0.utf8)) }
        let japaneseUnits = try readLines(directory.appendingPathComponent("ja_units.jsonl")).map { try decoder.decode(Unit.self, from: Data($0.utf8)) }
        let train = try readLines(directory.appendingPathComponent("ja_train.txt"))
        let counts = try decoder.decode([String: Int].self, from: Data(contentsOf: directory.appendingPathComponent("vocab.json")))

        // 読みを付ける
        let unitPieces = units.map { pieces($0.text) }
        let surfaces = unitPieces.flatMap { $0.filter { !$0.isEnglish }.map(\.text) } + japaneseUnits.map(\.text) + train
        var clock = ContinuousClock.now
        let readingOf = try readings(for: surfaces, cache: directory.appendingPathComponent("readings.json"))
        let readingTime = ContinuousClock.now - clock

        // 自分の語彙（日本語として打った文で英単語に取ってしまう語を外す）
        clock = ContinuousClock.now
        let trainInputs = train.compactMap { readingOf[$0].flatMap(roman) }
        let (ownWords, removed) = buildOwnVocabulary(counts: counts, japaneseInputs: trainInputs, bundledWords: bundled, maxBundledSize: maxSize, weights: weights, rules: rules)
        let vocabularyTime = ContinuousClock.now - clock
        var lexicon = EnglishLexicon(ownWords: ownWords, bundledWords: bundled, learned: [:], maxBundledSize: maxSize, weights: weights)
        lexicon.ownCounts = counts.filter { ownWords.contains($0.key) }
        lexicon.unknownWords = environment["AZOOKEY_ENGLISH_UNKNOWN"] != "0"

        var report: [String] = []
        report.append("同梱の一覧: \(bundled.count) 語（大きさ \(maxSize) まで使う）・一覧に無い語: \(lexicon.unknownWords)・重み: \(weights)")
        report.append("自分の語彙: 候補 \(counts.filter { $0.value >= rules.minCount }.count) 語 → \(ownWords.count) 語（日本語の文 \(trainInputs.count)/\(train.count) で確かめ、\(removed.count) 語を外した）")
        report.append("外した語（多い順の 80 語）: " + removed.sorted { $0.value > $1.value }.prefix(80).map { "\($0.key)(\($0.value))" }.joined(separator: " "))
        report.append("読みの付与: \(readingTime)・自分の語彙を作る: \(vocabularyTime)")

        // 混じる文
        for lowercased in [true, false] {
            var englishTotal = 0, englishKept = 0, japaneseTotal = 0, japaneseBroken = 0, sentences = 0, exact = 0
            var noReading = 0
            var missed: [String: [String]] = [:]
            var broken: [String] = []
            var detectTime = Duration.zero
            for pieceList in unitPieces {
                var input = ""
                var gold: [(range: Range<Int>, isEnglish: Bool, text: String)] = []
                var ok = true
                for piece in pieceList {
                    let text: String
                    if piece.isEnglish {
                        text = lowercased ? piece.text.lowercased() : piece.text
                    } else {
                        guard let roman = readingOf[piece.text].flatMap(roman) else { ok = false; break }
                        text = roman
                    }
                    gold.append((input.count..<(input.count + text.count), piece.isEnglish, piece.text))
                    input += text
                }
                guard ok else { noReading += 1; continue }
                sentences += 1
                let start = ContinuousClock.now
                let spans = englishSpans(in: input, lexicon: lexicon, complete: true)
                detectTime += ContinuousClock.now - start
                let characters = Array(input)
                let shown = { () -> String in
                    var text = ""
                    var position = 0
                    for span in spans {
                        text += readRomaji(String(characters[position..<span.lowerBound])).kana + "[" + String(characters[span]) + "]"
                        position = span.upperBound
                    }
                    return text + readRomaji(String(characters[position...])).kana
                }
                var allRight = true
                for piece in gold {
                    if piece.isEnglish {
                        englishTotal += 1
                        if spans.contains(piece.range) {
                            englishKept += 1
                        } else {
                            allRight = false
                            let word = piece.text.lowercased()
                            let reading = readRomaji(word)
                            let kind: String
                            if spans.contains(where: { $0.overlaps(piece.range) }) {
                                kind = "区切りがずれた"
                            } else if ownWords.contains(word) {
                                kind = "自分の語彙にあるのに取れない"
                            } else if removed[word] != nil {
                                kind = "自分の語彙から外した語"
                            } else if bundled[word].map({ $0 <= maxSize }) ?? false {
                                kind = reading.leftover == 0 ? "同梱の一覧だけ・ローマ字で読める（決まりで日本語）" : "同梱の一覧だけ・読めないのに取れない"
                            } else if (counts[word] ?? 0) > 0 {
                                kind = "自分の語彙に \(rules.minCount) 回未満"
                            } else {
                                kind = "どの一覧にも無い"
                            }
                            missed[kind, default: []].append("\(piece.text) ← \(input) → \(shown())")
                        }
                    } else {
                        japaneseTotal += 1
                        if spans.contains(where: { $0.overlaps(piece.range) }) {
                            japaneseBroken += 1
                            allRight = false
                            broken.append("\(piece.text) ← \(input) → \(shown())")
                        }
                    }
                }
                if allRight { exact += 1 }
            }
            let mode = lowercased ? "英字をすべて小文字で打った場合" : "英字を会話ログのとおりの大文字・小文字で打った場合"
            report.append("")
            report.append("== 混じる文・\(mode)")
            report.append("文: \(units.count)（読みが付かず外した \(noReading)）→ 評価 \(sentences) 文・すべて正しい \(exact)")
            report.append(String(format: "英単語の区間を英字にできた: %d / %d = %.1f%%", englishKept, englishTotal, 100 * Double(englishKept) / Double(max(1, englishTotal))))
            report.append(String(format: "日本語の区間を英字にしてしまった: %d / %d = %.2f%%", japaneseBroken, japaneseTotal, 100 * Double(japaneseBroken) / Double(max(1, japaneseTotal))))
            report.append("判定の時間（1 文あたりの平均）: \(detectTime / max(1, sentences))")
            for (kind, examples) in missed.sorted(by: { $0.value.count > $1.value.count }) {
                report.append("-- 取れなかった英単語: \(kind) \(examples.count)")
                report.append(contentsOf: examples.prefix(lowercased ? 60 : 20).map { "  " + $0 })
            }
            report.append("-- 英字にしてしまった日本語の区間 \(broken.count)")
            report.append(contentsOf: broken.prefix(60).map { "  " + $0 })
        }

        // 日本語だけの文
        var japaneseSentences = 0, japaneseNoReading = 0, japaneseBrokenSentences = 0
        var brokenExamples: [String] = []
        for unit in japaneseUnits {
            guard let input = readingOf[unit.text].flatMap(roman) else { japaneseNoReading += 1; continue }
            japaneseSentences += 1
            let spans = englishSpans(in: input, lexicon: lexicon, complete: true)
            if !spans.isEmpty {
                japaneseBrokenSentences += 1
                let characters = Array(input)
                brokenExamples.append("\(unit.text) ← \(input) → " + spans.map { String(characters[$0]) }.joined(separator: ","))
            }
        }
        report.append("")
        report.append("== 日本語だけの文")
        report.append("文: \(japaneseUnits.count)（読みが付かず外した \(japaneseNoReading)）→ 評価 \(japaneseSentences) 文")
        report.append(String(format: "どこかを英字にしてしまった文: %d / %d = %.2f%%", japaneseBrokenSentences, japaneseSentences, 100 * Double(japaneseBrokenSentences) / Double(max(1, japaneseSentences))))
        report.append(contentsOf: brokenExamples.prefix(80).map { "  " + $0 })

        // 指定した入力の候補と点（調べるとき）: AZOOKEY_ENGLISH_TRACE="filmrollwotsukau,..."
        for input in (environment["AZOOKEY_ENGLISH_TRACE"] ?? "").split(separator: ",").map(String.init) {
            let characters = Array(input)
            report.append("")
            report.append("== 候補: \(input) → " + englishSpans(in: input, lexicon: lexicon, complete: true).map { String(characters[$0]) }.joined(separator: ","))
            for candidate in englishCandidates(characters, lexicon: lexicon).sorted(by: { ($0.range.lowerBound, $0.range.upperBound) < ($1.range.lowerBound, $1.range.upperBound) }) {
                let word = String(characters[candidate.range]).lowercased()
                let source = ownWords.contains(word) ? "自分" : bundled[word].map { "一覧\($0)" } ?? "無し"
                report.append("  \(word) \(candidate.range) \(candidate.cost) \(source)")
            }
        }

        let text = report.joined(separator: "\n") + "\n"
        try text.write(to: directory.appendingPathComponent(environment["AZOOKEY_ENGLISH_REPORT"] ?? "report.txt"), atomically: true, encoding: .utf8)
        print(report.prefix(12).joined(separator: "\n"))
    }
}
