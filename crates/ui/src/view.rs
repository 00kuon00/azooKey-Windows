//! 候補ウィンドウに渡す表示内容を組み立てる（読みの行・帯・変換候補の一覧）。
//! 描くのは JS（candidate.rs）で、ここでは文字の区切りと印だけを決める

use serde::Serialize;

/// 読みの一部分と、その印
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Span {
    pub text: String,
    /// "" = 印なし / "typo" = 打ち間違えた字 / "fix" = 直した字 / "segment" = 区切った最初の文節
    pub mark: &'static str,
}

/// 帯の予測（#prediction-row）
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Prediction {
    pub text: String,
    /// 予測が読みで始まるなら、読みの後ろに薄く続ける部分
    pub ghost: Option<String>,
}

/// 帯の「もしかして」（#typo-row）
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Typo {
    pub text: String,
    /// 直した読み。直した字に "fix" の印
    pub reading: Vec<Span>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CandidateView {
    pub candidates: Vec<String>,
    /// 読みの行（#reading）。空なら出さない
    pub reading: Vec<Span>,
    pub predictions: Vec<Prediction>,
    pub typos: Vec<Typo>,
}

impl CandidateView {
    /// `typos` は（直した読みを変換した 1 位, 直した読み）
    pub fn new(
        candidates: Vec<String>,
        hiragana: &str,
        segment_length: usize,
        predictions: Vec<String>,
        typos: Vec<(String, String)>,
    ) -> Self {
        let reading = if segment_length > 0 {
            segment_spans(hiragana, segment_length)
        } else if let Some((_, corrected)) = typos.first() {
            // 打った読みのうち、1 つ目の「もしかして」と違う字に印を付ける
            let (typed, _) = diff_marks(hiragana, corrected);
            marked_spans(hiragana, &typed, "typo")
        } else {
            marked_spans(hiragana, &[], "")
        };
        let predictions = predictions
            .into_iter()
            .map(|text| Prediction {
                ghost: ghost(&text, hiragana),
                text,
            })
            .collect();
        let typos = typos
            .into_iter()
            .map(|(text, corrected)| {
                let (_, fixed) = diff_marks(hiragana, &corrected);
                Typo {
                    text,
                    reading: marked_spans(&corrected, &fixed, "fix"),
                }
            })
            .collect();
        Self {
            candidates,
            reading,
            predictions,
            typos,
        }
    }

    /// 幅を決める文字数。変換候補・予測・もしかして（語と直した読みを並べる）の長いほう。
    /// 読みは入れない（長い読みは先頭を省いて出す）
    pub fn max_len(&self) -> u32 {
        let candidates = self.candidates.iter().map(|s| s.chars().count());
        let predictions = self.predictions.iter().map(|p| p.text.chars().count());
        let typos = self.typos.iter().map(|t| {
            let reading: usize = t.reading.iter().map(|s| s.text.chars().count()).sum();
            t.text.chars().count() + reading
        });
        candidates
            .chain(predictions)
            .chain(typos)
            .max()
            .unwrap_or(0) as u32
    }
}

/// 予測が読みで始まるなら、読みより後ろの部分
pub fn ghost(prediction: &str, hiragana: &str) -> Option<String> {
    if hiragana.is_empty() {
        return None;
    }
    prediction
        .strip_prefix(hiragana)
        .filter(|rest| !rest.is_empty())
        .map(str::to_string)
}

/// 2 つの読みを文字の最長共通部分列で合わせ、共通でない字に true を立てる（それぞれの読みの文字ごと）
pub fn diff_marks(typed: &str, corrected: &str) -> (Vec<bool>, Vec<bool>) {
    let a: Vec<char> = typed.chars().collect();
    let b: Vec<char> = corrected.chars().collect();
    let (n, m) = (a.len(), b.len());
    let mut dp = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i][j] = if a[i] == b[j] {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }
    let mut marked_a = vec![true; n];
    let mut marked_b = vec![true; m];
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if a[i] == b[j] {
            marked_a[i] = false;
            marked_b[j] = false;
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    (marked_a, marked_b)
}

/// 文字ごとの印（true の字に `mark`）を、同じ印の続く部分ごとにまとめる
fn marked_spans(text: &str, marks: &[bool], mark: &'static str) -> Vec<Span> {
    let mut spans: Vec<Span> = Vec::new();
    for (i, c) in text.chars().enumerate() {
        let current = if marks.get(i).copied().unwrap_or(false) {
            mark
        } else {
            ""
        };
        match spans.last_mut() {
            Some(last) if last.mark == current => last.text.push(c),
            _ => spans.push(Span {
                text: c.to_string(),
                mark: current,
            }),
        }
    }
    spans
}

/// 最初の文節（先頭 `segment_length` 文字）と残り
fn segment_spans(hiragana: &str, segment_length: usize) -> Vec<Span> {
    let segment: String = hiragana.chars().take(segment_length).collect();
    let rest: String = hiragana.chars().skip(segment_length).collect();
    let mut spans = vec![Span {
        text: segment,
        mark: "segment",
    }];
    if !rest.is_empty() {
        spans.push(Span {
            text: rest,
            mark: "",
        });
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(text: &str, mark: &'static str) -> Span {
        Span {
            text: text.to_string(),
            mark,
        }
    }

    // 読みはそのまま 1 つの部分として出る
    #[test]
    fn reading_without_typo_is_plain() {
        let view = CandidateView::new(vec!["今日".into()], "きょう", 0, vec![], vec![]);
        assert_eq!(view.reading, vec![span("きょう", "")]);
    }

    // 隣のキーの押し間違い: 打った読みの違う字に typo、直した読みの違う字に fix
    #[test]
    fn substituted_character_is_marked() {
        let view = CandidateView::new(
            vec!["蟻が問い".into()],
            "ありがとい",
            0,
            vec![],
            vec![("ありがとう".into(), "ありがとう".into())],
        );
        assert_eq!(view.reading, vec![span("ありがと", ""), span("い", "typo")]);
        assert_eq!(
            view.typos[0].reading,
            vec![span("ありがと", ""), span("う", "fix")]
        );
    }

    // 文字の抜け: 打った読みに印は無く、直した読みの足した字に fix
    #[test]
    fn dropped_character_is_marked_in_correction() {
        let (typed, fixed) = diff_marks("おつかれさあ", "おつかれさま");
        assert_eq!(typed, vec![false, false, false, false, false, true]);
        assert_eq!(fixed, vec![false, false, false, false, false, true]);
        let (typed, fixed) = diff_marks("わたは", "わたしは");
        assert!(typed.iter().all(|m| !m));
        assert_eq!(fixed, vec![false, false, true, false]);
    }

    // Shift+←→ で区切ったときは、最初の文節と残りに分ける（もしかしてより優先）
    #[test]
    fn segment_is_split() {
        let view = CandidateView::new(
            vec!["今日は".into()],
            "きょうはいいてんき",
            4,
            vec![],
            vec![("x".into(), "きょうわいいてんき".into())],
        );
        assert_eq!(
            view.reading,
            vec![span("きょうは", "segment"), span("いいてんき", "")]
        );
    }

    // 予測が読みで始まれば続きを薄く出し、始まらなければ出さない
    #[test]
    fn ghost_only_when_prediction_starts_with_reading() {
        assert_eq!(
            ghost("おはようございます", "おはよ"),
            Some("うございます".into())
        );
        assert_eq!(ghost("お早うございます", "おはよ"), None);
        assert_eq!(ghost("おはよ", "おはよ"), None);
        assert_eq!(ghost("おはよう", ""), None);
    }

    // 幅は予測・もしかしても含めて決める。読みは含めない
    #[test]
    fn max_len_includes_predictions_and_typos() {
        let view = CandidateView::new(
            vec!["お疲れさあ".into()],
            "おつかれさあああああああああ",
            0,
            vec!["お疲れさまでした".into()],
            vec![],
        );
        assert_eq!(view.max_len(), 8);
        let view = CandidateView::new(
            vec!["蟻".into()],
            "ありがとい",
            0,
            vec![],
            vec![("有難う".into(), "ありがとう".into())],
        );
        assert_eq!(view.max_len(), 8);
    }
}
