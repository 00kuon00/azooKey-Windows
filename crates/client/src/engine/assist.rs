/// 候補ウィンドウの帯（#assist）で選んでいるもの。帯には予測（#prediction-row）ともしかして（#typo-row）が並び、
/// Tab で 予測 → もしかして の順に進み、最後の次は変換候補の一覧に戻る（Shift+Tab は逆順）
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum AssistSelection {
    // 帯を選んでいない（変換候補の一覧を選んでいる）
    #[default]
    None,
    Prediction(usize),
    Typo(usize),
}

impl AssistSelection {
    /// Tab（`forward`）・Shift+Tab で選ぶ次のもの。帯が空なら None
    pub fn next(self, predictions: usize, typos: usize, forward: bool) -> AssistSelection {
        let total = predictions + typos;
        if total == 0 {
            return AssistSelection::None;
        }
        // 帯の中の通し番号。None は -1 と total の両方にあたる
        let current = match self {
            AssistSelection::None => None,
            AssistSelection::Prediction(i) => Some(i),
            AssistSelection::Typo(i) => Some(predictions + i),
        };
        let next = match (current, forward) {
            (None, true) => Some(0),
            (None, false) => Some(total - 1),
            (Some(i), true) if i + 1 < total => Some(i + 1),
            (Some(i), false) if i > 0 => Some(i - 1),
            _ => None,
        };
        match next {
            None => AssistSelection::None,
            Some(i) if i < predictions => AssistSelection::Prediction(i),
            Some(i) => AssistSelection::Typo(i - predictions),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use AssistSelection::*;

    // Tab で 予測 → もしかして と進み、最後の次は一覧に戻る
    #[test]
    fn tab_walks_predictions_then_typos() {
        let mut selection = None;
        let mut walked = vec![];
        for _ in 0..5 {
            selection = selection.next(2, 2, true);
            walked.push(selection);
        }
        assert_eq!(
            walked,
            vec![Prediction(0), Prediction(1), Typo(0), Typo(1), None]
        );
    }

    // Shift+Tab は逆順で、一覧からは帯の最後へ
    #[test]
    fn shift_tab_walks_backwards() {
        assert_eq!(None.next(2, 1, false), Typo(0));
        assert_eq!(Typo(0).next(2, 1, false), Prediction(1));
        assert_eq!(Prediction(0).next(2, 1, false), None);
    }

    // 帯が空なら選べない（Tab は今までどおり次の変換候補になる）
    #[test]
    fn empty_assist_selects_nothing() {
        assert_eq!(None.next(0, 0, true), None);
        assert_eq!(None.next(0, 0, false), None);
    }

    // 予測だけ・もしかしてだけでも回る
    #[test]
    fn only_one_kind() {
        assert_eq!(None.next(0, 2, true), Typo(0));
        assert_eq!(Typo(1).next(0, 2, true), None);
        assert_eq!(None.next(3, 0, false), Prediction(2));
    }
}
