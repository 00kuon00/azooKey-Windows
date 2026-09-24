/// 文節の区切りを `delta` 文字動かしたときの、最初の文節の読みの文字数を返す（Shift+←→）。
///
/// `hiragana` は読み全体、`suffix` は表示中の候補を確定したあとに残る読み（最初の文節より後ろ）。
/// 最初の文節は 1 文字より短くも、読み全体より長くもならない。
pub fn moved_segment_len(hiragana: &str, suffix: &str, delta: i32) -> usize {
    let total = hiragana.chars().count();
    let current = total.saturating_sub(suffix.chars().count());
    let moved = current as i64 + delta as i64;
    moved.clamp(1, total.max(1) as i64) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 読みを、最初の文節（先頭 `segment_len` 文字）とそれより後ろに分ける
    fn split_reading(hiragana: &str, segment_len: usize) -> (String, String) {
        let segment = hiragana.chars().take(segment_len).collect();
        let rest = hiragana.chars().skip(segment_len).collect();
        (segment, rest)
    }

    // 「きょうは|いいてんき」と区切られているところから区切りを動かす
    const HIRAGANA: &str = "きょうはいいてんき";
    const SUFFIX: &str = "いいてんき";

    fn moved(delta: i32) -> (String, String) {
        split_reading(HIRAGANA, moved_segment_len(HIRAGANA, SUFFIX, delta))
    }

    #[test]
    fn shrink_moves_last_reading_to_next_segment() {
        assert_eq!(
            moved(-1),
            ("きょう".to_string(), "はいいてんき".to_string())
        );
    }

    #[test]
    fn expand_takes_first_reading_of_next_segment() {
        assert_eq!(moved(1), ("きょうはい".to_string(), "いてんき".to_string()));
    }

    #[test]
    fn segment_is_at_least_one_character() {
        assert_eq!(moved_segment_len(HIRAGANA, "ょうはいいてんき", -1), 1);
        assert_eq!(moved_segment_len(HIRAGANA, SUFFIX, -10), 1);
    }

    #[test]
    fn segment_is_at_most_whole_reading() {
        assert_eq!(moved_segment_len(HIRAGANA, "", 1), 9);
        assert_eq!(moved_segment_len(HIRAGANA, SUFFIX, 10), 9);
    }

    #[test]
    fn counts_characters_not_bytes() {
        // 読みの途中のローマ字（「てn」の n）も 1 文字として数える
        assert_eq!(moved_segment_len("てn", "n", 1), 2);
    }
}
