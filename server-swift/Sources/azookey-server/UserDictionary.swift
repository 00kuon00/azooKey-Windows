import KanaKanjiConverterModule
import Foundation
import SwiftUtils

/// ユーザー辞書の 1 語。%APPDATA%\Azookey\user_dictionary.json の配列の要素（設定画面が書く）
struct UserDictionaryEntry: Codable, Equatable, Sendable {
    var reading: String
    var word: String
}

/// `charID.chid` から文字 → ID の対応を作る。
/// `DictionaryBuilder.exportDictionary(charIDFileURL:)` と同じ作り方で、登録できるかの判定と辞書の作成に同じ対応を使う
func loadCharIDMap(dictionaryURL: URL) throws -> [Character: UInt8] {
    let url = dictionaryURL.appendingPathComponent("louds").appendingPathComponent("charID.chid")
    let string = try String(contentsOf: url, encoding: .utf8)
    return Dictionary(uniqueKeysWithValues: string.enumerated().map { ($0.element, UInt8($0.offset)) })
}

/// ユーザー辞書（`user.louds` / `user.loudschars2` / `user0.loudstxt3`）を `outputURL` に作り直す。
/// 返り値は登録できなかった語（読みが空、または `charID.chid` に無い文字を含む）。
/// `DictionaryBuilder` はこうした語を黙って捨てるので、先にここで分けて返す
func buildUserDictionary(
    entries: [UserDictionaryEntry],
    dictionaryURL: URL,
    outputURL: URL
) throws -> [UserDictionaryEntry] {
    let char2UInt8 = try loadCharIDMap(dictionaryURL: dictionaryURL)

    var elements: [DicdataElement] = []
    var unregistered: [UserDictionaryEntry] = []
    for entry in entries {
        let ruby = entry.reading.toKatakana()
        guard !ruby.isEmpty, !entry.word.isEmpty, ruby.allSatisfy({ char2UInt8[$0] != nil }) else {
            unregistered.append(entry)
            continue
        }
        // 品詞と重みは azooKey macOS のユーザー辞書と同じ（固有名詞・一般・-5）
        elements.append(DicdataElement(
            word: entry.word,
            ruby: ruby,
            cid: CIDData.固有名詞.cid,
            mid: MIDData.一般.mid,
            value: -5
        ))
    }

    // 作業用のフォルダに作ってから差し替える（途中で落ちても壊れた辞書を残さない）
    let fileManager = FileManager.default
    let parentURL = outputURL.deletingLastPathComponent()
    let workURL = parentURL.appendingPathComponent(outputURL.lastPathComponent + ".building", isDirectory: true)
    try fileManager.createDirectory(at: parentURL, withIntermediateDirectories: true)
    if fileManager.fileExists(atPath: workURL.path(percentEncoded: false)) {
        try fileManager.removeItem(at: workURL)
    }
    try fileManager.createDirectory(at: workURL, withIntermediateDirectories: true)
    if !elements.isEmpty {
        try DictionaryBuilder.exportDictionary(
            entries: elements,
            to: workURL,
            baseName: "user",
            shardByFirstCharacter: false,
            char2UInt8: char2UInt8
        )
    }
    // Windows では既存のフォルダへ上書きで移動できないので、先に消す
    if fileManager.fileExists(atPath: outputURL.path(percentEncoded: false)) {
        try fileManager.removeItem(at: outputURL)
    }
    try fileManager.moveItem(at: workURL, to: outputURL)
    return unregistered
}
