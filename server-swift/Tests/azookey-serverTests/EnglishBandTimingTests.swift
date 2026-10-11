import Testing
import Foundation
import KanaKanjiConverterModule
@testable import azookey_server

// 日英の自動判定（#26）の完了条件 ③: 1 打鍵の時間の中央値の増え方（+5 ms 以内）を測る。合否は付けず、結果を出力に書く。
// Zenzai の要る計測と同じく、AZOOKEY_ZENZ_GGUF を渡したときだけ走る。GPU の層の数は AZOOKEY_GPU_LAYERS、
// バックエンドは PATH の llama.dll で決まる（#14・#22 と同じ測り方）:
//   AZOOKEY_ZENZ_GGUF=<zenz.gguf> AZOOKEY_GPU_LAYERS=999 （PATH の先頭に llama_vulkan）swift test --filter measureEnglishBandTime
extension GlobalStateTests.ZenzaiSessionTests {
    @Test func measureEnglishBandTime() throws {
        let root = URL(filePath: #filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        systemDictionaryURL = dictionaryURL
        englishConverter = nil
        englishWordsURL = root.appendingPathComponent("EnglishWords").appendingPathComponent("english_words.txt")
        englishVocabularyURL = workURL.appendingPathComponent("english_vocabulary.json")
        englishLearnedURL = workURL.appendingPathComponent("english_learned.json")
        try JSONEncoder().encode(EnglishVocabularyFile(words: ["opencode": 207, "claude": 1285, "github": 297, "issue": 199, "comfyui": 653, "pr": 50]))
            .write(to: englishVocabularyURL)
        lastEnglishFiles = nil
        reloadEnglishLexicon()

        let japanese = ["kyouhaiitenkidesune", "kanjihenkanwosuru", "asitanoyoteiwokakunin", "zenzaiwotukau", "nihongonyuuryoku"]
        let mixed = ["kyouhagoogledekensaku", "konnshuuhaopencodedePRwodasu", "claudenoyouyakuwotanomitai", "githubnoissuewomiru", "comfyuinowaーkufuroーwotsukau"]

        // 読み込みと最初の変換を済ませておく（帯の変換器も作る）
        englishEnabled = true
        commitFirstAndClear(type("junbi").candidates)
        _ = type("googlewo")
        free(get_english_candidates())
        clear_text()

        // サーバと同じく 1 文字ごとに GetComposedText と GetEnglishCandidates を呼び、合わせた時間
        func keystrokes(_ roman: String) -> [Double] {
            var times: [Double] = []
            for character in roman {
                let cursor = UnsafeMutablePointer<Int>.allocate(capacity: 1)
                free(append_text(input: String(character), cursorPtr: cursor))
                cursor.deallocate()
                let start = ContinuousClock.now
                _ = candidates()
                free(get_english_candidates())
                let elapsed = ContinuousClock.now - start
                times.append(Double(elapsed.components.seconds) + Double(elapsed.components.attoseconds) / 1e18)
            }
            clear_text()
            return times
        }

        var times: [String: [Double]] = [:]
        // ON と OFF を交互に、同じ文を 3 周
        for round in 0..<3 {
            for sentence in japanese + mixed {
                for enabled in round % 2 == 0 ? [true, false] : [false, true] {
                    englishEnabled = enabled
                    let kind = (japanese.contains(sentence) ? "japanese" : "mixed") + (enabled ? " on" : " off")
                    times[kind, default: []] += keystrokes(sentence)
                }
            }
        }
        englishEnabled = true
        func median(_ values: [Double]) -> Double { values.sorted()[values.count / 2] * 1000 }
        let layers = ProcessInfo.processInfo.environment["AZOOKEY_GPU_LAYERS"] ?? "0"
        for kind in ["japanese", "mixed"] {
            let on = times["\(kind) on"]!, off = times["\(kind) off"]!
            print(String(format: "ENGLISH-BENCH [gpu layers %@] %@ keystroke median: off %.1f ms / on %.1f ms / +%.1f ms (n=%d) / max on %.1f ms",
                         layers, kind, median(off), median(on), median(on) - median(off), on.count, on.max()! * 1000))
        }
    }
}
