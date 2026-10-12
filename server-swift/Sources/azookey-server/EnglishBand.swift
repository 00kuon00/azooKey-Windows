import KanaKanjiConverterModule
import Foundation

// 日英の自動判定（#26）の段 ②: 帯（候補ウィンドウの #assist）に出す英字の候補。
// ライブ変換の 1 位は変えない。英単語の区間は英字のまま・日本語の区間だけ変換して繋いだ文全体を 1 件出す

/// 帯に出す英字の候補
struct EnglishBandCandidate: Codable, Equatable, Sendable {
    /// 英単語の区間は英字のまま・日本語の区間は変換して繋いだ文全体
    var text: String
    /// 英字のまま入れた語（打ったとおり）。帯で英字の部分に印を付けるのと、確定したときに覚えるのに使う
    var words: [String]
    /// F10 で直した語・帯で選んだ語を含む（帯の先頭に出す）
    var learned: Bool
    /// 確定すると使う入力の文字数（入力全体）
    var correspondingCount: Int
}

// 設定（settings.json の english.enable）。既定は ON
@MainActor var englishEnabled = true
// 判定に使う語の一覧。Initialize で読み、設定の変更（UpdateConfig）のたびに読み直す
@MainActor var englishLexicon = EnglishLexicon()
// 帯の候補を変換する変換器。ライブ変換の変換器（前回の入力・Zenzai の状態）を動かさないように別に持つ。Zenzai は使わない
@MainActor var englishConverter: KanaKanjiConverter?
// 直近の GetEnglishCandidates で返した候補。確定の知らせ（CommitCandidate）で、英字のまま入れた語を覚える
@MainActor var lastEnglishCandidates: [EnglishBandCandidate] = []
// 自分の語彙（設定画面の「作り直す」で作る）と、F10・F6・帯で覚えた語の置き場（テストでは差し替える）
@MainActor var englishVocabularyURL: URL = azookeyAppDataURL().appendingPathComponent("english_vocabulary.json")
@MainActor var englishLearnedURL: URL = azookeyAppDataURL().appendingPathComponent("english_learned.json")
// 前回読んだときのファイルの中身。同じなら読み直さない（UpdateConfig は他の設定の変更でも届く）
@MainActor var lastEnglishFiles: (vocabulary: Data?, learned: Data?)?

/// 自分の語彙のファイル（設定画面の「作り直す」が書く）
struct EnglishVocabularyFile: Codable, Equatable {
    /// 語（小文字）と、会話ログ・Vault に出た回数
    var words: [String: Int]
    /// 日本語として打った文で英単語に取ってしまうので外した語と、取ってしまった回数
    var removed: [String: Int] = [:]
    /// 作った日時（ISO 8601）
    var builtAt: String = ""
}

/// 同梱の一覧・自分の語彙・覚えた語を読み直す。同梱の一覧は場所が変わったときだけ読む
@MainActor func reloadEnglishLexicon() {
    let wordsURL = englishWordsURL ?? execURL.appendingPathComponent("EnglishWords").appendingPathComponent("english_words.txt")
    if englishLexicon.bundledWords.isEmpty || englishBundledURL != wordsURL {
        englishLexicon.bundledWords = loadEnglishWordList(wordsURL)
        englishBundledURL = wordsURL
    }
    let vocabularyData = try? Data(contentsOf: englishVocabularyURL)
    let learnedData = try? Data(contentsOf: englishLearnedURL)
    if let last = lastEnglishFiles, last.vocabulary == vocabularyData, last.learned == learnedData {
        return
    }
    lastEnglishFiles = (vocabularyData, learnedData)
    let vocabulary = vocabularyData.flatMap { try? JSONDecoder().decode(EnglishVocabularyFile.self, from: $0) }
    englishLexicon.ownWords = Set(vocabulary.map { Array($0.words.keys) } ?? [])
    englishLexicon.ownCounts = vocabulary?.words ?? [:]
    englishLexicon.learned = learnedData.flatMap { try? JSONDecoder().decode([String: EnglishDirection].self, from: $0) } ?? [:]
}

// 前回読んだ同梱の一覧の場所
@MainActor var englishBundledURL: URL?
// 同梱の一覧の場所。nil なら execURL の下（EnglishWords\english_words.txt）。テストでは差し替える
@MainActor var englishWordsURL: URL?

/// 帯の候補を変換する変換器（無ければ作る）
@MainActor func englishBandConverter() -> KanaKanjiConverter {
    if let englishConverter {
        return englishConverter
    }
    let created = KanaKanjiConverter(dictionaryURL: systemDictionaryURL, preloadDictionary: true)
    englishConverter = created
    return created
}

/// 入力中の文字列から、帯に出す英字の候補を作る。
/// 出さないとき: 設定が OFF・同梱の一覧が読めない・文節の区切りを動かしている・再変換中（読みを推定した入力）・英単語の区間が無い・
/// 文全体がライブ変換の 1 位か帯の予測と同じ
@MainActor func englishBandCandidates() -> [EnglishBandCandidate] {
    guard englishEnabled, !englishLexicon.bundledWords.isEmpty, segmentSurfaceCount == nil, !composingText.input.isEmpty else {
        return []
    }
    var characters: [Character] = []
    for element in composingText.input {
        guard element.inputStyle == .roman2kana, case .character(let character) = element.piece else {
            return []
        }
        characters.append(character)
    }
    let spans = englishSpans(in: String(characters), lexicon: englishLexicon)
    guard !spans.isEmpty else {
        return []
    }
    // 英単語の区間は打ったとおり（direct）、ほかはローマ字として入れて、1 回で変換する
    var mixed = ComposingText()
    var position = 0
    for span in spans {
        if position < span.lowerBound {
            mixed.insertAtCursorPosition(String(characters[position..<span.lowerBound]), inputStyle: .roman2kana)
        }
        mixed.insertAtCursorPosition(String(characters[span]), inputStyle: .direct)
        position = span.upperBound
    }
    if position < characters.count {
        mixed.insertAtCursorPosition(String(characters[position...]), inputStyle: .roman2kana)
    }
    var options = getOptions(context: (config["context"] as? String) ?? "")
    options.zenzaiMode = .off
    // 入力中の予測は要らない（帯には文全体を 1 件だけ出す）
    options.requireJapanesePrediction = .disabled
    guard let best = englishBandConverter().requestCandidates(mixed, options: options).mainResults.first else {
        return []
    }
    let text = constructCandidateString(candidate: best, hiragana: mixed.convertTarget)
    // ライブ変換の 1 位・帯の予測と同じなら、帯に出しても選ぶ意味が無い（大文字の略語だけの入力など）
    guard text != lastCandidates.first?.text, !lastPredictions.contains(where: { $0.text == text }) else {
        return []
    }
    let words = spans.map { String(characters[$0]) }
    let learned = words.contains { englishLexicon.learned[$0.lowercased()] == .english }
    return [EnglishBandCandidate(text: text, words: words, learned: learned, correspondingCount: composingText.input.count)]
}

/// 帯の英字の候補を JSON（`[{"text", "words", "learned", "correspondingCount"}]`）で返す。直近の GetComposedText の入力について作る
@_silgen_name("GetEnglishCandidates")
@MainActor public func get_english_candidates() -> UnsafeMutablePointer<CChar> {
    lastEnglishCandidates = englishBandCandidates()
    let data = (try? JSONEncoder().encode(lastEnglishCandidates)) ?? Data("[]".utf8)
    return _strdup(String(decoding: data, as: UTF8.self))!
}

/// 語を覚える（F10 で英字に直した語・帯で英字の候補を選んだ語は english、F6 でひらがなに直した語は japanese）。
/// 「学習しない」「新しく学習しない」のときは覚えない
@MainActor func learnEnglishWords(_ words: [String], direction: EnglishDirection) {
    guard learningType == .inputAndOutput else { return }
    let keys = words.map { $0.lowercased() }.filter { $0.count >= 2 && $0.allSatisfy { $0.isASCII && $0.isLetter } }
    guard !keys.isEmpty else { return }
    var learned = (try? Data(contentsOf: englishLearnedURL)).flatMap { try? JSONDecoder().decode([String: EnglishDirection].self, from: $0) } ?? [:]
    for key in keys {
        learned[key] = direction
    }
    do {
        try FileManager.default.createDirectory(at: englishLearnedURL.deletingLastPathComponent(), withIntermediateDirectories: true)
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        try encoder.encode(learned).write(to: englishLearnedURL, options: .atomic)
    } catch {
        print("Failed to save english learning: \(error)")
        return
    }
    reloadEnglishLexicon()
}

/// `english` が true なら英字の向き、false ならひらがなの向きで語を覚える
@_silgen_name("LearnEnglishWord")
@MainActor public func learn_english_word(word: UnsafePointer<CChar>, english: Bool) {
    learnEnglishWords([String(cString: word)], direction: english ? .english : .japanese)
}

/// 帯の英字の候補が確定されたら、英字のまま入れた語を覚える（次から帯の先頭に出る）
@MainActor func commitEnglishCandidate(_ text: String) -> Bool {
    guard let candidate = lastEnglishCandidates.first(where: { $0.text == text }) else {
        return false
    }
    learnEnglishWords(candidate.words, direction: .english)
    return true
}
