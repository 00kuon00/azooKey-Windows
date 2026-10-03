import KanaKanjiConverterModule
import SwiftUtils
import Foundation
import ffi

// 辞書の場所は変換器の生成時に渡すので、Initialize で execURL が決まってから作る
@MainActor var converter: KanaKanjiConverter!
@MainActor var composingText = ComposingText()

@MainActor var execURL = URL(filePath: "")
@MainActor var config: [String : Any] = [
    "enable": false,
    "profile": "",
    "backend": "cpu",
]

// 学習の設定（settings.json の learning.mode）。既定は学習する
@MainActor var learningType: LearningType = .inputAndOutput
// 学習の保存先。%APPDATA%\Azookey\memory（テストでは差し替える）
@MainActor var memoryDirectoryURL: URL = defaultMemoryDirectoryURL()
// 直近の GetComposedText で返した候補。確定の知らせ（CommitCandidate）を受けたとき、表示文字列から Candidate を引く
@MainActor var lastCandidates: [(text: String, candidate: Candidate)] = []
// 直近の GetComposedText で変換器が返した予測（入力中の読みの続きを補った語）。GetPredictions で返し、確定の知らせでも引く
@MainActor var lastPredictions: [(text: String, candidate: Candidate)] = []
// 直近の RequestTypoCorrection で返した「もしかして」。text は直した読みを変換した 1 位、candidate はその変換の候補
@MainActor var lastTypoCorrections: [(text: String, hiragana: String, candidate: Candidate)] = []
// Shift+←→ で決めた最初の文節の読みの長さ（convertTarget 上の文字数）。nil のときは区切りを変換器に任せる。
// 入力・削除・確定・消去で nil に戻す
@MainActor var segmentSurfaceCount: Int?

// システム辞書の場所。Initialize で execURL から決まる（テストでは差し替える）
@MainActor var systemDictionaryURL = URL(filePath: "")
// ユーザー辞書の元データ（%APPDATA%\Azookey\user_dictionary.json）と、それから作る辞書（%APPDATA%\Azookey\user_dictionary）
@MainActor var userDictionarySourceURL: URL = azookeyAppDataURL().appendingPathComponent("user_dictionary.json")
@MainActor var userDictionaryURL: URL = azookeyAppDataURL().appendingPathComponent("user_dictionary", isDirectory: true)
// 前回ユーザー辞書を作ったときの元データ。同じなら作り直さない（UpdateConfig は他の設定の変更でも届く）
@MainActor var lastUserDictionarySource: Data?
// 前回作ったときに登録できなかった語
@MainActor var unregisteredUserDictionaryEntries: [UserDictionaryEntry] = []

func azookeyAppDataURL() -> URL {
    let appData = ProcessInfo.processInfo.environment["APPDATA"] ?? "."
    return URL(filePath: appData).appendingPathComponent("Azookey", isDirectory: true)
}

func defaultMemoryDirectoryURL() -> URL {
    azookeyAppDataURL().appendingPathComponent("memory", isDirectory: true)
}

/// 元データが前回と変わっていれば、ユーザー辞書を作り直して変換器に読み直させる
@MainActor func reloadUserDictionary() {
    // 元データが無いときは空の辞書にする
    let source = (try? Data(contentsOf: userDictionarySourceURL)) ?? Data()
    if source == lastUserDictionarySource {
        return
    }
    do {
        let entries = source.isEmpty ? [] : try JSONDecoder().decode([UserDictionaryEntry].self, from: source)
        unregisteredUserDictionaryEntries = try buildUserDictionary(
            entries: entries,
            dictionaryURL: systemDictionaryURL,
            outputURL: userDictionaryURL
        )
        lastUserDictionarySource = source
    } catch {
        // 読めない・作れないときは前の辞書のまま使う
        print("Failed to build user dictionary: \(error)")
        return
    }
    // 同じ場所に作り直すので、forceReload で読み込み済みの辞書を捨てさせる
    converter.updateUserDictionaryURL(userDictionaryURL, forceReload: true)
    converter.stopComposition()
}

func parseLearningType(_ mode: String) -> LearningType? {
    switch mode {
    case "inputAndOutput": return .inputAndOutput
    case "onlyOutput": return .onlyOutput
    case "nothing": return .nothing
    default: return nil
    }
}

/// settings.json の zenzai.backend から、Zenzai のモデルを GPU に載せる層の数を決める。
/// どの llama.dll（llama_cpu / llama_cuda / llama_vulkan）を読むかはランチャーが起動時に PATH で決めるので、
/// ここで決めた数も起動時（Initialize）にだけ渡す
func gpuLayerCount(backend: String) -> Int32 {
    switch backend {
    // llama.cpp はモデルの層数 + 1（出力層）に切り詰めるので、多めの値で全層を載せる
    case "vulkan", "cuda": return 999
    default: return 0
    }
}

// 絵文字辞書。TextReplacer は作るたびにファイル（約 190KB）を読んで組み立てる（1 回 25 ms ほど）ので、
// 1 回だけ作って getOptions() で使い回す。場所は execURL で決まる（テストでは差し替える）ので、場所が変わったときだけ作り直す
@MainActor var emojiTextReplacer: (url: URL, replacer: TextReplacer)?

@MainActor func emojiReplacer() -> TextReplacer {
    let url = execURL.appendingPathComponent("EmojiDictionary").appendingPathComponent("emoji_all_E15.1.txt")
    if let cached = emojiTextReplacer, cached.url == url {
        return cached.replacer
    }
    let replacer = TextReplacer { url }
    emojiTextReplacer = (url, replacer)
    return replacer
}

@MainActor func getOptions(context: String = "") -> ConvertRequestOptions {
    return ConvertRequestOptions(
        // 予測は候補（mainResults）に混ぜず predictionResults に分けて受け取る（GetPredictions）。
        // .autoMix は予測を 1 位に置き、ライブ変換の表示が変わるので使わない
        requireJapanesePrediction: .manualMix,
        requireEnglishPrediction: .disabled,
        keyboardLanguage: .ja_JP,
        learningType: learningType,
        memoryDirectoryURL: memoryDirectoryURL,
        // ユーザー辞書（user.louds など）の場所。エンジンは変換のたびにここを見る
        sharedContainerURL: userDictionaryURL,
        textReplacer: emojiReplacer(),
        specialCandidateProviders: nil,
        // zenzai
        zenzaiMode: config["enable"] as! Bool ? .on(
            weight: execURL.appendingPathComponent("zenz.gguf"),
            inferenceLimit: 1,
            requestRichCandidates: true,
            personalizationMode: nil,
            versionDependentMode: .v3(
                .init(
                    profile: config["profile"] as! String,
                    leftSideContext: context
                )
            )
        ) : .off,
        preloadDictionary: true,
        metadata: .init(versionString: "Azookey for Windows")
    )
}

class SimpleComposingText {
    init(text: String, cursor: Int) {
        self.text = UnsafeMutablePointer<CChar>(mutating: text.utf8String)!
        self.cursor = cursor
    }

    var text: UnsafeMutablePointer<CChar>
    var cursor: Int
}

struct SComposingText {
    var text: UnsafeMutablePointer<CChar>
    var cursor: Int
}

func constructCandidateString(candidate: Candidate, hiragana: String) -> String {
    var remainingHiragana = hiragana
    var result = ""
    
    for data in candidate.data {
        if remainingHiragana.count < data.ruby.count {
            result += remainingHiragana
            break
        }
        remainingHiragana.removeFirst(data.ruby.count)
        result += data.word
    }
    
    return result
}

@_silgen_name("LoadConfig")
@MainActor public func load_config() {
    if let appDataPath = ProcessInfo.processInfo.environment["APPDATA"] {
        let settingsPath = URL(filePath: appDataPath).appendingPathComponent("Azookey/settings.json")
        
        do {
            let data = try Data(contentsOf: settingsPath)
            let json = try JSONSerialization.jsonObject(with: data) as? [String: Any]
            if let zenzaiDict = json?["zenzai"] as? [String: Any] {
                
                if let enableValue = zenzaiDict["enable"] as? Bool {
                    config["enable"] = enableValue
                }
                
                if let profileValue = zenzaiDict["profile"] as? String {
                    config["profile"] = profileValue
                }

                if let backendValue = zenzaiDict["backend"] as? String {
                    config["backend"] = backendValue
                }
            }

            // learning が無い（古い settings.json）ときは既定の「学習する」
            let learningDict = json?["learning"] as? [String: Any]
            learningType = (learningDict?["mode"] as? String).flatMap(parseLearningType) ?? .inputAndOutput
        } catch {
            print("Failed to read settings: \(error)")
        }
    }
    // Initialize の中では変換器を作る前に呼ばれる。そのときは変換器を作ったあとで読む
    if converter != nil {
        reloadUserDictionary()
    }
}

@_silgen_name("Initialize")
@MainActor public func initialize(
    path: UnsafePointer<CChar>,
    use_zenzai: Bool
) {
    let path = String(cString: path)
    execURL = URL(filePath: path)

    load_config()
    // Zenzai のモデルは最初の変換で読み込まれるので、変換器を作る前に渡しておく
    KanaKanjiConverterEngineRuntime.configure(gpuLayerCount: gpuLayerCount(backend: config["backend"] as! String))

    systemDictionaryURL = execURL.appendingPathComponent("Dictionary")
    converter = KanaKanjiConverter(
        dictionaryURL: systemDictionaryURL,
        preloadDictionary: true
    )
    reloadUserDictionary()
    composingText.insertAtCursorPosition("a", inputStyle: .roman2kana)
    converter.requestCandidates(composingText, options: getOptions())
    composingText = ComposingText()
}

@_silgen_name("AppendText")
@MainActor public func append_text(
    input: UnsafePointer<CChar>,
    cursorPtr: UnsafeMutablePointer<Int>
) -> UnsafeMutablePointer<CChar> {
    let inputString = String(cString: input)
    composingText.insertAtCursorPosition(inputString, inputStyle: .roman2kana)
    segmentSurfaceCount = nil

    cursorPtr.pointee = composingText.convertTargetCursorPosition    
    return _strdup(composingText.convertTarget)!
}

@_silgen_name("RemoveText")
@MainActor public func remove_text(
    cursorPtr: UnsafeMutablePointer<Int>
) -> UnsafeMutablePointer<CChar> {
    composingText.deleteBackwardFromCursorPosition(count: 1)
    segmentSurfaceCount = nil

    cursorPtr.pointee = composingText.convertTargetCursorPosition
    return _strdup(composingText.convertTarget)!
}

@_silgen_name("MoveCursor")
@MainActor public func move_cursor(
    offset: Int32,
    cursorPtr: UnsafeMutablePointer<Int>
) -> UnsafeMutablePointer<CChar> {
    let previousCursor = composingText.convertTargetCursorPosition
    let cursor = composingText.moveCursorFromCursorPosition(count: Int(offset))
    print("offset: \(offset), cursor: \(cursor)")

    cursorPtr.pointee = cursor
    return _strdup(composingText.convertTarget)!
}

@_silgen_name("ClearText")
@MainActor public func clear_text() {
    composingText = ComposingText()
    lastCandidates = []
    lastPredictions = []
    lastTypoCorrections = []
    segmentSurfaceCount = nil
    // 入力の区切り。前の入力で確定した語を、次の入力の学習の「直前の語」に持ち越さない
    // （前回の変換結果も捨てる。Zenzai の llama context は作り直さない版のエンジンを使っている・#13）
    converter.stopComposition()
}

func to_list_pointer(_ list: [FFICandidate]) -> UnsafeMutablePointer<UnsafeMutablePointer<FFICandidate>?> {
    let pointer = UnsafeMutablePointer<UnsafeMutablePointer<FFICandidate>?>.allocate(capacity: list.count)
    for (i, item) in list.enumerated() {
        pointer[i] = UnsafeMutablePointer<FFICandidate>.allocate(capacity: 1)
        pointer[i]?.pointee = item
    }
    return pointer
}

/// 候補が消費する `input` 上の文字数と、確定後に残る表示を返す。
/// FFI には従来どおり `input` 上の文字数を渡す（`ShrinkText` が `.inputCount` で戻す）。
/// `composingCount` を `input` の位置へ写すと、子音の前の単独の n（「nihongo」の n）で
/// 1 文字先まで進むことがあるので、残りの表示が一致するところまで戻す。
@MainActor func inputCount(of candidate: Candidate) -> (count: Int, remaining: String) {
    inputCount(of: candidate.composingCount)
}

@MainActor func inputCount(of composingCount: ComposingCount) -> (count: Int, remaining: String) {
    var afterComposingText = composingText
    afterComposingText.prefixComplete(composingCount: composingCount)
    let remaining = afterComposingText.convertTarget
    let rawCount = composingText.input.count - afterComposingText.input.count

    var count = rawCount
    while count > 0 {
        var trial = composingText
        trial.prefixComplete(composingCount: .inputCount(count))
        if trial.convertTarget == remaining {
            return (count, remaining)
        }
        count -= 1
    }
    return (rawCount, remaining)
}

/// 最初の文節の読みの長さを決める（Shift+←→）。範囲外の値は 1〜読みの長さに収める。
/// 候補は次の GetComposedText で、その長さの読みを 1 つの文節として変換したものになる
@_silgen_name("SetSegmentSurfaceCount")
@MainActor public func set_segment_surface_count(count: Int32) -> UnsafeMutablePointer<CChar> {
    let total = composingText.convertTarget.count
    segmentSurfaceCount = total == 0 ? nil : max(1, min(Int(count), total))
    return _strdup(composingText.convertTarget)!
}

/// 最初の文節（読みの先頭 `surfaceCount` 文字）だけを変換した候補。
/// どの候補も文節の読みをすべて使うものに絞り、確定後に残る表示は文節より後ろの読みになる
@MainActor func segmentCandidates(surfaceCount: Int) -> [FFICandidate] {
    var cursorMoved = composingText
    _ = cursorMoved.moveCursorFromCursorPosition(count: surfaceCount - cursorMoved.convertTargetCursorPosition)
    let segment = cursorMoved.prefixToCursorPosition()
    let segmentHiragana = segment.convertTarget

    let hiragana = composingText.convertTarget
    let (correspondingCount, remaining) = inputCount(of: .surfaceCount(surfaceCount))
    let contextString = (config["context"] as? String) ?? ""
    let converted = converter.requestCandidates(segment, options: getOptions(context: contextString))
    var result: [FFICandidate] = []

    for candidate in converted.mainResults {
        // 文節の途中までしか使わない候補は、区切りを動かした意味がなくなるので出さない
        var afterSegment = segment
        afterSegment.prefixComplete(composingCount: candidate.composingCount)
        guard afterSegment.convertTarget.isEmpty else {
            continue
        }
        let candidateString = constructCandidateString(candidate: candidate, hiragana: segmentHiragana)
        lastCandidates.append((candidateString, candidate))
        result.append(FFICandidate(text: strdup(candidateString), subtext: strdup(remaining), hiragana: strdup(hiragana), correspondingCount: Int32(correspondingCount)))
    }
    // 読みをすべて使う候補が 1 つも無いときは読みのまま出す（学習の対象にはしない）
    if result.isEmpty {
        result.append(FFICandidate(text: strdup(segmentHiragana), subtext: strdup(remaining), hiragana: strdup(hiragana), correspondingCount: Int32(correspondingCount)))
    }
    return result
}

@_silgen_name("GetComposedText")
@MainActor public func get_composed_text(lengthPtr: UnsafeMutablePointer<Int>) -> UnsafeMutablePointer<UnsafeMutablePointer<FFICandidate>?> {
    // 読みが変わったら、前の読みの「もしかして」は使わない
    lastTypoCorrections = []
    if let segmentSurfaceCount {
        lastCandidates = []
        // 文節の区切りを動かしている間は、読み全体の続きを補う予測は出さない
        lastPredictions = []
        let result = segmentCandidates(surfaceCount: segmentSurfaceCount)
        lengthPtr.pointee = result.count
        return to_list_pointer(result)
    }
    let hiragana = composingText.convertTarget
    let contextString = (config["context"] as? String) ?? ""
    let options = getOptions(context: contextString)
    let converted = converter.requestCandidates(composingText, options: options)
    var result: [FFICandidate] = []
    lastCandidates = []
    lastPredictions = predictions(from: converted, hiragana: hiragana)

    for i in 0..<converted.mainResults.count {
        let candidate = converted.mainResults[i]

        let candidateString = constructCandidateString(candidate: candidate, hiragana: hiragana)
        lastCandidates.append((candidateString, candidate))
        let text = strdup(candidateString)
        let hiragana = strdup(hiragana)

        let (correspondingCount, remaining) = inputCount(of: candidate)
        let subtext = strdup(remaining)

        result.append(FFICandidate(text: text, subtext: subtext, hiragana: hiragana, correspondingCount: Int32(correspondingCount)))        
    }

    lengthPtr.pointee = result.count

    return to_list_pointer(result)
}

/// 予測のうち、読みの続きを補ったものだけを返す（通常の候補と同じ語・読みそのままは出さない）。
/// 予測は語を切らずにそのまま出す（constructCandidateString を通すと打った読みの長さで切られる）
@MainActor func predictions(from converted: ConversionResult, hiragana: String) -> [(text: String, candidate: Candidate)] {
    var seen = Set(converted.mainResults.map { constructCandidateString(candidate: $0, hiragana: hiragana) })
    seen.insert(hiragana)
    var result: [(text: String, candidate: Candidate)] = []
    for candidate in converted.predictionResults where !seen.contains(candidate.text) {
        seen.insert(candidate.text)
        result.append((candidate.text, candidate))
    }
    return result
}

/// 直近の GetComposedText の予測。確定すると入力中の文字列をすべて使うので、subtext は空、correspondingCount は入力全体
@_silgen_name("GetPredictions")
@MainActor public func get_predictions(lengthPtr: UnsafeMutablePointer<Int>) -> UnsafeMutablePointer<UnsafeMutablePointer<FFICandidate>?> {
    let hiragana = composingText.convertTarget
    let count = Int32(composingText.input.count)
    let result = lastPredictions.map {
        FFICandidate(text: strdup($0.text), subtext: strdup(""), hiragana: strdup(hiragana), correspondingCount: count)
    }
    lengthPtr.pointee = result.count
    return to_list_pointer(result)
}

/// 打ち間違いを直した「もしかして」を返す（変換エンジンの experimentalRequestTypoCorrection）。
/// 言語モデルに zenz を使うので、Zenzai が無効なら何もしない（エンジンも空を返す）。
/// 直した読みを変換した 1 位を text、直した読みを hiragana に入れる。確定すると入力中の文字列をすべて使う。
/// 学習は直した読みの変換の候補で行う（打ち間違えた読みでは覚えない）
@_silgen_name("RequestTypoCorrection")
@MainActor public func request_typo_correction(lengthPtr: UnsafeMutablePointer<Int>) -> UnsafeMutablePointer<UnsafeMutablePointer<FFICandidate>?> {
    lastTypoCorrections = []
    let typed = composingText
    let options = getOptions(context: (config["context"] as? String) ?? "")
    // getOptions と同じく設定の enable で Zenzai の有無を見る（zenzaiMode.enabled はエンジンの外から見えない）
    guard config["enable"] as? Bool == true, segmentSurfaceCount == nil, !typed.input.isEmpty else {
        lengthPtr.pointee = 0
        return to_list_pointer([])
    }
    let corrections = converter.experimentalRequestTypoCorrection(
        leftSideContext: (config["context"] as? String) ?? "",
        composingText: typed,
        options: options,
        inputStyle: .roman2kana,
        config: typoCorrectionConfig
    )
    let shown = Set(lastCandidates.map(\.text) + lastPredictions.map(\.text))
    var convertedCorrection = false
    // 直した読みの変換で変換器の「前回の入力」が変わるので、打った読みで変換し直して戻す。
    // 戻さないと次の 1 文字の変換が直した読みに引きずられる（「arigatoi」の補正のあと「u」で「ありがと謂う」になった）
    defer {
        if convertedCorrection {
            _ = converter.requestCandidates(typed, options: options)
        }
    }
    for correction in corrections {
        guard lastTypoCorrections.count < maxTypoCorrections else { break }
        // エンジンは直した読みをカタカナで返す
        let hiragana = correction.convertedText.toHiragana()
        // 打った読みより下は出さない（正しく打った入力では、打った読みが 1 位に来る）
        if hiragana == typed.convertTarget { break }
        guard isLikelyTypoCorrection(correction, hiragana: hiragana, typed: typed.convertTarget),
              !lastTypoCorrections.contains(where: { $0.hiragana == hiragana }) else { continue }
        var corrected = ComposingText()
        corrected.insertAtCursorPosition(correction.correctedInput, inputStyle: .roman2kana)
        convertedCorrection = true
        guard let best = converter.requestCandidates(corrected, options: options).mainResults.first else { continue }
        let text = constructCandidateString(candidate: best, hiragana: corrected.convertTarget)
        // 通常の候補・予測に同じ語があるなら、そちらを選べばよい
        guard !shown.contains(text) else { continue }
        lastTypoCorrections.append((text, hiragana, best))
    }
    let count = Int32(typed.input.count)
    let result = lastTypoCorrections.map {
        FFICandidate(text: strdup($0.text), subtext: strdup(""), hiragana: strdup($0.hiragana), correspondingCount: count)
    }
    lengthPtr.pointee = result.count
    return to_list_pointer(result)
}

/// 「もしかして」を出す数の上限
let maxTypoCorrections = 2

/// 「もしかして」に出してよい補正か。
/// - 1 位との相対的な重み（prominence）が小さいものは、打ち間違い 10 件でどれも意味の通らない読みだった（「このい」0.08 など）
/// - 数字を変える補正は出さない（正しく打った「2025nen10gatu」に「2015ねん10がつ」が出た）
/// - ローマ字が残る読み（「によmn」など）は出さない
func isLikelyTypoCorrection(_ correction: ZenzaiTypoCandidate, hiragana: String, typed: String) -> Bool {
    let digits = { (text: String) in text.filter(\.isASCII).filter(\.isNumber) }
    return correction.prominence >= minTypoProminence
        && digits(hiragana) == digits(typed)
        && !hiragana.contains(where: { $0.isASCII && $0.isLetter })
}

/// 「もしかして」に出す補正の重み（prominence）の下限
let minTypoProminence: Float = 0.1
/// 打ち間違い補正の探索の広さ。エンジンの既定（beamSize 32・topK 64）は Vulkan・GPU 全層で 1 回 682 ms（中央値）かかる。
/// 4・8 に狭めると 150 ms で、打ち間違い 10 件で正しい読みが 1 位に来る数は同じ（6/10）だった（measureTypoCorrection）
let typoCorrectionConfig = ExperimentalTypoCorrectionConfig(beamSize: 4, topK: 8)

@_silgen_name("ShrinkText")
@MainActor public func shrink_text(
    offset: Int32
) -> UnsafeMutablePointer<CChar>  {
    var afterComposingText = composingText
    // 文節の区切りを動かしたあとは、文節の読みの長さで確定する（どの候補も文節の読みをすべて使う）
    if let segmentSurfaceCount {
        afterComposingText.prefixComplete(composingCount: .surfaceCount(segmentSurfaceCount))
    } else {
        afterComposingText.prefixComplete(composingCount: .inputCount(Int(offset)))
    }
    composingText = afterComposingText
    segmentSurfaceCount = nil

    return _strdup(composingText.convertTarget)!
}

@_silgen_name("SetContext")
@MainActor public func set_context(
    context: UnsafePointer<CChar>
) {
    let contextString = String(cString: context)
    config["context"] = contextString
}

/// 英数・記号だけの確定（全角の英数を含む）は学習しない
func shouldLearn(_ text: String) -> Bool {
    let isAlphanumericOnly = text.unicodeScalars.allSatisfy { scalar in
        scalar.isASCII
            || (0xFF01...0xFF5E).contains(scalar.value) // 全角の英数・記号
            || scalar.value == 0x3000 // 全角スペース
    }
    return !text.isEmpty && !isAlphanumericOnly
}

/// 候補が確定したことを受け取り、学習する。
/// `text` はクライアントに返した候補の文字列。クライアント（Rust）は同じ文字列の候補を最初の 1 件だけ残すので、
/// ここでも最初に一致したものを取る
@_silgen_name("CommitCandidate")
@MainActor public func commit_candidate(text: UnsafePointer<CChar>) {
    let text = String(cString: text)
    // 「新しく学習しない」「学習しない」では学習の保存先に書き込まない
    guard learningType == .inputAndOutput, shouldLearn(text) else {
        return
    }
    guard let candidate = lastCandidates.first(where: { $0.text == text })?.candidate
            ?? lastPredictions.first(where: { $0.text == text })?.candidate
            ?? lastTypoCorrections.first(where: { $0.text == text })?.candidate else {
        print("CommitCandidate: candidate not found: \(text)")
        return
    }

    do {
        try FileManager.default.createDirectory(at: memoryDirectoryURL, withIntermediateDirectories: true)
    } catch {
        print("Failed to create memory directory: \(error)")
        return
    }
    converter.updateLearningData(candidate)
    converter.commitUpdateLearningData()
}

/// 候補の学習だけを忘れる（候補選択中の Ctrl+Delete）。入力中の文字列はそのまま残し、それを返す。
/// `text` の探し方は CommitCandidate と同じ。学習の保存先だけを消すので、ユーザー辞書の語は残る。
/// 「新しく学習しない」「学習しない」のときは、変換エンジンが何もしない
@_silgen_name("ForgetCandidate")
@MainActor public func forget_candidate(text: UnsafePointer<CChar>) -> UnsafeMutablePointer<CChar> {
    let text = String(cString: text)
    if let candidate = lastCandidates.first(where: { $0.text == text })?.candidate {
        converter.forgetMemory(candidate)
        // 同じ入力で取り直すと前回の変換結果が使い回され、忘れる前の順位のまま出るので捨てさせる
        converter.stopComposition()
    } else {
        print("ForgetCandidate: candidate not found: \(text)")
    }
    return _strdup(composingText.convertTarget)!
}

/// 直近のユーザー辞書の作り直しで登録できなかった語を JSON（`[{"reading": ..., "word": ...}]`）で返す
@_silgen_name("GetUnregisteredUserDictionaryEntries")
@MainActor public func get_unregistered_user_dictionary_entries() -> UnsafeMutablePointer<CChar> {
    let data = (try? JSONEncoder().encode(unregisteredUserDictionaryEntries)) ?? Data("[]".utf8)
    return _strdup(String(decoding: data, as: UTF8.self))!
}

/// 学習をすべて消す
@_silgen_name("ResetLearning")
@MainActor public func reset_learning() {
    // resetMemory は直近の変換で渡した保存先（Initialize で一度変換しているので必ずある）を消す
    converter.resetMemory()
    converter.stopComposition()
}
