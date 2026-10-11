import KanaKanjiConverterModule
import Foundation

// 日英の自動判定（#26）。ローマ字で打った入力のうち、英単語の区間を見分ける。
// 考え方だけ Meltype（打鍵ごとに日本語と英語の点を比べる）から借りた。コード・辞書は使っていない。

/// F10（英字に直す）・F6（ひらがなに直す）で直した向き
enum EnglishDirection: String, Codable, Sendable {
    case english
    case japanese
}

/// 英単語の区間の選び方の重み。
/// 区切り方ごとに「日本語の区間でかなにならずに残る英字」と「英単語の区間」の点を足し、いちばん小さいものを選ぶ。
/// 英単語の区間をまったく取らない（全部を日本語として読む）区切り方の点は 0 で、必ず比べる。
/// 値は会話ログの調整用の期間（2026-09-01〜09-20 の発言・語彙はそれより前）で決めた（#26）
struct EnglishSpanWeights: Sendable {
    /// 日本語の区間で、かなにならずに残る英字 1 文字あたり
    var leftoverLetter: Float = 10
    /// 英単語の区間 1 つあたり。自分の語彙（後ろが助詞なら取れる値）
    var ownWord: Float = 0.9
    /// 自分の語彙の語の回数（log10）1 あたりに引く点
    var ownFrequency: Float = 0.2
    /// 自分の語彙のうち 2 文字の語に足す点
    var shortOwnWord: Float = 0.6
    /// 同梱の一覧にしか無い語（大きさ 35 のとき。大きさが 50 増えるごとに +1）。
    /// 境目の点をすべて足しても正のままの値にして、日本語として読めるところでは取らせない
    var bundledWord: Float = 2.0
    /// どの一覧にも無い語（単独ではローマ字として読めないもの）。境目の点の合計（1.5）より小さくして、
    /// 英単語 + 助詞 + 英単語を 1 語にまとめるより 2 語に分けるほうを安くする
    var unknownWord: Float = 1.2
    /// どの一覧にも無い語のうち、かなとして読める英字 1 文字あたり。日本語を飲み込んだ候補ほど高くなる。
    /// 日本語の中の kit + と（きっと）のような候補が 0 を下回らない値にする（1.2 + 0.3 × 2 − 1.5 = 0.3）
    var unknownKanaLetter: Float = 0.3
    /// 大文字だけの並び（大文字はかなにならないので、日本語として読むと残る英字になる）
    var uppercaseRun: Float = 0.5
    /// F10 で英字に直した語
    var learnedEnglish: Float = -1
    /// 英単語のすぐ後ろが助詞などで始まる・入力の終わり
    var particleAfter: Float = -1
    /// 英単語のすぐ前が入力の始まり・助詞などで終わる
    var particleBefore: Float = -0.5
}

/// 英単語の区間を見分けるのに使う語の一覧。語はすべて小文字
struct EnglishLexicon: Sendable {
    /// 自分の語彙（会話ログ・Vault の英字の語）。日本語として打つローマ字と重なる語は、作るときに外してある
    var ownWords: Set<String> = []
    /// 自分の語彙の語が会話ログ・Vault に出た回数。多い語ほど英単語に取りやすくする（無ければ足さない）
    var ownCounts: [String: Int] = [:]
    /// 同梱の英単語の一覧（SCOWL）。値は大きさ（35〜80。小さいほどよく使う語）
    var bundledWords: [String: Int] = [:]
    /// F10 / F6 で直した語。自分の語彙・同梱の一覧より優先する
    var learned: [String: EnglishDirection] = [:]
    /// 同梱の一覧のうち、この大きさまでの語を使う
    var maxBundledSize: Int = 80
    /// どの一覧にも無い語も、日本語として読むと英字が残るところでは英単語にする
    var unknownWords: Bool = true
    var weights = EnglishSpanWeights()
}

/// 英単語のすぐ後ろに来る日本語の頭（会話ログで英単語の直後に来るひらがなの 96%。「っ」は「って」）
let kanaAfterEnglish: Set<Character> = ["の", "を", "に", "は", "で", "が", "と", "も", "や", "か", "な", "だ", "へ", "っ"]
/// 英単語のすぐ前に来る日本語の終わり（助詞・て形）
let kanaBeforeEnglish: Set<Character> = ["の", "は", "と", "に", "を", "で", "が", "も", "や", "て"]
/// 助詞などのローマ字。一覧に無い語の候補の終わりを決めるのに使う
private let particleRomaji: [[Character]] = ["no", "wo", "ni", "ha", "de", "ga", "to", "mo", "ya", "ka", "na", "da", "he", "tte"].map(Array.init)

/// 英単語として見る語の長さの上限（文字数）
let maxEnglishWordLength = 24

/// ローマ字（`.roman2kana`）として読んだ結果
struct RomajiReading: Equatable {
    /// 読んだかな（かなにならなかった英字はそのまま）
    var kana: String
    /// かなにならずに残った英字の数
    var leftover: Int
}

/// `roman` をローマ字として読む。
/// 終わりの「n」1 つは、続けて打てば「ん」になるので残りに数えない（`nBeforeVowel` なら、後ろに母音・y が続くので数える）。
/// `pendingTail` なら、終わりに続く英字（打っている途中の子音）も残りに数えない。
/// 日本語を打つときに使わない英字（小さいかなの l・q・ch 以外の c）は、かなになっても残りに数える
/// （lounge → ぉうんげ・core → これ を日本語として読ませない）
func readRomaji(_ roman: some StringProtocol, pendingTail: Bool = false, nBeforeVowel: Bool = false) -> RomajiReading {
    var input = Array(roman)
    var text = ComposingText()
    text.insertAtCursorPosition(String(input), inputStyle: .roman2kana)
    var kana = Array(text.convertTarget)
    if pendingTail {
        while let last = kana.last, last.isASCII, last.isLetter {
            kana.removeLast()
            input.removeLast()
        }
    } else if kana.last == "n" && !nBeforeVowel {
        kana[kana.count - 1] = "ん"
    }
    let leftover = kana.count(where: { $0.isASCII && $0.isLetter })
        + max(0, unusualLetterCount(input) - unusualLetterCount(kana.filter(\.isASCII)))
    return RomajiReading(kana: String(kana), leftover: leftover)
}

/// 日本語を打つときに使わない英字の数（l・q と、後ろに h も c も続かない c。cchi は っち）
private func unusualLetterCount(_ characters: [Character]) -> Int {
    var count = 0
    for (index, character) in characters.enumerated() {
        let lower = Character(character.lowercased())
        let next = index + 1 < characters.count ? Character(characters[index + 1].lowercased()) : nil
        if lower == "l" || lower == "q" || (lower == "c" && next != "h" && next != "c") {
            count += 1
        }
    }
    return count
}

// 語ごとの「ローマ字として読んだときに残る英字の数」。同梱の一覧の語は多く（約 20 万語）、毎回読むと遅いので覚えておく。
// テストは並行に走るので、ロックで守る
nonisolated(unsafe) private var romajiLeftovers: [String: Int] = [:]
private let romajiLeftoversLock = NSLock()

/// 語を単独でローマ字として読んだときに、かなにならずに残る英字の数
func romajiLeftover(_ word: String) -> Int {
    if let known = romajiLeftoversLock.withLock({ romajiLeftovers[word] }) { return known }
    let leftover = readRomaji(word).leftover
    romajiLeftoversLock.withLock { romajiLeftovers[word] = leftover }
    return leftover
}

/// 語が単独でローマ字として読めるか（かなにならずに残る英字が無いか）
func isReadableAsRomaji(_ word: String) -> Bool {
    romajiLeftover(word) == 0
}

/// 英単語の区間の候補
struct EnglishCandidate {
    var range: Range<Int>
    var cost: Float
}

/// `input`（打った文字の並び）のうち、英字のまま入れる区間を返す。範囲は `input` の文字の位置で、隣り合う区間は 1 つにまとめる。
/// - 自分の語彙・F10 で直した語は、ローマ字として読めても英単語にする（後ろの境目が日本語の助詞などのとき）
/// - 同梱の一覧にしか無い語・どの一覧にも無い語は、日本語として読むと英字が残るところでだけ英単語にする
/// - F6 で直した語は英単語にしない
/// `complete` が false（打っている途中）なら、終わりの子音（kyouh の h）は打ちかけとみなして残りに数えない
func englishSpans(in input: String, lexicon: EnglishLexicon, complete: Bool = false) -> [Range<Int>] {
    let characters = Array(input)
    let count = characters.count
    guard count > 0, characters.contains(where: { $0.isASCII && $0.isLetter }) else {
        return []
    }
    // 全体が日本語として読める（英字が残らない）なら、同梱の一覧・一覧に無い語の候補は点が正のままで選ばれないので作らない
    let wholeReading = readRomaji(String(characters), pendingTail: !complete)
    let candidates = englishCandidates(characters, lexicon: lexicon, ownWordsOnly: wholeReading.leftover == 0)
    guard !candidates.isEmpty else {
        return []
    }
    let weights = lexicon.weights
    let byStart = Dictionary(grouping: candidates, by: \.range.lowerBound)

    // 日本語の区間の読み。同じ区間を何度も読まない
    var readings: [Range<Int>: RomajiReading] = [0..<count: wholeReading]
    func reading(_ range: Range<Int>) -> RomajiReading {
        if let cached = readings[range] { return cached }
        // 英単語が母音・y で始まるなら、その前の n 1 つは「ん」ではない（ん は nn と打つ）
        let nBeforeVowel = range.upperBound < count && "aiueoy".contains(characters[range.upperBound].lowercased())
        let result = readRomaji(String(characters[range]), pendingTail: !complete && range.upperBound == count, nBeforeVowel: nBeforeVowel)
        readings[range] = result
        return result
    }

    // best[b] = 先頭から b までを区切った最良の点（b は入力の始まりか、英単語の区間の終わり）。
    // 同じ点なら英字の多いほう（同じ残りの英字を、長い語で覆うほう）
    typealias Path = (cost: Float, englishCount: Int, spans: [Range<Int>])
    func isBetter(_ lhs: Path, than rhs: Path) -> Bool {
        lhs.cost != rhs.cost ? lhs.cost < rhs.cost : lhs.englishCount > rhs.englishCount
    }
    var best: [Int: Path] = [0: (0, 0, [])]
    var result: Path?
    let boundaries = Set([0] + candidates.map(\.range.upperBound)).sorted()
    let starts = byStart.keys.sorted() + [count]
    for boundary in boundaries {
        guard let path = best[boundary] else { continue }
        // 区切り b から、次の英単語の区間 [s, e) へ（間の [b, s) は日本語）。s = 入力の終わりなら残りはすべて日本語
        for start in starts where start >= boundary {
            let gap = boundary..<start
            // 英単語どうしが隣り合うところ（間の日本語が無い）には境目の点を付けない（短い語を連ねるほど得になるため）
            if gap.isEmpty && boundary > 0 && start < count {
                for next in byStart[start] ?? [] {
                    let candidate: Path = (path.cost + next.cost, path.englishCount + next.range.count, path.spans + [next.range])
                    if best[next.range.upperBound].map({ isBetter(candidate, than: $0) }) ?? true {
                        best[next.range.upperBound] = candidate
                    }
                }
                continue
            }
            let gapReading = gap.isEmpty ? nil : reading(gap)
            var cost = path.cost + Float(gapReading?.leftover ?? 0) * weights.leftoverLetter
            // 前の英単語の後ろの境目（助詞などで始まる・入力の終わり）
            if boundary > 0 {
                if let kana = gapReading?.kana, let first = kana.first {
                    // 「っ」は「って」のときだけ（gltf + っふぁいる のように、子音を重ねた読みの頭は助詞ではない）
                    if kanaAfterEnglish.contains(first) && (first != "っ" || kana.dropFirst().first == "て") {
                        cost += weights.particleAfter
                    }
                } else {
                    cost += weights.particleAfter
                }
            }
            if start == count {
                let candidate: Path = (cost, path.englishCount, path.spans)
                if result.map({ isBetter(candidate, than: $0) }) ?? true {
                    result = candidate
                }
                break
            }
            guard let nexts = byStart[start] else { continue }
            // 次の英単語の前の境目（入力の始まり・助詞などで終わる）。助詞はその前に語があるとき（2 かな以上）だけ
            var before: Float = 0
            if let kana = gapReading?.kana {
                if kana.count >= 2, let last = kana.last, kanaBeforeEnglish.contains(last) { before = weights.particleBefore }
            } else {
                before = weights.particleBefore
            }
            for next in nexts {
                let candidate: Path = (cost + before + next.cost, path.englishCount + next.range.count, path.spans + [next.range])
                if best[next.range.upperBound].map({ isBetter(candidate, than: $0) }) ?? true {
                    best[next.range.upperBound] = candidate
                }
            }
        }
    }
    var merged: [Range<Int>] = []
    for span in result?.spans ?? [] {
        if let last = merged.last, last.upperBound == span.lowerBound {
            merged[merged.count - 1] = last.lowerBound..<span.upperBound
        } else {
            merged.append(span)
        }
    }
    return merged
}

/// 英単語の区間の候補と、その点（境目の点は含まない）。
/// `ownWordsOnly` なら、自分の語彙・F10 で直した語・大文字の並びだけ
func englishCandidates(_ characters: [Character], lexicon: EnglishLexicon, ownWordsOnly: Bool = false) -> [EnglishCandidate] {
    let weights = lexicon.weights
    var candidates: [EnglishCandidate] = []
    // F6 でひらがなに直した語が出てくる位置。そこにかかる候補は、一覧に無い語の候補も含めて作らない
    var japaneseRanges: [Range<Int>] = []
    let isLetter = { (c: Character) in c.isASCII && c.isLetter }
    for start in characters.indices where isLetter(characters[start]) {
        var word = ""
        var end = start
        while end < characters.count, end - start < maxEnglishWordLength, isLetter(characters[end]) {
            word += characters[end].lowercased()
            end += 1
            guard word.count >= 2 else { continue }
            let range = start..<end
            switch lexicon.learned[word] {
            case .japanese:
                japaneseRanges.append(range)
                continue
            case .english:
                candidates.append(EnglishCandidate(range: range, cost: weights.learnedEnglish))
                continue
            case nil:
                break
            }
            if lexicon.ownWords.contains(word) {
                // 2 文字の語は、英文の断片や略語の一部（ts・at）が多いので弱くする
                let frequency = lexicon.ownCounts[word].map { weights.ownFrequency * log10(Float($0)) } ?? 0
                candidates.append(EnglishCandidate(range: range, cost: weights.ownWord + (word.count == 2 ? weights.shortOwnWord : 0) - frequency))
            } else if !ownWordsOnly, word.count >= 3, let size = lexicon.bundledWords[word], size <= lexicon.maxBundledSize, !isReadableAsRomaji(word) {
                // ローマ字として読める語（made・name）は、同梱の一覧にしか無ければ日本語にする。
                // 2 文字の語は略語（ff・ll・sf）ばかりで、日本語の間の英字を細切れに取ってしまうので使わない
                candidates.append(EnglishCandidate(range: range, cost: weights.bundledWord + Float(size - 35) / 50))
            }
        }
    }
    // どの一覧にも無い語（jev・filmroll）。単独ではローマ字として読めず、前がかなの区切り（母音・n・入力の始まり）、
    // 後ろが入力の終わり・助詞のローマ字・子音のあとに子音が続くところ（gltf + ふぁいる。助詞が無くても終われる）。
    // 一覧にある語の途中から始まるもの（google の gle・desk の sk）と、一覧にある語の頭だけのもの（claude の clau）は、
    // 語の断片なので作らない（境目をまたいで重なるだけの語は気にしない）
    if lexicon.unknownWords && !ownWordsOnly {
        let knownRanges = candidates.map(\.range)
        let known = Set(knownRanges)
        let isVowelOrY = { (c: Character) in "aiueoy".contains(c.lowercased()) }
        for start in characters.indices where isLetter(characters[start]) {
            if start > 0, isLetter(characters[start - 1]), !"aiueon".contains(characters[start - 1]) {
                continue
            }
            var end = start + 1
            while end < characters.count, end - start < maxEnglishWordLength, isLetter(characters[end]) {
                end += 1
                let range = start..<end
                let atEnd = end == characters.count || !isLetter(characters[end])
                let beforeParticle = particleRomaji.contains { particle in
                    end + particle.count <= characters.count
                        && characters[end..<(end + particle.count)].elementsEqual(particle)
                }
                // ch・sh・th・ts の間では切らない（次の母音と組んでひとつのかなになる）
                var betweenConsonants = false
                if !atEnd {
                    let last = Character(characters[end - 1].lowercased())
                    let next = Character(characters[end].lowercased())
                    betweenConsonants = !"aiueon".contains(last) && !isVowelOrY(next) && next != "h" && !(last == "t" && next == "s")
                }
                guard atEnd || beforeParticle || betweenConsonants, !known.contains(range) else { continue }
                guard !knownRanges.contains(where: { known in
                    (known.lowerBound < range.lowerBound && range.lowerBound < known.upperBound)
                        || (known.lowerBound == range.lowerBound && range.upperBound < known.upperBound)
                }) else { continue }
                let leftover = romajiLeftover(String(characters[range]).lowercased())
                guard leftover > 0 else { continue }
                // かなとして読める英字が多いほど、日本語を飲み込んでいる見込みが高い（jev + とあいもでる）
                candidates.append(EnglishCandidate(range: range, cost: weights.unknownWord + weights.unknownKanaLetter * Float(range.count - leftover)))
            }
        }
    }
    // 大文字だけの並び（略語）。ローマ字にならないので、そのまま英字として扱う。
    // 後ろに小文字が続く大文字 1 文字（Jaccard の J）は語の頭なので、ここでは取らない（UI + an（案）は取る）
    var start = 0
    while start < characters.count {
        guard characters[start].isASCII, characters[start].isUppercase else {
            start += 1
            continue
        }
        var end = start
        while end < characters.count, characters[end].isASCII, characters[end].isUppercase {
            end += 1
        }
        let capitalizedWordHead = end - start == 1 && end < characters.count && characters[end].isASCII && characters[end].isLowercase
        if !capitalizedWordHead, !candidates.contains(where: { $0.range == start..<end }) {
            candidates.append(EnglishCandidate(range: start..<end, cost: weights.uppercaseRun))
        }
        start = end
    }
    return candidates.filter { candidate in !japaneseRanges.contains { $0.overlaps(candidate.range) } }
}

/// 同梱の英単語の一覧（`語<TAB>大きさ` の行）を読む。読めなければ空（判定しない）
func loadEnglishWordList(_ url: URL) -> [String: Int] {
    guard let text = try? String(contentsOf: url, encoding: .utf8) else {
        return [:]
    }
    var words: [String: Int] = [:]
    for line in text.split(whereSeparator: \.isNewline) {
        let fields = line.split(separator: "\t")
        guard fields.count == 2, let size = Int(fields[1]) else { continue }
        words[String(fields[0])] = size
    }
    return words
}
