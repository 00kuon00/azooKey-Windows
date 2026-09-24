import Testing
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
