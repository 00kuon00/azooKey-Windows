// 確定済みの文字列の再変換（Win+/ と ITfFnReconversion::Reconvert）
use anyhow::Result;
use windows::Win32::UI::TextServices::ITfRange;

use crate::tsf::factory::TextServiceFactory;

use super::{
    composition::{Composition, CompositionState},
    ipc_service::{Candidates, IPCService},
};

/// 再変換する文字列の長さの上限（文字数）。サーバの `maxReconversionSurfaceLength` と同じ
pub const MAX_RECONVERSION_CHARS: usize = 64;

/// 選択範囲の文字列を再変換に使えるか。空・長すぎる・改行を含むものは使わない
pub fn reconversion_target(selected: &str) -> Option<&str> {
    if selected.is_empty()
        || selected.chars().count() > MAX_RECONVERSION_CHARS
        || selected.contains(['\r', '\n'])
    {
        return None;
    }
    Some(selected)
}

/// サーバから得た候補で、再変換の状態（候補を選んでいる状態）を作る。候補が無ければ `None`。
/// 元の文字列と同じ候補があればそれを選んでおく（そのまま Enter で確定しても文字列が変わらない）
pub fn reconversion_composition(original: &str, candidates: Candidates) -> Option<Composition> {
    if candidates.texts.is_empty() {
        return None;
    }
    let index = candidates
        .texts
        .iter()
        .position(|text| text == original)
        .unwrap_or(0);

    Some(Composition {
        preview: candidates.texts[index].clone(),
        suffix: candidates.sub_texts.get(index).cloned().unwrap_or_default(),
        // 読みを入力したのと同じ扱いにする（F6〜F10・確定後の残りの計算が raw_input を使う）
        raw_input: candidates.hiragana.clone(),
        raw_hiragana: candidates.hiragana.clone(),
        corresponding_count: candidates
            .corresponding_count
            .get(index)
            .cloned()
            .unwrap_or(0),
        selection_index: index as i32,
        candidates,
        converted_by_function_key: false,
        reconversion_original: Some(original.to_string()),
        state: CompositionState::Previewing,
        tip_composition: None,
    })
}

impl TextServiceFactory {
    /// `range`（`None` なら選択範囲）の文字列を再変換の状態にする。
    /// 文字列が無い・読みが推定できないときは文書に触らず `None` を返す
    pub fn begin_reconversion(
        &self,
        ipc_service: &mut IPCService,
        range: Option<&ITfRange>,
    ) -> Result<Option<Composition>> {
        let Some(selected) = self.reconversion_text(range)? else {
            return Ok(None);
        };
        let Some(original) = reconversion_target(&selected) else {
            return Ok(None);
        };

        let candidates = ipc_service.start_reconversion(original.to_string())?;
        let Some(composition) = reconversion_composition(original, candidates) else {
            tracing::debug!("No reading for reconversion");
            ipc_service.clear_text()?;
            return Ok(None);
        };

        // 読みを調べている間に文書が変わっていたら始めない
        if !self.start_composition_on_reconversion_range(range, original)? {
            ipc_service.clear_text()?;
            return Ok(None);
        }

        self.set_text(&composition.preview, &composition.suffix)?;
        self.update_pos()?;
        ipc_service.show_window()?;
        ipc_service.set_candidates(composition.candidates.texts.clone())?;
        ipc_service.set_selection(composition.selection_index)?;

        Ok(Some(composition))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidates(texts: &[&str], hiragana: &str) -> Candidates {
        Candidates {
            texts: texts.iter().map(|text| text.to_string()).collect(),
            sub_texts: texts.iter().map(|_| String::new()).collect(),
            hiragana: hiragana.to_string(),
            corresponding_count: texts
                .iter()
                .map(|_| hiragana.chars().count() as i32)
                .collect(),
        }
    }

    #[test]
    fn selected_text_becomes_reconversion_state() {
        let composition =
            reconversion_composition("漢字", candidates(&["感じ", "漢字", "幹事"], "かんじ"))
                .expect("reconversion state");

        assert_eq!(composition.state, CompositionState::Previewing);
        assert_eq!(composition.reconversion_original.as_deref(), Some("漢字"));
        // 元の文字列の候補を選んでいる
        assert_eq!(composition.selection_index, 1);
        assert_eq!(composition.preview, "漢字");
        assert_eq!(composition.raw_input, "かんじ");
        assert_eq!(composition.raw_hiragana, "かんじ");
        assert_eq!(composition.corresponding_count, 3);
        assert_eq!(composition.candidates.texts, vec!["感じ", "漢字", "幹事"]);
        assert!(!composition.converted_by_function_key);
    }

    #[test]
    fn first_candidate_is_selected_when_original_is_not_a_candidate() {
        let composition = reconversion_composition("官寺", candidates(&["感じ", "漢字"], "かんじ"))
            .expect("reconversion state");

        assert_eq!(composition.selection_index, 0);
        assert_eq!(composition.preview, "感じ");
        assert_eq!(composition.reconversion_original.as_deref(), Some("官寺"));
    }

    #[test]
    fn no_candidates_means_no_reconversion() {
        assert!(reconversion_composition("漢字", candidates(&[], "")).is_none());
    }

    #[test]
    fn empty_or_unsuitable_selection_is_not_reconverted() {
        assert_eq!(reconversion_target(""), None);
        assert_eq!(reconversion_target("一行目\r\n二行目"), None);
        assert_eq!(
            reconversion_target(&"漢".repeat(MAX_RECONVERSION_CHARS + 1)),
            None
        );
        assert_eq!(reconversion_target("漢字"), Some("漢字"));
    }
}
