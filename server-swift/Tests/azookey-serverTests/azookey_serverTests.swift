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
