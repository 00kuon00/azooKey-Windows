/// 候補ウィンドウの帯（#assist）で選んでいるもの。帯には英字の候補（#english-row・日英の自動判定 #26）・
/// 予測（#prediction-row）・もしかして（#typo-row）が並び、Tab で上から順に進み、最後の次は変換候補の一覧に戻る
/// （Shift+Tab は逆順）
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum AssistSelection {
    // 帯を選んでいない（変換候補の一覧を選んでいる）
    #[default]
    None,
    English(usize),
    Prediction(usize),
    Typo(usize),
}

/// 英字の候補の行を、帯のどこに置くか（覚えた語を含むときは、どちらでも先頭）
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EnglishRowPosition {
    // いつも帯の先頭
    First,
    // 予測の次（予測 → 英字 → もしかして）
    AfterPredictions,
}

/// 英字の候補の行の置き場。帯の試作（https://claude.ai/artifact/QWE6XmFfQaRpEjYhfVGBJB）で選んだもの
pub const ENGLISH_ROW_POSITION: EnglishRowPosition = EnglishRowPosition::AfterPredictions;

/// 帯に並ぶ行の数
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct AssistRows {
    pub english: usize,
    pub predictions: usize,
    pub typos: usize,
    // 英字の候補が覚えた語（F10 で直した語・帯で選んだ語）を含む
    pub english_learned: bool,
}

impl AssistRows {
    /// 英字の候補の行が帯の先頭か
    pub fn english_first(&self, position: EnglishRowPosition) -> bool {
        self.english_learned || position == EnglishRowPosition::First
    }

    /// 帯の上から順に並べた行
    pub fn order(&self, position: EnglishRowPosition) -> Vec<AssistSelection> {
        let english = (0..self.english).map(AssistSelection::English);
        let predictions = (0..self.predictions).map(AssistSelection::Prediction);
        let typos = (0..self.typos).map(AssistSelection::Typo);
        if self.english_first(position) {
            english.chain(predictions).chain(typos).collect()
        } else {
            predictions.chain(english).chain(typos).collect()
        }
    }

    pub fn is_empty(&self) -> bool {
        self.english + self.predictions + self.typos == 0
    }
}

impl AssistSelection {
    /// Tab（`forward`）・Shift+Tab で選ぶ次のもの。帯が空なら None
    pub fn next(
        self,
        rows: &AssistRows,
        position: EnglishRowPosition,
        forward: bool,
    ) -> AssistSelection {
        let order = rows.order(position);
        let total = order.len();
        if total == 0 {
            return AssistSelection::None;
        }
        // 帯の中の通し番号。None（と、帯から消えたもの）は -1 と total の両方にあたる
        let current = order.iter().position(|selection| *selection == self);
        let next = match (current, forward) {
            (None, true) => Some(0),
            (None, false) => Some(total - 1),
            (Some(i), true) if i + 1 < total => Some(i + 1),
            (Some(i), false) if i > 0 => Some(i - 1),
            _ => None,
        };
        next.map(|i| order[i]).unwrap_or(AssistSelection::None)
    }
}

#[cfg(test)]
mod tests {
    use super::AssistSelection::*;
    use super::*;

    fn rows(english: usize, predictions: usize, typos: usize, english_learned: bool) -> AssistRows {
        AssistRows {
            english,
            predictions,
            typos,
            english_learned,
        }
    }

    fn walk(
        rows: &AssistRows,
        position: EnglishRowPosition,
        forward: bool,
        steps: usize,
    ) -> Vec<AssistSelection> {
        let mut selection = None;
        let mut walked = vec![];
        for _ in 0..steps {
            selection = selection.next(rows, position, forward);
            walked.push(selection);
        }
        walked
    }

    // Tab で 予測 → もしかして と進み、最後の次は一覧に戻る
    #[test]
    fn tab_walks_predictions_then_typos() {
        let rows = rows(0, 2, 2, false);
        assert_eq!(
            walk(&rows, EnglishRowPosition::AfterPredictions, true, 5),
            vec![Prediction(0), Prediction(1), Typo(0), Typo(1), None]
        );
    }

    // Shift+Tab は逆順で、一覧からは帯の最後へ
    #[test]
    fn shift_tab_walks_backwards() {
        let rows = rows(0, 2, 1, false);
        let position = EnglishRowPosition::AfterPredictions;
        assert_eq!(None.next(&rows, position, false), Typo(0));
        assert_eq!(Typo(0).next(&rows, position, false), Prediction(1));
        assert_eq!(Prediction(0).next(&rows, position, false), None);
    }

    // 帯が空なら選べない（Tab は今までどおり次の変換候補になる）
    #[test]
    fn empty_assist_selects_nothing() {
        let rows = rows(0, 0, 0, false);
        assert!(rows.is_empty());
        assert_eq!(None.next(&rows, EnglishRowPosition::First, true), None);
        assert_eq!(None.next(&rows, EnglishRowPosition::First, false), None);
    }

    // 予測だけ・もしかしてだけでも回る
    #[test]
    fn only_one_kind() {
        let position = EnglishRowPosition::AfterPredictions;
        assert_eq!(None.next(&rows(0, 0, 2, false), position, true), Typo(0));
        assert_eq!(Typo(1).next(&rows(0, 0, 2, false), position, true), None);
        assert_eq!(None.next(&rows(0, 3, 0, false), position, false), Prediction(2));
        assert_eq!(None.next(&rows(1, 0, 0, false), position, true), English(0));
    }

    // 英字の候補は 予測 → 英字 → もしかして（予測の次に置くとき）
    #[test]
    fn english_after_predictions() {
        let rows = rows(1, 1, 1, false);
        assert_eq!(
            walk(&rows, EnglishRowPosition::AfterPredictions, true, 4),
            vec![Prediction(0), English(0), Typo(0), None]
        );
    }

    // 先頭に置くときと、覚えた語を含むときは英字が先頭
    #[test]
    fn english_first_when_placed_first_or_learned() {
        assert_eq!(
            walk(&rows(1, 1, 1, false), EnglishRowPosition::First, true, 4),
            vec![English(0), Prediction(0), Typo(0), None]
        );
        assert_eq!(
            walk(&rows(1, 1, 0, true), EnglishRowPosition::AfterPredictions, true, 3),
            vec![English(0), Prediction(0), None]
        );
    }

    // 選んでいた行が帯から消えたら、次の Tab は先頭から
    #[test]
    fn vanished_selection_restarts() {
        let rows = rows(0, 1, 0, false);
        assert_eq!(
            English(0).next(&rows, EnglishRowPosition::AfterPredictions, true),
            Prediction(0)
        );
    }
}
