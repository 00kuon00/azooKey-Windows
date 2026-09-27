use azookey_server::TonicNamedPipeServer;
use tonic::{transport::Server, Request, Response, Status};
use tonic_reflection::server::Builder as ReflectionBuilder;

use shared::proto::azookey_service_server::{AzookeyService, AzookeyServiceServer};
use shared::proto::{
    AppendTextRequest, AppendTextResponse, ClearTextRequest, ClearTextResponse, ComposingText,
    MoveCursorRequest, MoveCursorResponse, RemoveTextRequest, RemoveTextResponse,
    SetSegmentRequest, SetSegmentResponse, ShrinkTextRequest, ShrinkTextResponse, Suggestion,
};

use std::ffi::{c_char, c_int, CStr, CString};

const USE_ZENZAI: bool = true;

// Swift の `Int`（64 ビット）。Swift 側が `UnsafeMutablePointer<Int>` で書き込む引数はこの型で受ける。
// `c_int`（32 ビット）で受けると 8 バイト書き込まれて隣のスタックが壊れ、変換エンジンが落ちる
type SwiftInt = isize;

struct RawComposingText {
    text: String,
    cursor: i8,
}

#[derive(Debug, Clone)]
#[repr(C)]
struct FFICandidate {
    text: *mut c_char,
    subtext: *mut c_char,
    hiragana: *mut c_char,
    corresponding_count: c_int,
}

unsafe extern "C" {
    fn Initialize(path: *const c_char, use_zenzai: bool);
    fn SetContext(context: *const c_char);
    fn AppendText(input: *const c_char, cursorPtr: *mut SwiftInt) -> *mut c_char;
    fn RemoveText(cursorPtr: *mut SwiftInt) -> *mut c_char;
    fn MoveCursor(offset: c_int, cursorPtr: *mut SwiftInt) -> *mut c_char;
    fn ShrinkText(offset: c_int) -> *mut c_char;
    fn SetSegmentSurfaceCount(count: c_int) -> *mut c_char;
    fn ClearText();
    fn GetComposedText(lengthPtr: *mut SwiftInt) -> *mut *mut FFICandidate;
    fn LoadConfig();
    fn CommitCandidate(text: *const c_char);
    fn ResetLearning();
    fn ForgetCandidate(text: *const c_char) -> *mut c_char;
    fn GetUnregisteredUserDictionaryEntries() -> *mut c_char;
    fn StartReconversion(surface: *const c_char) -> *mut c_char;
    fn GetPredictions(lengthPtr: *mut SwiftInt) -> *mut *mut FFICandidate;
    fn RequestTypoCorrection(lengthPtr: *mut SwiftInt) -> *mut *mut FFICandidate;
}

#[derive(serde::Deserialize)]
struct RawUserDictionaryEntry {
    reading: String,
    word: String,
}

/// 直近のユーザー辞書の作り直しで登録できなかった語（Swift 側は JSON で返す）
fn unregistered_user_dictionary_entries() -> Vec<shared::proto::UserDictionaryEntry> {
    let json = unsafe {
        let result = GetUnregisteredUserDictionaryEntries();
        CStr::from_ptr(result).to_string_lossy().into_owned()
    };
    serde_json::from_str::<Vec<RawUserDictionaryEntry>>(&json)
        .unwrap_or_default()
        .into_iter()
        .map(|entry| shared::proto::UserDictionaryEntry {
            reading: entry.reading,
            word: entry.word,
        })
        .collect()
}

fn initialize(path: &str) {
    unsafe {
        let path = CString::new(path).expect("CString::new failed");
        Initialize(path.as_ptr(), USE_ZENZAI);
    }
}

fn add_text(input: &str) -> RawComposingText {
    unsafe {
        let input = CString::new(input).expect("CString::new failed");
        let mut cursor: SwiftInt = 0;

        let result = AppendText(input.as_ptr(), &mut cursor);

        let text = CStr::from_ptr(&*result as *const c_char).to_str().unwrap();

        RawComposingText {
            text: text.to_string(),
            cursor: cursor as i8,
        }
    }
}

fn move_cursor(offset: i8) -> RawComposingText {
    unsafe {
        let offset = c_int::from(offset);
        println!("Offset: {}", offset);
        let mut cursor: SwiftInt = 0;

        let result = MoveCursor(offset, &mut cursor);

        let text = CStr::from_ptr(&*result as *const c_char).to_str().unwrap();

        RawComposingText {
            text: text.to_string(),
            cursor: cursor as i8,
        }
    }
}

fn remove_text() -> RawComposingText {
    unsafe {
        let mut cursor: SwiftInt = 0;

        let result = RemoveText(&mut cursor);

        let text = CStr::from_ptr(&*result as *const c_char).to_str().unwrap();

        RawComposingText {
            text: text.to_string(),
            cursor: cursor as i8,
        }
    }
}

fn clear_text() {
    unsafe {
        ClearText();
    }
}

/// Swift が返した候補の配列を読む（同じ文字列の候補は最初の 1 件だけ残す）。2 つ目は読み（hiragana）
unsafe fn read_candidates(
    list: unsafe extern "C" fn(*mut SwiftInt) -> *mut *mut FFICandidate,
) -> Vec<(Suggestion, String)> {
    unsafe {
        let mut length: SwiftInt = 0;
        let result = list(&mut length);
        let mut candidates: Vec<(Suggestion, String)> = Vec::with_capacity(length as usize);
        for index in 0..length as usize {
            let candidate = (**result.add(index)).clone();
            let text = CStr::from_ptr(candidate.text)
                .to_string_lossy()
                .into_owned();
            let subtext = CStr::from_ptr(candidate.subtext)
                .to_string_lossy()
                .into_owned();
            let hiragana = CStr::from_ptr(candidate.hiragana)
                .to_string_lossy()
                .into_owned();

            // check if suggestions have the same text
            if candidates.iter().any(|(s, _)| s.text == text) {
                continue;
            }
            candidates.push((
                Suggestion {
                    text,
                    subtext,
                    corresponding_count: candidate.corresponding_count,
                },
                hiragana,
            ));
        }
        candidates
    }
}

fn get_composed_text() -> Vec<Suggestion> {
    unsafe { read_candidates(GetComposedText) }
        .into_iter()
        .map(|(suggestion, _)| suggestion)
        .collect()
}

/// 直近の get_composed_text の予測（get_composed_text のあとに呼ぶ）
fn get_predictions() -> Vec<Suggestion> {
    unsafe { read_candidates(GetPredictions) }
        .into_iter()
        .map(|(suggestion, _)| suggestion)
        .collect()
}

/// 候補を変換し直し、予測と合わせて返す
fn composing_text(hiragana: String) -> ComposingText {
    let suggestions = get_composed_text();
    ComposingText {
        hiragana,
        suggestions,
        predictions: get_predictions(),
    }
}

/// 入力中の文字列を `surface` の読みにする。返り値は読み（推定できなければ空）
fn start_reconversion(surface: &CStr) -> String {
    unsafe {
        let result = StartReconversion(surface.as_ptr());
        CStr::from_ptr(result).to_string_lossy().into_owned()
    }
}

fn shrink_text(offset: i8) -> RawComposingText {
    unsafe {
        let offset = c_int::from(offset);
        let result = ShrinkText(offset);

        let text = CStr::from_ptr(&*result as *const c_char).to_str().unwrap();

        RawComposingText {
            text: text.to_string(),
            cursor: 0,
        }
    }
}

#[derive(Debug, Default)]
pub struct MyAzookeyService;

#[tonic::async_trait]
impl AzookeyService for MyAzookeyService {
    async fn append_text(
        &self,
        request: Request<AppendTextRequest>,
    ) -> Result<Response<AppendTextResponse>, Status> {
        let input = request.into_inner().text_to_append;
        let composing_text = add_text(&input);

        Ok(Response::new(AppendTextResponse {
            composing_text: Some(self::composing_text(composing_text.text)),
        }))
    }

    async fn remove_text(
        &self,
        _: Request<RemoveTextRequest>,
    ) -> Result<Response<RemoveTextResponse>, Status> {
        let composing_text = remove_text();

        Ok(Response::new(RemoveTextResponse {
            composing_text: Some(self::composing_text(composing_text.text)),
        }))
    }

    async fn move_cursor(
        &self,
        request: Request<MoveCursorRequest>,
    ) -> Result<Response<MoveCursorResponse>, Status> {
        let offset = request.into_inner().offset as i8;
        let composing_text = move_cursor(offset);

        Ok(Response::new(MoveCursorResponse {
            composing_text: Some(self::composing_text(composing_text.text)),
        }))
    }

    async fn clear_text(
        &self,
        _: Request<ClearTextRequest>,
    ) -> Result<Response<ClearTextResponse>, Status> {
        clear_text();
        Ok(Response::new(ClearTextResponse {}))
    }

    async fn shrink_text(
        &self,
        request: Request<ShrinkTextRequest>,
    ) -> Result<Response<ShrinkTextResponse>, Status> {
        let offset = request.into_inner().offset as i8;
        let composing_text = shrink_text(offset);

        Ok(Response::new(ShrinkTextResponse {
            composing_text: Some(self::composing_text(composing_text.text)),
        }))
    }

    async fn set_segment(
        &self,
        request: Request<SetSegmentRequest>,
    ) -> Result<Response<SetSegmentResponse>, Status> {
        let surface_count = request.into_inner().surface_count;
        let hiragana = unsafe {
            let result = SetSegmentSurfaceCount(surface_count);
            CStr::from_ptr(result).to_string_lossy().into_owned()
        };

        Ok(Response::new(SetSegmentResponse {
            composing_text: Some(composing_text(hiragana)),
        }))
    }

    async fn set_context(
        &self,
        request: Request<shared::proto::SetContextRequest>,
    ) -> Result<Response<shared::proto::SetContextResponse>, Status> {
        let context = request.into_inner().context;
        let trimmed_context = context
            .split('\r')
            .filter(|s| !s.is_empty())
            .last()
            .unwrap_or_default();

        let context = CString::new(trimmed_context).expect("CString::new failed");

        unsafe { SetContext(context.as_ptr()) };
        Ok(Response::new(shared::proto::SetContextResponse {}))
    }

    async fn update_config(
        &self,
        _: Request<shared::proto::UpdateConfigRequest>,
    ) -> Result<Response<shared::proto::UpdateConfigResponse>, Status> {
        // LoadConfig はユーザー辞書も（元データが変わっていれば）作り直す
        unsafe { LoadConfig() };
        Ok(Response::new(shared::proto::UpdateConfigResponse {
            unregistered_user_dictionary_entries: unregistered_user_dictionary_entries(),
        }))
    }

    async fn commit_candidate(
        &self,
        request: Request<shared::proto::CommitCandidateRequest>,
    ) -> Result<Response<shared::proto::CommitCandidateResponse>, Status> {
        let text = request.into_inner().text;
        let text = CString::new(text).map_err(|e| Status::invalid_argument(e.to_string()))?;
        unsafe { CommitCandidate(text.as_ptr()) };
        Ok(Response::new(shared::proto::CommitCandidateResponse {}))
    }

    async fn reset_learning(
        &self,
        _: Request<shared::proto::ResetLearningRequest>,
    ) -> Result<Response<shared::proto::ResetLearningResponse>, Status> {
        unsafe { ResetLearning() };
        Ok(Response::new(shared::proto::ResetLearningResponse {}))
    }

    async fn forget_candidate(
        &self,
        request: Request<shared::proto::ForgetCandidateRequest>,
    ) -> Result<Response<shared::proto::ForgetCandidateResponse>, Status> {
        let text = request.into_inner().text;
        let text = CString::new(text).map_err(|e| Status::invalid_argument(e.to_string()))?;
        let hiragana = unsafe {
            let result = ForgetCandidate(text.as_ptr());
            CStr::from_ptr(result).to_string_lossy().into_owned()
        };

        Ok(Response::new(shared::proto::ForgetCandidateResponse {
            composing_text: Some(composing_text(hiragana)),
        }))
    }

    async fn start_reconversion(
        &self,
        request: Request<shared::proto::StartReconversionRequest>,
    ) -> Result<Response<shared::proto::StartReconversionResponse>, Status> {
        let text = request.into_inner().text;
        let text = CString::new(text).map_err(|e| Status::invalid_argument(e.to_string()))?;
        let hiragana = start_reconversion(&text);
        // 読みが無ければ変換しない（空の入力で変換すると候補の取り出しが失敗する）
        let composing_text = if hiragana.is_empty() {
            ComposingText {
                hiragana,
                ..Default::default()
            }
        } else {
            composing_text(hiragana)
        };

        Ok(Response::new(shared::proto::StartReconversionResponse {
            composing_text: Some(composing_text),
        }))
    }

    async fn request_typo_correction(
        &self,
        _: Request<shared::proto::RequestTypoCorrectionRequest>,
    ) -> Result<Response<shared::proto::RequestTypoCorrectionResponse>, Status> {
        let corrections = unsafe { read_candidates(RequestTypoCorrection) }
            .into_iter()
            .map(|(suggestion, hiragana)| shared::proto::TypoCorrection {
                text: suggestion.text,
                hiragana,
                corresponding_count: suggestion.corresponding_count,
            })
            .collect();
        Ok(Response::new(
            shared::proto::RequestTypoCorrectionResponse { corrections },
        ))
    }
}

// Swift 側の FFI 関数は @MainActor なので、tonic のハンドラを 1 本のスレッドで動かす
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("AzookeyServer started");
    // get executable directory
    let current_exe = std::env::current_exe()?;
    let parent_dir = current_exe.parent().unwrap();
    initialize(parent_dir.to_str().unwrap());

    let service = MyAzookeyService::default();

    println!("AzookeyServer listening");

    Server::builder()
        .add_service(AzookeyServiceServer::new(service))
        .add_service(
            ReflectionBuilder::configure()
                .register_encoded_file_descriptor_set(shared::proto::FILE_DESCRIPTOR_SET)
                .build_v1()
                .unwrap(),
        )
        .serve_with_incoming(TonicNamedPipeServer::new("azookey_server"))
        .await?;

    Ok(())
}
