import Testing
import Foundation
import KanaKanjiConverterModule
@testable import azookey_server

// 日英の自動判定（#26）の段 ②: 帯に出す英字の候補（文全体）。ライブ変換の 1 位は変えない
extension GlobalStateTests {
@MainActor @Suite struct EnglishBandTests {
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
        config["context"] = ""
        workURL = FileManager.default.temporaryDirectory
            .appendingPathComponent("azookey-english-band-test-\(UUID().uuidString)", isDirectory: true)
        memoryDirectoryURL = workURL.appendingPathComponent("memory", isDirectory: true)
        userDictionaryURL = workURL.appendingPathComponent("user_dictionary", isDirectory: true)
        learningType = .inputAndOutput
        dictionaryURL = root.appendingPathComponent("azooKey_dictionary_storage").appendingPathComponent("Dictionary")
        systemDictionaryURL = dictionaryURL
        converter = KanaKanjiConverter(dictionaryURL: dictionaryURL, preloadDictionary: false)
        englishConverter = nil
        englishEnabled = true
        englishWordsURL = root.appendingPathComponent("EnglishWords").appendingPathComponent("english_words.txt")
        englishVocabularyURL = workURL.appendingPathComponent("english_vocabulary.json")
        englishLearnedURL = workURL.appendingPathComponent("english_learned.json")
        lastEnglishFiles = nil
        reloadEnglishLexicon()
        clear_text()
    }

    /// 自分の語彙のファイルを置いて読み直す
    func setOwnWords(_ words: [String: Int]) throws {
        try FileManager.default.createDirectory(at: workURL, withIntermediateDirectories: true)
        try JSONEncoder().encode(EnglishVocabularyFile(words: words)).write(to: englishVocabularyURL)
        reloadEnglishLexicon()
    }

    /// ローマ字を 1 文字ずつ打ち、ライブ変換の 1 位と帯の英字の候補を返す
    func type(_ roman: String) -> (top: String?, band: [EnglishBandCandidate]) {
        for character in roman {
            let cursor = UnsafeMutablePointer<Int>.allocate(capacity: 1)
            free(append_text(input: String(character), cursorPtr: cursor))
            cursor.deallocate()
        }
        let length = UnsafeMutablePointer<Int>.allocate(capacity: 1)
        defer { length.deallocate() }
        let list = get_composed_text(lengthPtr: length)
        let top = length.pointee > 0 ? String(cString: list[0]!.pointee.text) : nil
        let json = get_english_candidates()
        defer { free(json) }
        let band = (try? JSONDecoder().decode([EnglishBandCandidate].self, from: Data(String(cString: json).utf8))) ?? []
        return (top, band)
    }

    /// 帯を出さないときと同じ 1 位か（英字の候補を作っても、ライブ変換の 1 位は変わらない）
    func topWithoutEnglish(_ roman: String) -> String? {
        clear_text()
        englishEnabled = false
        defer { englishEnabled = true; clear_text() }
        return type(roman).top
    }

    // 固定の例（帯の候補として）
    @Test func fixedExamplesInBand() throws {
        try setOwnWords(["opencode": 207])
        for (input, expected) in [("kyouhagoogledekensaku", "今日はgoogleで検索"), ("claude", "claude"), ("opencode", "opencode")] {
            let (top, band) = type(input)
            #expect(band.map(\.text) == [expected], "\(input)")
            #expect(band.first?.correspondingCount == input.count)
            #expect(top == topWithoutEnglish(input), "\(input) のライブ変換の 1 位")
        }
        // 同梱の一覧にしか無い、ローマ字として読める語は日本語。帯に英字の候補を出さない
        let (top, band) = type("made")
        #expect(band.isEmpty)
        #expect(top == topWithoutEnglish("made"))
        clear_text()
    }

    /// 1 文字ずつ打ったときの、打鍵ごとのライブ変換の 1 位と帯の英字の候補
    func typeEach(_ roman: String, english: Bool) -> [(top: String?, band: [EnglishBandCandidate])] {
        clear_text()
        englishEnabled = english
        defer { englishEnabled = true; clear_text() }
        return roman.map { type(String($0)) }
    }

    // 日本語だけの入力（liveConversionTopIsUnchanged の 10 入力・136 打鍵）では、帯に英字の候補を出さず、どの打鍵でも 1 位は変わらない。
    // 英字の候補が出る入力でも、どの打鍵でも 1 位は変わらない
    @Test func liveConversionTopIsUnchangedWithEnglish() throws {
        try setOwnWords(["opencode": 207])
        let japanese = ["nihongo", "kyouhaiitenkidesune", "watashinonamaehanakanodesu", "toukyoutokkyokyokakyoku",
                        "konnnichiha", "nihonn", "kanjihenkan", "2025nen10gatu", "a", "shinkansennnonoriba"]
        let mixed = ["kyouhagoogledekensaku", "claude", "made", "opencode", "konnshuuhaopencodedePRwodasu"]
        for input in japanese + mixed {
            let with = typeEach(input, english: true)
            let without = typeEach(input, english: false)
            #expect(with.map(\.top) == without.map(\.top), "\(input)")
            if japanese.contains(input) {
                #expect(with.allSatisfy { $0.band.isEmpty }, "\(input)")
            }
        }
    }

    // F10 で直した語・帯で選んだ語は、次から帯の先頭（learned）。F6 で直した語は英字の候補に入れない
    @Test func learnedWordsGoFirstAndF6Excludes() {
        // zenzai は一覧に無く、ローマ字として読める（ぜんざい）ので、覚えるまで帯に出ない
        #expect(type("zenzaiwotsukau").band.isEmpty)
        clear_text()
        "zenzai".withCString { learn_english_word(word: $0, english: true) }
        let learned = type("zenzaiwotsukau").band
        #expect(learned.map(\.text) == ["zenzaiを使う"])
        #expect(learned.first?.learned == true)
        clear_text()

        // 帯で選んで確定した語は覚える
        let chosen = type("kyouhagoogledekensaku").band
        #expect(chosen.first?.learned == false)
        chosen[0].text.withCString { commit_candidate(text: $0) }
        clear_text()
        #expect(type("googlenitsuite").band.first?.learned == true)
        clear_text()

        // F6 でひらがなに直した語は、英字の候補に入れない
        #expect(!type("claude").band.isEmpty)
        clear_text()
        "claude".withCString { learn_english_word(word: $0, english: false) }
        #expect(type("claude").band.isEmpty)
        clear_text()
    }

    // 設定で OFF にすると出さない
    @Test func disabledHasNoBand() {
        englishEnabled = false
        #expect(type("kyouhagoogledekensaku").band.isEmpty)
        englishEnabled = true
        clear_text()
    }

    // 文節の区切りを動かしている間は出さない
    @Test func noBandWhileSegmenting() {
        _ = type("kyouhagoogledekensaku")
        free(set_segment_surface_count(count: 4))
        let length = UnsafeMutablePointer<Int>.allocate(capacity: 1)
        defer { length.deallocate() }
        _ = get_composed_text(lengthPtr: length)
        let json = get_english_candidates()
        #expect(String(cString: json) == "[]")
        free(json)
        clear_text()
    }
}
}
