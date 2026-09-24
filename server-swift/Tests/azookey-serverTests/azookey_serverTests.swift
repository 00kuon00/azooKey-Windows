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

// 学習のテストは変換器・入力中の文字列・設定のグローバルを使うので、1 本ずつ流す
@MainActor @Suite(.serialized) struct LearningTests {
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
