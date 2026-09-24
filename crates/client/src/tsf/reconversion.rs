// 再変換の TSF 側: 選択範囲の文字列の読み取り、選択範囲での composition の開始、
// ITfFunctionProvider / ITfFnReconversion（アプリや OS から再変換を頼まれたとき）
use std::{mem::ManuallyDrop, rc::Rc};

use anyhow::{Context as _, Result};
use windows::{
    core::{Interface, BSTR, GUID},
    Win32::{
        Foundation::{BOOL, E_INVALIDARG, E_NOINTERFACE, E_NOTIMPL},
        UI::TextServices::{
            ITfCandidateList, ITfComposition, ITfCompositionSink, ITfContext,
            ITfContextComposition, ITfFnReconversion, ITfFnReconversion_Impl,
            ITfFunctionProvider_Impl, ITfFunction_Impl, ITfRange, TF_DEFAULT_SELECTION, TF_ES_READ,
            TF_ES_READWRITE, TF_ES_SYNC, TF_SELECTION, TF_TF_MOVESTART,
        },
    },
};

use crate::{
    engine::{
        client_action::ClientAction, composition::CompositionState,
        reconversion::MAX_RECONVERSION_CHARS,
    },
    globals::GUID_TEXT_SERVICE,
};

use super::{
    edit_session::edit_session_with_flags,
    factory::{TextServiceFactory, TextServiceFactory_Impl},
};

/// 読み取る長さの上限（UTF-16）。サロゲートペアでも上限の文字数を読めるように 2 倍にする
const MAX_RECONVERSION_UTF16: usize = MAX_RECONVERSION_CHARS * 2;

/// `range` が `Some` ならその複製、`None` なら選択範囲を返す
unsafe fn target_range(
    cookie: u32,
    context: &ITfContext,
    range: Option<&ITfRange>,
) -> Result<Option<ITfRange>> {
    if let Some(range) = range {
        return Ok(Some(range.Clone()?));
    }
    let mut selection = [TF_SELECTION::default()];
    let mut fetched = 0;
    context.GetSelection(cookie, TF_DEFAULT_SELECTION, &mut selection, &mut fetched)?;
    if fetched == 0 {
        return Ok(None);
    }
    // GetSelection が参照を渡すので、取り出して手放す
    Ok(ManuallyDrop::take(&mut selection[0].range))
}

/// `range` の文字列。空・上限より長い・UTF-16 として壊れているときは `None`
unsafe fn range_text(cookie: u32, range: &ITfRange) -> Result<Option<String>> {
    if range.IsEmpty(cookie)?.as_bool() {
        return Ok(None);
    }
    let mut buffer = vec![0u16; MAX_RECONVERSION_UTF16 + 1];
    let mut length = 0;
    // TF_TF_MOVESTART は範囲の始まりを動かすので複製で読む
    range
        .Clone()?
        .GetText(cookie, TF_TF_MOVESTART, &mut buffer, &mut length)?;
    let length = length as usize;
    if length == 0 || length > MAX_RECONVERSION_UTF16 {
        return Ok(None);
    }
    Ok(String::from_utf16(&buffer[..length]).ok())
}

impl TextServiceFactory {
    /// 再変換する文字列（`range` が `None` なら選択範囲）を読む
    pub fn reconversion_text(&self, range: Option<&ITfRange>) -> Result<Option<String>> {
        let (tid, context) = {
            let text_service = self.borrow()?;
            (text_service.tid, text_service.context::<ITfContext>()?)
        };
        let range = range.cloned();

        // 結果をこの場で使うので同期で読む。すぐに読めなければ再変換しない
        let text = edit_session_with_flags(
            tid,
            context.clone(),
            TF_ES_READ | TF_ES_SYNC,
            Rc::new(move |cookie| unsafe {
                match target_range(cookie, &context, range.as_ref())? {
                    Some(range) => range_text(cookie, &range),
                    None => Ok(None),
                }
            }),
        )?;
        Ok(text.flatten())
    }

    /// `range`（`None` なら選択範囲）を composition にする。
    /// その範囲の文字列が `expected` と違えば（読みを調べている間に変わったら）始めずに `false` を返す
    pub fn start_composition_on_reconversion_range(
        &self,
        range: Option<&ITfRange>,
        expected: &str,
    ) -> Result<bool> {
        let (tid, context, context_composition, sink) = {
            let text_service = self.borrow()?;
            (
                text_service.tid,
                text_service.context::<ITfContext>()?,
                text_service.context::<ITfContextComposition>()?,
                text_service.this::<ITfCompositionSink>()?,
            )
        };
        let range = range.cloned();
        let expected = expected.to_string();

        let composition = edit_session_with_flags::<Option<ITfComposition>>(
            tid,
            context.clone(),
            TF_ES_READWRITE | TF_ES_SYNC,
            Rc::new(move |cookie| unsafe {
                let Some(range) = target_range(cookie, &context, range.as_ref())? else {
                    return Ok(None);
                };
                if range_text(cookie, &range)?.as_deref() != Some(expected.as_str()) {
                    return Ok(None);
                }
                Ok(Some(
                    context_composition.StartComposition(cookie, &range, &sink)?,
                ))
            }),
        )?
        .flatten();

        let Some(composition) = composition else {
            return Ok(false);
        };
        self.borrow()?.borrow_mut_composition()?.tip_composition = Some(composition);
        Ok(true)
    }

    /// アプリや OS から頼まれた再変換（`ITfFnReconversion::Reconvert`）
    fn reconvert_range(&self, range: &ITfRange) -> Result<()> {
        let context = unsafe { range.GetContext()? };
        let state = {
            let mut text_service = self.borrow_mut()?;
            text_service.context = Some(context);
            let state = text_service.borrow_composition()?.state.clone();
            state
        };
        // 入力中なら、その入力を確定してから再変換する
        if state != CompositionState::None {
            self.handle_action(&[ClientAction::EndComposition], CompositionState::None)?;
        }
        self.handle_action(
            &[ClientAction::StartReconversion(Some(range.clone()))],
            CompositionState::None,
        )
    }
}

impl ITfFunctionProvider_Impl for TextServiceFactory_Impl {
    fn GetType(&self) -> windows::core::Result<GUID> {
        Ok(GUID_TEXT_SERVICE)
    }

    fn GetDescription(&self) -> windows::core::Result<BSTR> {
        Ok(BSTR::from("Azookey"))
    }

    fn GetFunction(
        &self,
        rguid: *const GUID,
        riid: *const GUID,
    ) -> windows::core::Result<windows::core::IUnknown> {
        let (Some(rguid), Some(riid)) = (unsafe { rguid.as_ref() }, unsafe { riid.as_ref() })
        else {
            return Err(E_INVALIDARG.into());
        };
        // 再変換（GUID_NULL と IID_ITfFnReconversion）だけを提供する
        if *rguid != GUID::zeroed() || *riid != ITfFnReconversion::IID {
            return Err(E_NOINTERFACE.into());
        }
        let function = self
            .borrow()
            .ok()
            .and_then(|text_service| text_service.this::<ITfFnReconversion>().ok())
            .ok_or(windows::core::Error::from(E_NOINTERFACE))?;
        // 戻り値の型は IUnknown だが、riid のインターフェースのポインタを返す決まりなので QueryInterface しない
        Ok(function.into())
    }
}

impl ITfFunction_Impl for TextServiceFactory_Impl {
    fn GetDisplayName(&self) -> windows::core::Result<BSTR> {
        Ok(BSTR::from("再変換"))
    }
}

impl ITfFnReconversion_Impl for TextServiceFactory_Impl {
    fn QueryRange(
        &self,
        prange: Option<&ITfRange>,
        ppnewrange: *mut Option<ITfRange>,
        pfconvertable: *mut BOOL,
    ) -> windows::core::Result<()> {
        let range = prange.ok_or(windows::core::Error::from(E_INVALIDARG))?;
        if pfconvertable.is_null() {
            return Err(E_INVALIDARG.into());
        }
        // 範囲をそのまま再変換する（語の境界まで広げない）。文字列の中身は Reconvert で確かめる
        unsafe {
            if !ppnewrange.is_null() {
                ppnewrange.write(Some(range.Clone()?));
            }
            pfconvertable.write(true.into());
        }
        Ok(())
    }

    fn GetReconversion(
        &self,
        _prange: Option<&ITfRange>,
    ) -> windows::core::Result<ITfCandidateList> {
        // 候補は自前の候補ウィンドウに出すので、TSF の候補リストは返さない
        Err(E_NOTIMPL.into())
    }

    #[macros::anyhow]
    fn Reconvert(&self, prange: Option<&ITfRange>) -> Result<()> {
        let range = prange.context("range is null")?;
        self.reconvert_range(range)
    }
}
