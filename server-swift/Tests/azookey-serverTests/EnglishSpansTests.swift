import Testing
import Foundation
import KanaKanjiConverterModule
@testable import azookey_server

// 日英の自動判定（#26）。打ったローマ字のうち、英字のまま入れる区間を見分ける。
// 変換エンジンの ComposingText はサーバーでは MainActor からしか使わないので、テストも MainActor で 1 本ずつ走らせる
@MainActor @Suite(.serialized) struct EnglishSpansTests {
    // 同梱の一覧（SCOWL）にある語と大きさ（実際の一覧と同じ値）
    let bundled: [String: Int] = [
        "google": 50, "googled": 50, "ogle": 35, "claude": 50, "made": 35, "name": 35, "sake": 35,
        "note": 35, "as": 35, "kit": 35, "open": 35, "code": 35, "search": 35, "zen": 50,
    ]

    func spans(_ input: String, own: Set<String> = [], learned: [String: EnglishDirection] = [:]) -> [String] {
        let lexicon = EnglishLexicon(ownWords: own, bundledWords: bundled, learned: learned)
        let characters = Array(input)
        return englishSpans(in: input, lexicon: lexicon).map { String(characters[$0]) }
    }

    // kyouhagoogledekensaku → 今日はgoogleで検索（googled・ogle も一覧にあるが、区切りは google）
    @Test func englishWordInsideJapanese() {
        #expect(spans("kyouhagoogledekensaku") == ["google"])
    }

    @Test func wholeInputIsEnglish() {
        #expect(spans("claude") == ["claude"])
    }

    // ローマ字として読め、同梱の一覧にしか無い語は日本語（made → まで）
    @Test func readableWordOnlyInBundledListIsJapanese() {
        #expect(spans("made") == [])
        #expect(spans("name") == [])
        #expect(spans("sakewonomu") == [])
    }

    // 自分の語彙にある語は、ローマ字として読めても英語
    @Test func ownWordIsEnglishEvenIfReadable() {
        #expect(spans("opencode", own: ["opencode"]) == ["opencode"])
        #expect(spans("opencodewotsukau", own: ["opencode"]) == ["opencode"])
    }

    // F10 で英字に直した語は次から英字。F6 でひらがなに直した語は、自分の語彙にあっても日本語
    @Test func learnedDirectionWins() {
        #expect(spans("zenzai") == [])
        #expect(spans("zenzai", learned: ["zenzai": .english]) == ["zenzai"])
        #expect(spans("kotori", own: ["kotori"]) == ["kotori"])
        #expect(spans("kotori", own: ["kotori"], learned: ["kotori": .japanese]) == [])
        #expect(spans("claude", learned: ["claude": .japanese]) == [])
    }

    // 日本語として読めるところに、英語の語が埋まっていても取らない
    @Test func wordEmbeddedInJapaneseIsNotTaken() {
        #expect(spans("konotekisuto", own: ["note"]) == [])
        #expect(spans("asa", own: ["as"]) == [])
        #expect(spans("kitte") == [])
        #expect(spans("asahakitteiru", own: ["as"]) == [])
    }

    // 日本語だけの入力（liveConversionTopIsUnchanged の 10 入力）は、どこも英字にしない
    @Test func japaneseOnlyInputsStayJapanese() {
        let inputs = ["nihongo", "kyouhaiitenkidesune", "watashinonamaehanakanodesu", "toukyoutokkyokyokakyoku",
                      "konnnichiha", "nihonn", "kanjihenkan", "2025nen10gatu", "a", "shinkansennnonoriba"]
        for input in inputs {
            #expect(spans(input, own: ["name", "note", "open"]) == [], "\(input)")
        }
    }

    // 英単語の後ろの日本語の区間の中に、一覧に無い語の候補を作らない（追加 → [tsui]か になった）
    @Test func readableRunIsNotUnknownWord() {
        #expect(isReadableAsRomaji("tsui"))
        #expect(isReadableAsRomaji("erabi"))
        #expect(spans("zipdetsuikadekiruyounishitai", own: ["zip"]) == ["zip"])
        #expect(spans("sanshouonseinoerabikata") == [])
    }

    // 大文字で打った英字は、ローマ字にならずに残る
    @Test func uppercaseLettersStayAsTyped() {
        var text = ComposingText()
        text.insertAtCursorPosition("Gakkou", inputStyle: .roman2kana)
        #expect(text.convertTarget.hasPrefix("G"))
    }

    // ローマ字にする表が、変換エンジンで元のかなに戻ること
    @Test func romanizeRoundTrips() {
        let samples = ["きょうはいいてんきですね", "こんにちは", "しんかんせんのりば", "がっこう", "ちょっと", "きって", "ふぁいる",
                       "てぃー", "ゔぁいおりん", "かんい", "ほんや", "せんぱい", "ぜんぶ", "ほんとう", "っ", "あっ", "でぃすく", "うぇぶ"]
        for sample in samples {
            let roman = romanize(hiragana: sample)
            #expect(roman != nil, "\(sample)")
            if let roman {
                #expect(readRomaji(roman).kana == sample, "\(sample) → \(roman)")
            }
        }
        #expect(romanize(hiragana: "かんい") == "kanni")
        #expect(romanize(hiragana: "せんぱい") == "senpai")
        #expect(romanize(hiragana: "がっこう") == "gakkou")
    }
}
