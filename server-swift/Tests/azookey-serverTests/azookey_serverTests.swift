import Testing
import Foundation
import KanaKanjiConverterModule
@testable import azookey_server

@Test func example() async throws {
    // Write your test here and use APIs like `#expect(...)` to check expected conditions.
}

// 「nihongo」で「日本」を選んだとき、旧エンジンと同じく input の 5 文字（nihon）を消費し「ご」が残る
@MainActor @Test func inputCountStopsBeforeConsonantAfterSingleN() {
    composingText = ComposingText()
    for character in "nihongo" {
        composingText.insertAtCursorPosition(String(character), inputStyle: .roman2kana)
    }
    let candidate = Candidate(text: "日本", value: 0, composingCount: .surfaceCount(3), lastMid: 0, data: [])

    let (count, remaining) = inputCount(of: candidate)
    #expect(count == 5)
    #expect(remaining == "ご")

    var shrunk = composingText
    shrunk.prefixComplete(composingCount: .inputCount(count))
    #expect(shrunk.convertTarget == "ご")
    composingText = ComposingText()
}

// 学習とユーザー辞書のテストは変換器・入力中の文字列・設定のグローバルを使うので、スイートをまたいで 1 本ずつ流す。
// .serialized は入れ子のスイートにも効くが、別々のトップレベルのスイートどうしは並行に走る
@MainActor @Suite(.serialized) struct GlobalStateTests {}

extension GlobalStateTests {
@MainActor @Suite struct LearningTests {
    let memoryURL: URL

    init() {
        // server-swift/Tests/azookey-serverTests/このファイル → server-swift
        let root = URL(filePath: #filePath)
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
        execURL = root.appendingPathComponent("azooKey_emoji_dictionary_storage")
        // 本物の settings.json（load_config）は読まず、Zenzai も使わない
        config["enable"] = false
        config["profile"] = ""
        memoryURL = FileManager.default.temporaryDirectory
            .appendingPathComponent("azookey-learning-test-\(UUID().uuidString)", isDirectory: true)
        memoryDirectoryURL = memoryURL
        // 本物のユーザー辞書（%APPDATA%）を読まない
        userDictionaryURL = memoryURL.appendingPathComponent("user_dictionary", isDirectory: true)
        converter = KanaKanjiConverter(
            dictionaryURL: root.appendingPathComponent("azooKey_dictionary_storage").appendingPathComponent("Dictionary"),
            preloadDictionary: false
        )
        composingText = ComposingText()
    }

    /// ローマ字を 1 文字ずつ入力し、クライアントと同じく同じ文字列の候補を最初の 1 件だけ残した候補の文字列を返す
    func type(_ roman: String) -> [String] {
        for character in roman {
            let cursor = UnsafeMutablePointer<Int>.allocate(capacity: 1)
            free(append_text(input: String(character), cursorPtr: cursor))
            cursor.deallocate()
        }
        let length = UnsafeMutablePointer<Int>.allocate(capacity: 1)
        defer { length.deallocate() }
        let list = get_composed_text(lengthPtr: length)
        var texts: [String] = []
        for i in 0..<length.pointee {
            let text = String(cString: list[i]!.pointee.text)
            if !texts.contains(text) {
                texts.append(text)
            }
        }
        return texts
    }

    func commit(_ text: String) {
        text.withCString { commit_candidate(text: $0) }
    }

    /// 候補の学習を忘れさせ、取り直した候補の文字列（同じ文字列は最初の 1 件だけ）を返す
    func forget(_ text: String) -> [String] {
        free(text.withCString { forget_candidate(text: $0) })
        let length = UnsafeMutablePointer<Int>.allocate(capacity: 1)
        defer { length.deallocate() }
        let list = get_composed_text(lengthPtr: length)
        var texts: [String] = []
        for i in 0..<length.pointee {
            let text = String(cString: list[i]!.pointee.text)
            if !texts.contains(text) {
                texts.append(text)
            }
        }
        return texts
    }

    func memoryFiles() -> [String] {
        (try? FileManager.default.contentsOfDirectory(atPath: memoryURL.path(percentEncoded: false))) ?? []
    }

    // 2 位の候補を確定すると、次に同じ読みを変換したとき 1 位に来る
    @Test func committedCandidateComesFirstNextTime() {
        learningType = .inputAndOutput
        let before = type("kanji")
        #expect(before.count >= 2)
        let chosen = before[1]

        commit(chosen)
        clear_text()

        let after = type("kanji")
        #expect(after.first == chosen)
        #expect(memoryFiles().contains { $0.hasPrefix("memory.louds") })
        clear_text()
    }

    // 「学習しない」「新しく学習しない」では memory に書き込まない
    @Test(arguments: [LearningType.nothing, .onlyOutput])
    func noWriteWhenLearningIsOff(type learning: LearningType) {
        learningType = learning
        let before = type("kanji")
        #expect(before.count >= 2)

        commit(before[1])
        clear_text()

        #expect(!FileManager.default.fileExists(atPath: memoryURL.path(percentEncoded: false)))
        let after = type("kanji")
        #expect(after.first == before.first)
        clear_text()
    }

    // 学習をリセットすると、元の順位に戻る
    @Test func resetLearningRestoresOriginalOrder() {
        learningType = .inputAndOutput
        let before = type("kanji")
        commit(before[1])
        clear_text()

        reset_learning()

        let after = type("kanji")
        #expect(after.first == before.first)
        clear_text()
    }

    // 学習した候補を忘れさせると、その場で取り直した候補も、次に同じ読みを変換したときも元の順位に戻る
    @Test func forgetCandidateRestoresOriginalOrder() {
        learningType = .inputAndOutput
        let before = type("kanji")
        #expect(before.count >= 2)
        let chosen = before[1]
        commit(chosen)
        clear_text()

        let learned = type("kanji")
        #expect(learned.first == chosen)

        // 候補を選んでいる最中に忘れさせる（入力中の文字列はそのまま）
        let refreshed = forget(chosen)
        #expect(refreshed.first == before.first)
        clear_text()

        let after = type("kanji")
        #expect(after.first == before.first)
        clear_text()
    }

    // 学習していない候補を忘れさせても何も変わらない
    @Test func forgetUnlearnedCandidateKeepsOrder() {
        learningType = .inputAndOutput
        let before = type("kanji")
        #expect(before.count >= 2)

        let refreshed = forget(before[1])
        #expect(refreshed == before)
        clear_text()
    }

    @Test func alphanumericOnlyIsNotLearned() {
        #expect(!shouldLearn("abc"))
        #expect(!shouldLearn("ａｂｃ１２３"))
        #expect(!shouldLearn(""))
        #expect(shouldLearn("漢字"))
        #expect(shouldLearn("Ｗｉｎｄｏｗｓ版"))
    }
}

}

extension GlobalStateTests {
@MainActor @Suite struct UserDictionaryTests {
    let workURL: URL
    let dictionaryURL: URL

    init() {
        let root = URL(filePath: #filePath)
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
        execURL = root.appendingPathComponent("azooKey_emoji_dictionary_storage")
        config["enable"] = false
        config["profile"] = ""
        workURL = FileManager.default.temporaryDirectory
            .appendingPathComponent("azookey-user-dictionary-test-\(UUID().uuidString)", isDirectory: true)
        try? FileManager.default.createDirectory(at: workURL, withIntermediateDirectories: true)
        // 学習は使わない（学習の結果で順位が変わらないように）
        learningType = .nothing
        memoryDirectoryURL = workURL.appendingPathComponent("memory", isDirectory: true)
        dictionaryURL = root.appendingPathComponent("azooKey_dictionary_storage").appendingPathComponent("Dictionary")
        systemDictionaryURL = dictionaryURL
        userDictionarySourceURL = workURL.appendingPathComponent("user_dictionary.json")
        userDictionaryURL = workURL.appendingPathComponent("user_dictionary", isDirectory: true)
        lastUserDictionarySource = nil
        unregisteredUserDictionaryEntries = []
        converter = KanaKanjiConverter(dictionaryURL: dictionaryURL, preloadDictionary: false)
        composingText = ComposingText()
    }

    func type(_ roman: String) -> [String] {
        for character in roman {
            let cursor = UnsafeMutablePointer<Int>.allocate(capacity: 1)
            free(append_text(input: String(character), cursorPtr: cursor))
            cursor.deallocate()
        }
        let length = UnsafeMutablePointer<Int>.allocate(capacity: 1)
        defer { length.deallocate() }
        let list = get_composed_text(lengthPtr: length)
        var texts: [String] = []
        for i in 0..<length.pointee {
            let text = String(cString: list[i]!.pointee.text)
            if !texts.contains(text) {
                texts.append(text)
            }
        }
        clear_text()
        return texts
    }

    /// 設定画面と同じく元データを書き、サーバと同じ経路（reloadUserDictionary）で読ませる
    func save(_ entries: [UserDictionaryEntry]) throws {
        try JSONEncoder().encode(entries).write(to: userDictionarySourceURL)
        reloadUserDictionary()
    }

    func unregisteredFromFFI() throws -> [UserDictionaryEntry] {
        let pointer = get_unregistered_user_dictionary_entries()
        defer { free(pointer) }
        return try JSONDecoder().decode([UserDictionaryEntry].self, from: Data(String(cString: pointer).utf8))
    }

    // 登録した語は、同じ読みのシステム辞書の候補より上に出る。消すと出なくなる
    @Test func registeredWordComesFirstAndDisappearsAfterRemoval() throws {
        let before = type("kanji")
        #expect(!before.contains("幹寺"))

        try save([UserDictionaryEntry(reading: "かんじ", word: "幹寺")])
        #expect(unregisteredUserDictionaryEntries.isEmpty)
        let after = type("kanji")
        #expect(after.first == "幹寺")
        // システム辞書の候補も残っている
        #expect(after.contains(before[0]))

        // 同じ場所に作り直しても、読み込み済みの古い辞書が残らない
        try save([])
        #expect(!type("kanji").contains("幹寺"))
    }

    // charID.chid に無い文字（漢字・@ など）を含む読みは「登録できなかった語」として返り、ほかの語は登録される
    @Test func readingWithUnknownCharacterIsReportedAsUnregistered() throws {
        let rejected = [
            UserDictionaryEntry(reading: "漢じ", word: "かんじテスト"),
            UserDictionaryEntry(reading: "めーる@", word: "メールアドレス"),
            UserDictionaryEntry(reading: "", word: "読みなし"),
        ]
        try save([UserDictionaryEntry(reading: "かんじ", word: "幹寺")] + rejected)

        #expect(unregisteredUserDictionaryEntries == rejected)
        #expect(try unregisteredFromFFI() == rejected)
        #expect(type("kanji").first == "幹寺")
    }

    // 元データが変わらなければ作り直さない（他の設定の変更で UpdateConfig が届いたとき）
    @Test func unchangedSourceIsNotRebuilt() throws {
        try save([UserDictionaryEntry(reading: "かんじ", word: "幹寺")])
        let loudsURL = userDictionaryURL.appendingPathComponent("user.louds")
        try FileManager.default.removeItem(at: loudsURL)

        reloadUserDictionary()
        #expect(!FileManager.default.fileExists(atPath: loudsURL.path(percentEncoded: false)))
    }

    // 1,000 件で辞書を作り直す時間を測る（PR に書く）。登録した語がすべて引ける
    @Test func rebuildThousandEntries() throws {
        let kana = Array("あいうえおかきくけこさしすせそたちつてとなにぬねのはひふへほまみむめもやゆよらりるれろわ")
        var entries: [UserDictionaryEntry] = []
        for i in 0..<1000 {
            let reading = String([kana[i % kana.count], kana[(i / kana.count) % kana.count], kana[(i / 7) % kana.count]]) + "ぴょ"
            entries.append(UserDictionaryEntry(reading: reading, word: "登録語\(i)"))
        }
        try JSONEncoder().encode(entries).write(to: userDictionarySourceURL)

        let clock = ContinuousClock()
        let buildTime = clock.measure {
            reloadUserDictionary()
        }
        print("user dictionary rebuild (1,000 entries): \(buildTime)")
        #expect(unregisteredUserDictionaryEntries.isEmpty)

        // 最初・最後の語が変換で出る（辞書の読み込みと 1 回の変換の時間も測る）
        var first: [String] = []
        let convertTime = clock.measure {
            first = type(romanOf(entries[0].reading))
        }
        print("first conversion after rebuild: \(convertTime)")
        #expect(first.first == entries[0].word)
        #expect(type(romanOf(entries[999].reading)).first == entries[999].word)
    }

    /// テスト用のひらがな → ローマ字（上の読みに使う文字だけ）
    func romanOf(_ reading: String) -> String {
        let table: [Character: String] = [
            "あ": "a", "い": "i", "う": "u", "え": "e", "お": "o",
            "か": "ka", "き": "ki", "く": "ku", "け": "ke", "こ": "ko",
            "さ": "sa", "し": "si", "す": "su", "せ": "se", "そ": "so",
            "た": "ta", "ち": "ti", "つ": "tu", "て": "te", "と": "to",
            "な": "na", "に": "ni", "ぬ": "nu", "ね": "ne", "の": "no",
            "は": "ha", "ひ": "hi", "ふ": "hu", "へ": "he", "ほ": "ho",
            "ま": "ma", "み": "mi", "む": "mu", "め": "me", "も": "mo",
            "や": "ya", "ゆ": "yu", "よ": "yo",
            "ら": "ra", "り": "ri", "る": "ru", "れ": "re", "ろ": "ro",
            "わ": "wa",
        ]
        var result = ""
        var rest = Substring(reading)
        while let character = rest.first {
            if rest.hasPrefix("ぴょ") {
                result += "pyo"
                rest = rest.dropFirst(2)
                continue
            }
            result += table[character]!
            rest = rest.dropFirst()
        }
        return result
    }
}
}

extension GlobalStateTests {
@MainActor @Suite struct SegmentTests {
    let workURL: URL

    init() {
        let root = URL(filePath: #filePath)
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
        execURL = root.appendingPathComponent("azooKey_emoji_dictionary_storage")
        config["enable"] = false
        config["profile"] = ""
        workURL = FileManager.default.temporaryDirectory
            .appendingPathComponent("azookey-segment-test-\(UUID().uuidString)", isDirectory: true)
        // 学習は使わない（学習の結果で候補が変わらないように）
        learningType = .nothing
        memoryDirectoryURL = workURL.appendingPathComponent("memory", isDirectory: true)
        userDictionaryURL = workURL.appendingPathComponent("user_dictionary", isDirectory: true)
        converter = KanaKanjiConverter(
            dictionaryURL: root.appendingPathComponent("azooKey_dictionary_storage").appendingPathComponent("Dictionary"),
            preloadDictionary: false
        )
        clear_text()
    }

    func type(_ roman: String) {
        for character in roman {
            let cursor = UnsafeMutablePointer<Int>.allocate(capacity: 1)
            free(append_text(input: String(character), cursorPtr: cursor))
            cursor.deallocate()
        }
    }

    /// GetComposedText の候補（文字列・確定後に残る読み）
    func candidates() -> [(text: String, subtext: String)] {
        let length = UnsafeMutablePointer<Int>.allocate(capacity: 1)
        defer { length.deallocate() }
        let list = get_composed_text(lengthPtr: length)
        return (0..<length.pointee).map { i in
            (String(cString: list[i]!.pointee.text), String(cString: list[i]!.pointee.subtext))
        }
    }

    func setSegment(_ count: Int32) {
        free(set_segment_surface_count(count: count))
    }

    // 区切りを動かすと、最初の文節の読みが変わり、どの候補もその読みをすべて使う
    @Test func movingBoundaryChangesReadingOfFirstSegment() {
        type("kyouhaiitenki")
        #expect(composingText.convertTarget == "きょうはいいてんき")

        setSegment(3)
        let shrunk = candidates()
        #expect(!shrunk.isEmpty)
        #expect(shrunk.allSatisfy { $0.subtext == "はいいてんき" })
        #expect(shrunk.contains { $0.text == "今日" })

        setSegment(4)
        let expanded = candidates()
        #expect(!expanded.isEmpty)
        #expect(expanded.allSatisfy { $0.subtext == "いいてんき" })
        clear_text()
    }

    // 区切りを動かしたあとに確定すると、文節の読みだけが消え、残りは区切りを変換器に任せて変換し直す
    @Test func commitAfterMovingBoundaryLeavesRest() {
        type("kyouhaiitenki")
        setSegment(3)
        free(shrink_text(offset: 0))
        #expect(composingText.convertTarget == "はいいてんき")
        #expect(segmentSurfaceCount == nil)
        #expect(!candidates().isEmpty)
        clear_text()
    }

    // 文節は 1 文字より短くも、読み全体より長くもならない
    @Test func segmentIsClampedToReading() {
        type("kyouha")
        setSegment(0)
        #expect(segmentSurfaceCount == 1)
        #expect(candidates().allSatisfy { $0.subtext == "ょうは" })
        setSegment(100)
        #expect(segmentSurfaceCount == 4)
        #expect(candidates().allSatisfy { $0.subtext.isEmpty })
        clear_text()
    }

    // 入力・削除で区切りは変換器に任せる状態へ戻る
    @Test func typingOrDeletingResetsBoundary() {
        type("kyouha")
        setSegment(2)
        type("i")
        #expect(segmentSurfaceCount == nil)

        setSegment(2)
        let cursor = UnsafeMutablePointer<Int>.allocate(capacity: 1)
        free(remove_text(cursorPtr: cursor))
        cursor.deallocate()
        #expect(segmentSurfaceCount == nil)
        clear_text()
    }
}
}

// 再変換の読みの推定（システム辞書を逆引きする）
@Suite struct ReadingInferenceTests {
    static let index: ReadingIndex = {
        let root = URL(filePath: #filePath)
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
        let loudsURL = root
            .appendingPathComponent("azooKey_dictionary_storage")
            .appendingPathComponent("Dictionary")
            .appendingPathComponent("louds", isDirectory: true)
        return ReadingIndex(files: ReadingIndex.loadFiles(in: loudsURL))
    }()

    // 辞書にある語はその読みになる
    @Test func wordInDictionary() {
        #expect(inferReadings(for: "漢字", index: Self.index).first == "かんじ")
        #expect(inferReadings(for: "変換", index: Self.index).contains("へんかん"))
    }

    // かなだけならカタカナをひらがなにするだけ
    @Test func kanaOnly() {
        #expect(inferReadings(for: "ひらがな", index: Self.index) == ["ひらがな"])
        #expect(inferReadings(for: "カタカナ", index: Self.index) == ["かたかな"])
    }

    // 辞書に 1 語として無い文字列は、語とかなに分けて読みをつなぐ
    @Test func phraseIsSplitIntoWords() {
        let readings = inferReadings(for: "漢字を変換する", index: Self.index)
        #expect(readings == ["かんじをへんかんする"])
    }

    // 空・長すぎる文字列は推定しない
    @Test func emptyOrTooLong() {
        #expect(inferReadings(for: "", index: Self.index).isEmpty)
        #expect(inferReadings(for: String(repeating: "漢", count: maxReconversionSurfaceLength + 1), index: Self.index).isEmpty)
    }

    // 辞書の読み込みと逆引き 1 回の時間を測る（PR に書く）
    @Test func lookupTime() {
        let clock = ContinuousClock()
        let loadTime = clock.measure {
            _ = Self.index
        }
        print("reconversion dictionary load (first use): \(loadTime)")
        let lookupTime = clock.measure {
            _ = inferReadings(for: "今日は良い天気です", index: Self.index)
        }
        print("reconversion reading lookup: \(lookupTime)")
    }
}

extension GlobalStateTests {
@MainActor @Suite struct ReconversionTests {
    init() {
        let root = URL(filePath: #filePath)
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
        execURL = root.appendingPathComponent("azooKey_emoji_dictionary_storage")
        config["enable"] = false
        config["profile"] = ""
        config["context"] = ""
        learningType = .nothing
        let workURL = FileManager.default.temporaryDirectory
            .appendingPathComponent("azookey-reconversion-test-\(UUID().uuidString)", isDirectory: true)
        memoryDirectoryURL = workURL.appendingPathComponent("memory", isDirectory: true)
        userDictionaryURL = workURL.appendingPathComponent("user_dictionary", isDirectory: true)
        systemDictionaryURL = root.appendingPathComponent("azooKey_dictionary_storage").appendingPathComponent("Dictionary")
        converter = KanaKanjiConverter(dictionaryURL: systemDictionaryURL, preloadDictionary: false)
        composingText = ComposingText()
    }

    func candidates() -> [String] {
        let length = UnsafeMutablePointer<Int>.allocate(capacity: 1)
        defer { length.deallocate() }
        let list = get_composed_text(lengthPtr: length)
        return (0..<length.pointee).map { String(cString: list[$0]!.pointee.text) }
    }

    // 確定済みの文字列を読みに戻し、候補に元の文字列が出る
    @Test func startReconversionSetsReadingAndOriginalIsACandidate() {
        let hiragana = "漢字".withCString { pointer in
            let result = start_reconversion(surface: pointer)
            defer { free(result) }
            return String(cString: result)
        }
        #expect(hiragana == "かんじ")
        #expect(composingText.convertTarget == "かんじ")
        #expect(candidates().contains("漢字"))
        clear_text()
    }

    // 読みが推定できなければ入力中の文字列は空のまま
    @Test func unknownSurfaceLeavesCompositionEmpty() {
        composingText.insertAtCursorPosition("a", inputStyle: .roman2kana)
        #expect(startReconversion(surface: "") == "")
        #expect(composingText.convertTarget.isEmpty)
    }
}
}

// Zenzai を有効にした変換。zenz.gguf が要るので、環境変数 AZOOKEY_ZENZ_GGUF にその場所を渡したときだけ走らせる
// （例: AZOOKEY_ZENZ_GGUF=<リポジトリ>\zenz.gguf swift test --filter ZenzaiSessionTests）
extension GlobalStateTests {
@MainActor @Suite(.enabled(if: ProcessInfo.processInfo.environment["AZOOKEY_ZENZ_GGUF"] != nil))
struct ZenzaiSessionTests {
    let workURL: URL

    init() throws {
        let root = URL(filePath: #filePath)
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
        workURL = FileManager.default.temporaryDirectory
            .appendingPathComponent("azookey-zenzai-test-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: workURL, withIntermediateDirectories: true)
        // getOptions は execURL の下の zenz.gguf と EmojiDictionary を見るので、両方を置いた場所を execURL にする
        let gguf = URL(filePath: ProcessInfo.processInfo.environment["AZOOKEY_ZENZ_GGUF"]!)
        try FileManager.default.copyItem(at: gguf, to: workURL.appendingPathComponent("zenz.gguf"))
        try FileManager.default.copyItem(
            at: root.appendingPathComponent("azooKey_emoji_dictionary_storage").appendingPathComponent("EmojiDictionary"),
            to: workURL.appendingPathComponent("EmojiDictionary")
        )
        execURL = workURL
        config["enable"] = true
        config["profile"] = ""
        config["context"] = ""
        // 学習で順位が変わらないようにする（比べたいのは変換器の状態の持ち越しだけ）
        learningType = .nothing
        memoryDirectoryURL = workURL.appendingPathComponent("memory", isDirectory: true)
        userDictionaryURL = workURL.appendingPathComponent("user_dictionary", isDirectory: true)
        converter = KanaKanjiConverter(
            dictionaryURL: root.appendingPathComponent("azooKey_dictionary_storage").appendingPathComponent("Dictionary"),
            preloadDictionary: false
        )
        composingText = ComposingText()
    }

    func candidates() -> [String] {
        let length = UnsafeMutablePointer<Int>.allocate(capacity: 1)
        defer { length.deallocate() }
        let list = get_composed_text(lengthPtr: length)
        return (0..<length.pointee).map { String(cString: list[$0]!.pointee.text) }
    }

    /// クライアントと同じく 1 文字ごとに変換し、最後の候補と、最初の 1 文字の変換にかかった秒数を返す
    func type(_ roman: String) -> (candidates: [String], firstConversion: Double) {
        var result: [String] = []
        var firstConversion = 0.0
        for (i, character) in roman.enumerated() {
            let cursor = UnsafeMutablePointer<Int>.allocate(capacity: 1)
            free(append_text(input: String(character), cursorPtr: cursor))
            cursor.deallocate()
            let start = Date()
            result = candidates()
            if i == 0 {
                firstConversion = -start.timeIntervalSinceNow
            }
        }
        return (result, firstConversion)
    }

    func commitFirstAndClear(_ candidates: [String]) {
        if let first = candidates.first {
            first.withCString { commit_candidate(text: $0) }
        }
        clear_text()
    }

    // 確定のあとで続けて入力しても、変換器を新しくしたときと同じ候補が出る（前の入力の結果に引きずられない）
    @Test func conversionAfterCommitMatchesFreshConversion() {
        // 前の入力の読みが次の入力の読みの頭になる組（前の入力の 1 位を次の入力の制約に使い回していないか）
        let pairs = [("kyou", "kyouto"), ("kanji", "kanjiru"), ("hasi", "hasiru"), ("kouen", "kouennkai")]
        for (previous, next) in pairs {
            converter.stopComposition()
            commitFirstAndClear(type(previous).candidates)
            let carried = type(next).candidates
            clear_text()

            converter.stopComposition()
            let fresh = type(next).candidates
            clear_text()

            #expect(carried.first == fresh.first, "\(previous) → \(next)")
        }
    }

    // 確定直後の最初の変換の時間を測る（合否は付けない。結果は出力に書く）。
    // 文脈あり: クライアントと同じく、カーソルの前の文字列（確定した文字列を足していった末尾 64 文字）を文脈に渡す
    @Test(arguments: [false, true]) func measureFirstConversionAfterCommit(withContext: Bool) {
        var preceding = "明日の会議の資料をまとめてから、午後に打ち合わせの予定を確認します。"
        config["context"] = withContext ? preceding : ""
        let sentences = ["kyouhaiitenkidesune", "kanjihenkanwosuru", "asitanoyoteiwokakunin", "zenzaiwotukau", "nihongonyuuryoku"]
        // 読み込みと最初の変換を済ませておく
        commitFirstAndClear(type("junbi").candidates)

        var clears: [Double] = []
        var firsts: [Double] = []
        for round in 0..<20 {
            let (candidates, first) = type(sentences[round % sentences.count])
            firsts.append(first)
            if let text = candidates.first {
                text.withCString { commit_candidate(text: $0) }
                if withContext {
                    preceding = String((preceding + text).suffix(64))
                    config["context"] = preceding
                }
            }
            let start = Date()
            clear_text()
            clears.append(-start.timeIntervalSinceNow)
        }
        // 1 周目の最初の変換は準備の入力のあとなので、どちらも同じ条件で比べられる
        func summary(_ values: [Double]) -> String {
            let sorted = values.sorted()
            let median = sorted[sorted.count / 2]
            return String(format: "median %.1f ms / max %.1f ms (n=%d)", median * 1000, sorted.last! * 1000, sorted.count)
        }
        let label = withContext ? "context" : "no context"
        print("ZENZAI-BENCH [\(label)] ClearText: \(summary(clears))")
        print("ZENZAI-BENCH [\(label)] first conversion after commit: \(summary(firsts))")
    }
}
}
