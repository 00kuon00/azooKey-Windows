use std::rc::Rc;

use anyhow::Result;
use windows::{
    core::{IUnknown, Interface},
    Win32::{
        System::Com::CoTaskMemFree,
        UI::TextServices::{
            ITfContext, ITfInputScope, InputScope, GUID_PROP_INPUTSCOPE, IS_ALPHANUMERIC_PIN,
            IS_ALPHANUMERIC_PIN_SET, IS_NUMERIC_PIN, IS_PASSWORD, IS_PRIVATE,
        },
    },
};

use super::{edit_session::edit_session, factory::TextServiceFactory};

// 確定した候補を学習しない欄（パスワード・PIN・プライベートな入力）
const NO_LEARNING_SCOPES: [InputScope; 5] = [
    IS_PASSWORD,
    IS_PRIVATE,
    IS_NUMERIC_PIN,
    IS_ALPHANUMERIC_PIN,
    IS_ALPHANUMERIC_PIN_SET,
];

impl TextServiceFactory {
    /// いま入力している欄の InputScope が、学習してよいものか。
    /// InputScope を持たない欄（多くのアプリ）は学習してよいとみなす
    pub fn is_learning_allowed_field(&self) -> Result<bool> {
        let text_service = self.borrow()?;
        let Some(composition) = text_service.borrow_composition()?.tip_composition.clone() else {
            return Ok(true);
        };
        let context = text_service.context::<ITfContext>()?;

        let scopes = edit_session::<Vec<InputScope>>(
            text_service.tid,
            context.clone(),
            Rc::new(move |cookie| unsafe {
                let range = composition.GetRange()?;
                let property = context.GetAppProperty(&GUID_PROP_INPUTSCOPE)?;
                let value = property.GetValue(cookie, &range)?;
                let Ok(unknown) = IUnknown::try_from(&value) else {
                    return Ok(vec![]);
                };
                let input_scope = unknown.cast::<ITfInputScope>()?;

                let mut pointer = std::ptr::null_mut();
                let mut count = 0;
                input_scope.GetInputScopes(&mut pointer, &mut count)?;
                if pointer.is_null() {
                    return Ok(vec![]);
                }
                let scopes = std::slice::from_raw_parts(pointer, count as usize).to_vec();
                CoTaskMemFree(Some(pointer as *const _));
                Ok(scopes)
            }),
        )?;

        let scopes = scopes.unwrap_or_default();
        Ok(!scopes
            .iter()
            .any(|scope| NO_LEARNING_SCOPES.contains(scope)))
    }
}
