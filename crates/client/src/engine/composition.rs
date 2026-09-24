use std::cmp::{max, min};

use crate::{
    engine::user_action::UserAction,
    extension::VKeyExt as _,
    tsf::factory::{TextServiceFactory, TextServiceFactory_Impl},
};

use super::{
    client_action::{ClientAction, SetSelectionType, SetTextType},
    full_width::{to_fullwidth, to_halfwidth},
    input_mode::InputMode,
    ipc_service::Candidates,
    segment::moved_segment_len,
    state::IMEState,
    text_util::{to_half_katakana, to_katakana},
    user_action::{Function, Navigation},
};
use windows::Win32::{
    Foundation::WPARAM,
    UI::{
        Input::KeyboardAndMouse::VK_CONTROL,
        TextServices::{ITfComposition, ITfCompositionSink_Impl, ITfContext},
    },
};

use anyhow::{Context, Result};

#[derive(Default, Clone, PartialEq, Debug)]
pub enum CompositionState {
    #[default]
    None,
    Composing,
    Previewing,
    Selecting,
}

#[derive(Default, Clone, Debug)]
pub struct Composition {
    pub preview: String, // text to be previewed
    pub suffix: String,  // text to be appended after preview
    pub raw_input: String,
    pub raw_hiragana: String,

    pub corresponding_count: i32, // corresponding count of the preview

    pub selection_index: i32,
    pub candidates: Candidates,
    // F6〜F10 でひらがな・カタカナ・英数に置き換えて表示している（候補ではないので学習しない）
    pub converted_by_function_key: bool,
    // 再変換中なら元の文字列（Escape でこれに戻す）。読みを打ち直したら None
    pub reconversion_original: Option<String>,

    pub state: CompositionState,
    pub tip_composition: Option<ITfComposition>,
}

impl ITfCompositionSink_Impl for TextServiceFactory_Impl {
    #[macros::anyhow]
    fn OnCompositionTerminated(
        &self,
        _ecwrite: u32,
        _pcomposition: Option<&ITfComposition>,
    ) -> Result<()> {
        // if user clicked outside the composition, the composition will be terminated
        tracing::debug!("OnCompositionTerminated");

        let actions = vec![ClientAction::EndComposition];
        self.handle_action(&actions, CompositionState::None)?;

        Ok(())
    }
}

impl TextServiceFactory {
    #[tracing::instrument]
    pub fn process_key(
        &self,
        context: Option<&ITfContext>,
        wparam: WPARAM,
    ) -> Result<Option<(Vec<ClientAction>, CompositionState)>> {
        if context.is_none() {
            return Ok(None);
        };

        // check shortcut keys (Ctrl+Space・Ctrl+Delete だけは IME で受ける)
        let control_action = if VK_CONTROL.is_pressed() {
            match UserAction::from_control_key(wparam.0) {
                Some(action) => Some(action),
                None => return Ok(None),
            }
        } else {
            None
        };

        #[allow(clippy::let_and_return)]
        let (composition, mode) = {
            let text_service = self.borrow()?;
            let composition = text_service.borrow_composition()?.clone();
            let mode = IMEState::get()?.input_mode.clone();
            (composition, mode)
        };

        let action = match control_action {
            Some(action) => action,
            None => UserAction::try_from(wparam.0)?,
        };

        let (transition, actions) = match composition.state {
            CompositionState::None => match action {
                UserAction::Input(char) if mode == InputMode::Kana => (
                    CompositionState::Composing,
                    vec![
                        ClientAction::StartComposition,
                        ClientAction::AppendText(char.to_string()),
                    ],
                ),
                UserAction::Number(number) if mode == InputMode::Kana => (
                    CompositionState::Composing,
                    vec![
                        ClientAction::StartComposition,
                        ClientAction::AppendText(number.to_string()),
                    ],
                ),
                UserAction::ToggleInputMode => (
                    CompositionState::None,
                    vec![match mode {
                        InputMode::Kana => ClientAction::SetIMEMode(InputMode::Latin),
                        InputMode::Latin => ClientAction::SetIMEMode(InputMode::Kana),
                    }],
                ),
                // 選択範囲が無いときも Win+/ は受け取って何もしない
                UserAction::Reconvert => (
                    CompositionState::None,
                    vec![ClientAction::StartReconversion(None)],
                ),
                _ => {
                    return Ok(None);
                }
            },
            CompositionState::Composing => match action {
                UserAction::Input(char) => (
                    CompositionState::Composing,
                    vec![ClientAction::AppendText(char.to_string())],
                ),
                UserAction::Number(number) => (
                    CompositionState::Composing,
                    vec![ClientAction::AppendText(number.to_string())],
                ),
                UserAction::Backspace => {
                    if composition.preview.chars().count() == 1 {
                        (
                            CompositionState::None,
                            vec![ClientAction::RemoveText, ClientAction::EndComposition],
                        )
                    } else {
                        (CompositionState::Composing, vec![ClientAction::RemoveText])
                    }
                }
                UserAction::Enter => {
                    if composition.suffix.is_empty() {
                        (
                            CompositionState::None,
                            vec![ClientAction::CommitCandidate, ClientAction::EndComposition],
                        )
                    } else {
                        (
                            CompositionState::Composing,
                            vec![
                                ClientAction::CommitCandidate,
                                ClientAction::ShrinkText("".to_string()),
                            ],
                        )
                    }
                }
                UserAction::Escape if composition.reconversion_original.is_some() => (
                    CompositionState::None,
                    vec![
                        ClientAction::RestoreReconversion,
                        ClientAction::EndComposition,
                    ],
                ),
                UserAction::Escape => (
                    CompositionState::None,
                    vec![ClientAction::RemoveText, ClientAction::EndComposition],
                ),
                UserAction::Navigation(direction) => match direction {
                    Navigation::Right => (
                        CompositionState::Composing,
                        vec![ClientAction::MoveCursor(1)],
                    ),
                    Navigation::Left => (
                        CompositionState::Composing,
                        vec![ClientAction::MoveCursor(-1)],
                    ),
                    Navigation::Up => (
                        CompositionState::Previewing,
                        vec![ClientAction::SetSelection(SetSelectionType::Up)],
                    ),
                    Navigation::Down => (
                        CompositionState::Previewing,
                        vec![ClientAction::SetSelection(SetSelectionType::Down)],
                    ),
                },
                UserAction::ToggleInputMode => (
                    CompositionState::None,
                    vec![
                        ClientAction::EndComposition,
                        ClientAction::SetIMEMode(InputMode::Latin),
                    ],
                ),
                UserAction::Space | UserAction::Tab => (
                    CompositionState::Previewing,
                    vec![ClientAction::SetSelection(SetSelectionType::Down)],
                ),
                UserAction::ShrinkSegment => (
                    CompositionState::Previewing,
                    vec![ClientAction::MoveSegmentBoundary(-1)],
                ),
                UserAction::ExpandSegment => (
                    CompositionState::Previewing,
                    vec![ClientAction::MoveSegmentBoundary(1)],
                ),
                UserAction::Function(key) => match key {
                    Function::Six => (
                        CompositionState::Previewing,
                        vec![ClientAction::SetTextWithType(SetTextType::Hiragana)],
                    ),
                    Function::Seven => (
                        CompositionState::Previewing,
                        vec![ClientAction::SetTextWithType(SetTextType::Katakana)],
                    ),
                    Function::Eight => (
                        CompositionState::Previewing,
                        vec![ClientAction::SetTextWithType(SetTextType::HalfKatakana)],
                    ),
                    Function::Nine => (
                        CompositionState::Previewing,
                        vec![ClientAction::SetTextWithType(SetTextType::FullLatin)],
                    ),
                    Function::Ten => (
                        CompositionState::Previewing,
                        vec![ClientAction::SetTextWithType(SetTextType::HalfLatin)],
                    ),
                },
                _ => {
                    return Ok(None);
                }
            },
            CompositionState::Previewing => match action {
                UserAction::Input(char) => (
                    CompositionState::Composing,
                    vec![ClientAction::ShrinkText(char.to_string())],
                ),
                UserAction::Number(number) => (
                    CompositionState::Composing,
                    vec![ClientAction::ShrinkText(number.to_string())],
                ),
                UserAction::Backspace => {
                    if composition.preview.chars().count() == 1 {
                        (
                            CompositionState::None,
                            vec![ClientAction::RemoveText, ClientAction::EndComposition],
                        )
                    } else {
                        (CompositionState::Composing, vec![ClientAction::RemoveText])
                    }
                }
                UserAction::Enter => {
                    if composition.suffix.is_empty() {
                        (
                            CompositionState::None,
                            vec![ClientAction::CommitCandidate, ClientAction::EndComposition],
                        )
                    } else {
                        (
                            CompositionState::Composing,
                            vec![
                                ClientAction::CommitCandidate,
                                ClientAction::ShrinkText("".to_string()),
                            ],
                        )
                    }
                }
                UserAction::Escape if composition.reconversion_original.is_some() => (
                    CompositionState::None,
                    vec![
                        ClientAction::RestoreReconversion,
                        ClientAction::EndComposition,
                    ],
                ),
                UserAction::Escape => (
                    CompositionState::None,
                    vec![ClientAction::RemoveText, ClientAction::EndComposition],
                ),
                UserAction::Navigation(direction) => match direction {
                    Navigation::Right => (
                        CompositionState::Composing,
                        vec![ClientAction::MoveCursor(1)],
                    ),
                    Navigation::Left => (
                        CompositionState::Composing,
                        vec![ClientAction::MoveCursor(-1)],
                    ),
                    Navigation::Up => (
                        CompositionState::Previewing,
                        vec![ClientAction::SetSelection(SetSelectionType::Up)],
                    ),
                    Navigation::Down => (
                        CompositionState::Previewing,
                        vec![ClientAction::SetSelection(SetSelectionType::Down)],
                    ),
                },
                UserAction::ToggleInputMode => (
                    CompositionState::None,
                    vec![
                        ClientAction::EndComposition,
                        ClientAction::SetIMEMode(InputMode::Latin),
                    ],
                ),
                UserAction::Space | UserAction::Tab => (
                    CompositionState::Previewing,
                    vec![ClientAction::SetSelection(SetSelectionType::Down)],
                ),
                UserAction::ShrinkSegment => (
                    CompositionState::Previewing,
                    vec![ClientAction::MoveSegmentBoundary(-1)],
                ),
                UserAction::ExpandSegment => (
                    CompositionState::Previewing,
                    vec![ClientAction::MoveSegmentBoundary(1)],
                ),
                UserAction::Function(key) => match key {
                    Function::Six => (
                        CompositionState::Previewing,
                        vec![ClientAction::SetTextWithType(SetTextType::Hiragana)],
                    ),
                    Function::Seven => (
                        CompositionState::Previewing,
                        vec![ClientAction::SetTextWithType(SetTextType::Katakana)],
                    ),
                    Function::Eight => (
                        CompositionState::Previewing,
                        vec![ClientAction::SetTextWithType(SetTextType::HalfKatakana)],
                    ),
                    Function::Nine => (
                        CompositionState::Previewing,
                        vec![ClientAction::SetTextWithType(SetTextType::FullLatin)],
                    ),
                    Function::Ten => (
                        CompositionState::Previewing,
                        vec![ClientAction::SetTextWithType(SetTextType::HalfLatin)],
                    ),
                },
                UserAction::Forget => (
                    CompositionState::Previewing,
                    vec![ClientAction::ForgetCandidate],
                ),
                _ => {
                    return Ok(None);
                }
            },
            _ => {
                return Ok(None);
            }
        };

        Ok(Some((actions, transition)))
    }

    #[tracing::instrument]
    pub fn handle_key(&self, context: Option<&ITfContext>, wparam: WPARAM) -> Result<bool> {
        if let Some(context) = context {
            self.borrow_mut()?.context = Some(context.clone());
        } else {
            return Ok(false);
        };

        if let Some((actions, transition)) = self.process_key(context, wparam)? {
            self.handle_action(&actions, transition)?;
        } else {
            return Ok(false);
        }

        Ok(true)
    }

    #[tracing::instrument]
    pub fn handle_action(
        &self,
        actions: &[ClientAction],
        transition: CompositionState,
    ) -> Result<()> {
        #[allow(clippy::let_and_return)]
        let (composition, mode) = {
            let text_service = self.borrow()?;
            let composition = text_service.borrow_composition()?.clone();
            let mode = IMEState::get()?.input_mode.clone();
            (composition, mode)
        };

        let mut preview = composition.preview.clone();
        let mut suffix = composition.suffix.clone();
        let mut raw_input = composition.raw_input.clone();
        let mut raw_hiragana = composition.raw_hiragana.clone();
        let mut corresponding_count = composition.corresponding_count.clone();
        let mut candidates = composition.candidates.clone();
        let mut selection_index = composition.selection_index;
        let mut converted_by_function_key = composition.converted_by_function_key;
        let mut reconversion_original = composition.reconversion_original.clone();
        let mut ipc_service = IMEState::get()?
            .ipc_service
            .clone()
            .context("ipc_service is None")?;
        let mut transition = transition;

        self.update_context(&preview)?;

        for action in actions {
            match action {
                ClientAction::StartComposition => {
                    self.start_composition()?;
                    self.update_pos()?;
                    ipc_service.show_window()?;
                }
                ClientAction::EndComposition => {
                    self.end_composition()?;
                    converted_by_function_key = false;
                    reconversion_original = None;
                    selection_index = 0;
                    corresponding_count = 0;
                    preview.clear();
                    suffix.clear();
                    raw_input.clear();
                    raw_hiragana.clear();
                    ipc_service.hide_window()?;
                    ipc_service.set_candidates(vec![])?;
                    ipc_service.clear_text()?;
                }
                ClientAction::AppendText(text) => {
                    converted_by_function_key = false;
                    reconversion_original = None;
                    raw_input.push_str(&text);

                    let text = match mode {
                        InputMode::Kana => to_fullwidth(text, false),
                        InputMode::Latin => text.to_string(),
                    };

                    candidates = ipc_service.append_text(text.clone())?;
                    let text = candidates.texts[selection_index as usize].clone();
                    let sub_text = candidates.sub_texts[selection_index as usize].clone();
                    let hiragana = candidates.hiragana.clone();

                    corresponding_count = candidates.corresponding_count[selection_index as usize];

                    preview = text.clone();
                    suffix = sub_text.clone();
                    raw_hiragana = hiragana.clone();

                    self.set_text(&text, &sub_text)?;
                    ipc_service.set_candidates(candidates.texts.clone())?;
                    ipc_service.set_selection(selection_index as i32)?;
                }
                ClientAction::RemoveText => {
                    converted_by_function_key = false;
                    reconversion_original = None;
                    candidates = ipc_service.remove_text()?;
                    let empty = "".to_string();
                    let text = candidates
                        .texts
                        .get(selection_index as usize)
                        .cloned()
                        .unwrap_or(empty.clone());
                    let sub_text = candidates
                        .sub_texts
                        .get(selection_index as usize)
                        .cloned()
                        .unwrap_or(empty.clone());
                    let hiragana = candidates.hiragana.clone();
                    corresponding_count = candidates
                        .corresponding_count
                        .get(selection_index as usize)
                        .cloned()
                        .unwrap_or(0);

                    raw_input = raw_input
                        .chars()
                        .take(corresponding_count as usize)
                        .collect();
                    preview = text.clone();
                    suffix = sub_text.clone();
                    raw_hiragana = hiragana.clone();

                    self.set_text(&text, &sub_text)?;
                    ipc_service.set_candidates(candidates.texts.clone())?;
                    ipc_service.set_selection(selection_index as i32)?;
                }
                ClientAction::MoveCursor(_offset) => {
                    // TODO: I'll use azookey-kkc's composingText
                    // self.set_cursor(offset)?;
                }
                ClientAction::SetIMEMode(mode) => {
                    converted_by_function_key = false;
                    self.start_composition()?;
                    self.update_pos()?;
                    self.end_composition()?;

                    let mut ime_state = IMEState::get()?;
                    ime_state.input_mode = mode.clone();

                    // update the language bar
                    self.update_lang_bar()?;

                    let mode = match mode {
                        InputMode::Latin => "A",
                        InputMode::Kana => "あ",
                    };

                    ipc_service.set_input_mode(mode)?;

                    selection_index = 0;
                    corresponding_count = 0;
                    preview.clear();
                    suffix.clear();
                    raw_input.clear();
                    raw_hiragana.clear();
                    ipc_service.clear_text()?;
                }
                ClientAction::SetSelection(selection) => {
                    converted_by_function_key = false;
                    let candidates = {
                        let text_service = self.borrow()?;
                        let composition = text_service.borrow_composition()?.clone();
                        let candidates = composition.candidates.clone();
                        candidates
                    };

                    let texts = candidates.texts.clone();
                    let sub_texts = candidates.sub_texts.clone();

                    selection_index = match selection {
                        SetSelectionType::Up => max(0, selection_index - 1),
                        SetSelectionType::Down => min(texts.len() as i32 - 1, selection_index + 1),
                        SetSelectionType::Number(number) => *number,
                    };

                    ipc_service.set_selection(selection_index as i32)?;
                    let text = texts[selection_index as usize].clone();
                    let sub_text = sub_texts[selection_index as usize].clone();
                    let hiragana = candidates.hiragana.clone();
                    corresponding_count = candidates.corresponding_count[selection_index as usize];

                    preview = text.clone();
                    suffix = sub_text.clone();
                    raw_hiragana = hiragana.clone();

                    self.set_text(&text, &sub_text)?;
                }
                ClientAction::MoveSegmentBoundary(delta) => {
                    converted_by_function_key = false;
                    let segment_len = moved_segment_len(&raw_hiragana, &suffix, *delta);
                    candidates = ipc_service.set_segment(segment_len as i32)?;
                    selection_index = 0;

                    let text = candidates.texts[selection_index as usize].clone();
                    let sub_text = candidates.sub_texts[selection_index as usize].clone();
                    corresponding_count = candidates.corresponding_count[selection_index as usize];

                    preview = text.clone();
                    suffix = sub_text.clone();
                    raw_hiragana = candidates.hiragana.clone();

                    self.set_text(&text, &sub_text)?;
                    ipc_service.set_candidates(candidates.texts.clone())?;
                    ipc_service.set_selection(selection_index as i32)?;
                }
                ClientAction::ShrinkText(text) => {
                    converted_by_function_key = false;
                    reconversion_original = None;
                    // shrink text
                    raw_input.push_str(&text);
                    raw_input = raw_input
                        .chars()
                        .skip(corresponding_count as usize)
                        .collect();

                    ipc_service.shrink_text(corresponding_count.clone())?;
                    let text = match mode {
                        InputMode::Kana => to_fullwidth(text, false),
                        InputMode::Latin => text.to_string(),
                    };
                    candidates = ipc_service.append_text(text)?;
                    selection_index = 0;

                    let text = candidates.texts[selection_index as usize].clone();
                    let sub_text = candidates.sub_texts[selection_index as usize].clone();
                    let hiragana = candidates.hiragana.clone();
                    self.shift_start(&preview, &text)?;

                    corresponding_count = candidates.corresponding_count[selection_index as usize];
                    preview = text.clone();
                    suffix = sub_text.clone();
                    raw_hiragana = hiragana.clone();

                    ipc_service.set_candidates(candidates.texts.clone())?;
                    ipc_service.set_selection(selection_index as i32)?;
                    self.update_pos()?;

                    transition = CompositionState::Composing;
                }
                ClientAction::CommitCandidate => {
                    // 表示中の文字列が候補そのもののときだけ学習する（F6〜F10 の置き換えは学習しない）
                    let is_candidate = !converted_by_function_key
                        && candidates.texts.get(selection_index as usize) == Some(&preview);
                    if is_candidate && self.is_learning_allowed_field().unwrap_or(true) {
                        // 学習できなくても入力は続けられるので、失敗は記録だけする
                        if let Err(error) = ipc_service.commit_candidate(preview.clone()) {
                            tracing::warn!("Failed to commit candidate: {error:?}");
                        }
                    }
                }
                ClientAction::ForgetCandidate => {
                    // F6〜F10 の置き換えは候補ではないので、何もしない
                    let is_candidate = !converted_by_function_key
                        && candidates.texts.get(selection_index as usize) == Some(&preview);
                    if is_candidate {
                        candidates = ipc_service.forget_candidate(preview.clone())?;
                        // 忘れた候補は順位が下がるので、先頭の候補を選び直す
                        selection_index = 0;
                        let text = candidates.texts.first().cloned().unwrap_or_default();
                        let sub_text = candidates.sub_texts.first().cloned().unwrap_or_default();
                        corresponding_count =
                            candidates.corresponding_count.first().cloned().unwrap_or(0);

                        preview = text.clone();
                        suffix = sub_text.clone();
                        raw_hiragana = candidates.hiragana.clone();

                        self.set_text(&text, &sub_text)?;
                        ipc_service.set_candidates(candidates.texts.clone())?;
                        ipc_service.set_selection(selection_index)?;
                    }
                }
                ClientAction::SetTextWithType(set_type) => {
                    converted_by_function_key = true;
                    let text = match set_type {
                        SetTextType::Hiragana => raw_hiragana.clone(),
                        SetTextType::Katakana => to_katakana(&raw_hiragana),
                        SetTextType::HalfKatakana => to_half_katakana(&raw_hiragana),
                        SetTextType::FullLatin => to_fullwidth(&raw_input, true),
                        SetTextType::HalfLatin => to_halfwidth(&raw_input),
                    };

                    self.set_text(&text, "")?;
                }
                ClientAction::StartReconversion(range) => {
                    let Some(reconverted) =
                        self.begin_reconversion(&mut ipc_service, range.as_ref())?
                    else {
                        continue;
                    };
                    preview = reconverted.preview;
                    suffix = reconverted.suffix;
                    raw_input = reconverted.raw_input;
                    raw_hiragana = reconverted.raw_hiragana;
                    corresponding_count = reconverted.corresponding_count;
                    selection_index = reconverted.selection_index;
                    candidates = reconverted.candidates;
                    converted_by_function_key = reconverted.converted_by_function_key;
                    reconversion_original = reconverted.reconversion_original;
                    transition = reconverted.state;
                }
                ClientAction::RestoreReconversion => {
                    if let Some(original) = &reconversion_original {
                        self.set_text(original, "")?;
                        preview = original.clone();
                        suffix.clear();
                    }
                }
            }
        }

        let text_service = self.borrow()?;
        let mut composition = text_service.borrow_mut_composition()?;

        composition.preview = preview.clone();
        composition.state = transition;
        composition.selection_index = selection_index;
        composition.raw_input = raw_input.clone();
        composition.raw_hiragana = raw_hiragana.clone();
        composition.candidates = candidates;
        composition.suffix = suffix.clone();
        composition.corresponding_count = corresponding_count;
        composition.converted_by_function_key = converted_by_function_key;
        composition.reconversion_original = reconversion_original;

        Ok(())
    }
}
