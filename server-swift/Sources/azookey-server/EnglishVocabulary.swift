import Foundation

// 日英の自動判定（#26）の「自分の語彙」を作る。
// 会話ログ・Vault に出る英字の語のうち、自分が日本語として打つローマ字と重なる語を外す

/// ひらがな 2 文字・1 文字をローマ字にする表（ヘボン式寄り。どれも変換エンジンの roman2kana で元のかなに戻る）
private let romajiOfKana: [String: String] = {
    var table: [String: String] = [
        "あ": "a", "い": "i", "う": "u", "え": "e", "お": "o",
        "か": "ka", "き": "ki", "く": "ku", "け": "ke", "こ": "ko",
        "さ": "sa", "し": "shi", "す": "su", "せ": "se", "そ": "so",
        "た": "ta", "ち": "chi", "つ": "tsu", "て": "te", "と": "to",
        "な": "na", "に": "ni", "ぬ": "nu", "ね": "ne", "の": "no",
        "は": "ha", "ひ": "hi", "ふ": "fu", "へ": "he", "ほ": "ho",
        "ま": "ma", "み": "mi", "む": "mu", "め": "me", "も": "mo",
        "や": "ya", "ゆ": "yu", "よ": "yo",
        "ら": "ra", "り": "ri", "る": "ru", "れ": "re", "ろ": "ro",
        "わ": "wa", "を": "wo",
        "が": "ga", "ぎ": "gi", "ぐ": "gu", "げ": "ge", "ご": "go",
        "ざ": "za", "じ": "ji", "ず": "zu", "ぜ": "ze", "ぞ": "zo",
        "だ": "da", "ぢ": "di", "づ": "du", "で": "de", "ど": "do",
        "ば": "ba", "び": "bi", "ぶ": "bu", "べ": "be", "ぼ": "bo",
        "ぱ": "pa", "ぴ": "pi", "ぷ": "pu", "ぺ": "pe", "ぽ": "po",
        "ゔ": "vu",
        "ぁ": "xa", "ぃ": "xi", "ぅ": "xu", "ぇ": "xe", "ぉ": "xo",
        "ゃ": "xya", "ゅ": "xyu", "ょ": "xyo", "ゎ": "xwa", "ゕ": "xka", "ゖ": "xke",
        "しぇ": "she", "ちぇ": "che", "じぇ": "je",
        "ふぁ": "fa", "ふぃ": "fi", "ふぇ": "fe", "ふぉ": "fo",
        "てぃ": "thi", "でぃ": "dhi", "でゅ": "dhu", "とぅ": "twu", "どぅ": "dwu",
        "うぃ": "wi", "うぇ": "we", "いぇ": "ye",
        "ゔぁ": "va", "ゔぃ": "vi", "ゔぇ": "ve", "ゔぉ": "vo",
        "つぁ": "tsa", "つぃ": "tsi", "つぇ": "tse", "つぉ": "tso",
    ]
    // 拗音（きゃ・しゃ…）
    let palatal: [String: String] = [
        "き": "ky", "ぎ": "gy", "し": "sh", "じ": "j", "ち": "ch", "ぢ": "dy", "に": "ny", "ひ": "hy",
        "び": "by", "ぴ": "py", "み": "my", "り": "ry",
    ]
    for (kana, head) in palatal {
        for (small, vowel) in [("ゃ", "a"), ("ゅ", "u"), ("ょ", "o")] {
            table[kana + small] = head + vowel
        }
    }
    return table
}()

/// ひらがな（と長音）をローマ字にする。ローマ字にできない文字を含めば nil。
/// ん は母音・y・n の前と終わりで nn、ほかは n。っ は次の子音を重ねる（重ねられなければ xtu）。
/// 長音「ー」はそのまま（クライアントが「-」を「ー」にしてから変換エンジンへ渡すため）
func romanize(hiragana: String) -> String? {
    let nPlaceholder = "\u{0}"
    let characters = Array(hiragana)
    var pieces: [String] = []
    var index = 0
    var pendingSokuon = false
    func append(_ roman: String) {
        if pendingSokuon {
            if let first = roman.first, first.isLetter, !"aiueon".contains(first) {
                pieces.append(String(first))
            } else {
                pieces.append("xtu")
            }
            pendingSokuon = false
        }
        pieces.append(roman)
    }
    while index < characters.count {
        let character = characters[index]
        if character == "っ" {
            if pendingSokuon { pieces.append("xtu") }
            pendingSokuon = true
            index += 1
            continue
        }
        if character == "ー" {
            append("ー")
            index += 1
            continue
        }
        if character == "ん" {
            // n か nn かは次の文字を読んでから決める
            append(nPlaceholder)
            index += 1
            continue
        }
        if index + 1 < characters.count, let roman = romajiOfKana[String(characters[index...index + 1])] {
            append(roman)
            index += 2
            continue
        }
        guard let roman = romajiOfKana[String(character)] else {
            return nil
        }
        append(roman)
        index += 1
    }
    if pendingSokuon {
        pieces.append("xtu")
    }
    // ん の書き方を決める
    for i in pieces.indices where pieces[i] == nPlaceholder {
        let next = pieces[(i + 1)...].first?.first
        if let next, next.isASCII, next.isLetter, !"aiueoyn".contains(next) {
            pieces[i] = "n"
        } else {
            pieces[i] = "nn"
        }
    }
    return pieces.joined()
}

/// 自分の語彙を作るときの決まり
struct OwnVocabularyRules {
    /// 会話ログ・Vault に出た回数の下限（#17 の登録の提案と同じ）
    var minCount = 3
    /// 自分が日本語として打った文（ローマ字）で、その語を英単語に取ってしまう回数の上限
    var maxJapaneseHits = 0
}

/// 英字の語の回数（小文字）から、自分の語彙を作る。
/// 語ごとに、自分が日本語として打った文（`japaneseInputs`・ローマ字）でその語を英単語に取ってしまうかを判定器で確かめ、
/// 取ってしまう回数が上限を超える語を外す（made → まで・note → このテキストの「のて」など）
func buildOwnVocabulary(
    counts: [String: Int],
    japaneseInputs: [String],
    bundledWords: [String: Int],
    maxBundledSize: Int = 80,
    weights: EnglishSpanWeights = EnglishSpanWeights(),
    rules: OwnVocabularyRules = OwnVocabularyRules()
) -> (words: Set<String>, removed: [String: Int]) {
    let words = Set(counts.filter { word, count in
        count >= rules.minCount && word.count >= 2 && word.count <= maxEnglishWordLength
            && word.allSatisfy { $0.isASCII && $0.isLowercase }
    }.keys)
    // 語が部分文字列として現れる日本語の文
    var inputsByWord: [String: [Int]] = [:]
    for (index, input) in japaneseInputs.enumerated() {
        let characters = Array(input)
        var seen = Set<String>()
        for start in characters.indices {
            var word = ""
            for end in start..<min(characters.count, start + maxEnglishWordLength) {
                word.append(characters[end])
                if words.contains(word), seen.insert(word).inserted {
                    inputsByWord[word, default: []].append(index)
                }
            }
        }
    }
    // 日本語の文は読みから作ったローマ字で英字が残らないので、一覧に無い語の候補は選ばれない（調べるだけ遅くなる）
    let base = EnglishLexicon(ownWords: [], bundledWords: bundledWords, learned: [:], maxBundledSize: maxBundledSize, unknownWords: false, weights: weights)
    var removed: [String: Int] = [:]
    for (word, indices) in inputsByWord {
        var lexicon = base
        lexicon.ownWords = [word]
        lexicon.ownCounts = [word: counts[word] ?? 0]
        var hits = 0
        // 上限を超えたところで外すと決まるので、残りの文は見ない
        for index in indices where hits <= rules.maxJapaneseHits {
            let input = japaneseInputs[index]
            let characters = Array(input)
            if englishSpans(in: input, lexicon: lexicon, complete: true).contains(where: { String(characters[$0]) == word }) {
                hits += 1
            }
        }
        if hits > rules.maxJapaneseHits {
            removed[word] = hits
        }
    }
    return (words.subtracting(removed.keys), removed)
}
