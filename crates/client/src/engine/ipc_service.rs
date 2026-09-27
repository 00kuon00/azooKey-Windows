use anyhow::{Context as _, Result};
use hyper_util::rt::TokioIo;
use shared::proto::{
    azookey_service_client::AzookeyServiceClient, window_service_client::WindowServiceClient,
};
use std::{sync::Arc, time::Duration};
use tokio::{net::windows::named_pipe::ClientOptions, time};
use tonic::transport::Endpoint;
use tower::service_fn;
use windows::Win32::Foundation::ERROR_PIPE_BUSY;

// connect to kkc server
#[derive(Debug, Clone)]
pub struct IPCService {
    // kkc server client
    azookey_client: AzookeyServiceClient<tonic::transport::channel::Channel>,
    // candidate window server client
    window_client: WindowServiceClient<tonic::transport::channel::Channel>,
    runtime: Arc<tokio::runtime::Runtime>,
}

#[derive(Debug, Clone, Default)]
pub struct Candidates {
    pub texts: Vec<String>,
    pub sub_texts: Vec<String>,
    pub hiragana: String,
    pub corresponding_count: Vec<i32>,
    // 入力中の読みの続きを補った予測。確定すると入力中の文字列をすべて使う
    pub predictions: Vec<Prediction>,
    // 打ち間違いを直した「もしかして」（Space で変換したときに RequestTypoCorrection で取る）
    pub typos: Vec<TypoCandidate>,
    // この候補（読み）で「もしかして」をもう求めたか。読みが変わると候補ごと作り直されて false に戻る
    pub typos_requested: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Prediction {
    pub text: String,
    pub corresponding_count: i32, // 入力全体の文字数
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TypoCandidate {
    pub text: String,             // 直した読みを変換した 1 位
    pub hiragana: String,         // 直した読み
    pub corresponding_count: i32, // 入力全体の文字数
}

impl From<shared::proto::ComposingText> for Candidates {
    fn from(composing_text: shared::proto::ComposingText) -> Self {
        Candidates {
            texts: composing_text
                .suggestions
                .iter()
                .map(|s| s.text.clone())
                .collect(),
            sub_texts: composing_text
                .suggestions
                .iter()
                .map(|s| s.subtext.clone())
                .collect(),
            hiragana: composing_text.hiragana,
            corresponding_count: composing_text
                .suggestions
                .iter()
                .map(|s| s.corresponding_count)
                .collect(),
            predictions: composing_text
                .predictions
                .into_iter()
                .map(|s| Prediction {
                    text: s.text,
                    corresponding_count: s.corresponding_count,
                })
                .collect(),
            typos: vec![],
            typos_requested: false,
        }
    }
}

impl IPCService {
    pub fn new() -> Result<Self> {
        let runtime = tokio::runtime::Runtime::new()?;

        let server_channel = runtime.block_on(
            Endpoint::try_from("http://[::]:50051")?.connect_with_connector(service_fn(
                |_| async {
                    let client = loop {
                        match ClientOptions::new().open(r"\\.\pipe\azookey_server") {
                            Ok(client) => break client,
                            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY.0 as i32) => (),
                            Err(e) => return Err(e),
                        }

                        time::sleep(Duration::from_millis(50)).await;
                    };

                    Ok::<_, std::io::Error>(TokioIo::new(client))
                },
            )),
        )?;

        let ui_channel = runtime.block_on(
            Endpoint::try_from("http://[::]:50052")?.connect_with_connector(service_fn(
                |_| async {
                    let client = loop {
                        match ClientOptions::new().open(r"\\.\pipe\azookey_ui") {
                            Ok(client) => break client,
                            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY.0 as i32) => (),
                            Err(e) => return Err(e),
                        }

                        time::sleep(Duration::from_millis(50)).await;
                    };

                    Ok::<_, std::io::Error>(TokioIo::new(client))
                },
            )),
        )?;

        let azookey_client = AzookeyServiceClient::new(server_channel);
        let window_client = WindowServiceClient::new(ui_channel);
        tracing::debug!("Connected to server: {:?}", azookey_client);

        Ok(Self {
            azookey_client,
            window_client,
            runtime: Arc::new(runtime),
        })
    }
}

// implement methods to interact with kkc server
impl IPCService {
    #[tracing::instrument]
    pub fn append_text(&mut self, text: String) -> anyhow::Result<Candidates> {
        let request = tonic::Request::new(shared::proto::AppendTextRequest {
            text_to_append: text,
        });

        let response = self
            .runtime
            .clone()
            .block_on(self.azookey_client.append_text(request))?;
        let composing_text = response.into_inner().composing_text;

        let candidates = if let Some(composing_text) = composing_text {
            Candidates::from(composing_text)
        } else {
            anyhow::bail!("composing_text is None");
        };

        Ok(candidates)
    }

    #[tracing::instrument]
    pub fn remove_text(&mut self) -> anyhow::Result<Candidates> {
        let request = tonic::Request::new(shared::proto::RemoveTextRequest {});
        let response = self
            .runtime
            .clone()
            .block_on(self.azookey_client.remove_text(request))?;
        let composing_text = response.into_inner().composing_text;

        let candidates = if let Some(composing_text) = composing_text {
            Candidates::from(composing_text)
        } else {
            anyhow::bail!("composing_text is None");
        };

        Ok(candidates)
    }

    #[tracing::instrument]
    pub fn clear_text(&mut self) -> anyhow::Result<()> {
        let request = tonic::Request::new(shared::proto::ClearTextRequest {});
        let _response = self
            .runtime
            .clone()
            .block_on(self.azookey_client.clear_text(request))?;

        Ok(())
    }

    #[tracing::instrument]
    pub fn shrink_text(&mut self, offset: i32) -> anyhow::Result<Candidates> {
        let request = tonic::Request::new(shared::proto::ShrinkTextRequest { offset });
        let response = self
            .runtime
            .clone()
            .block_on(self.azookey_client.shrink_text(request))?;
        let composing_text = response.into_inner().composing_text;

        let candidates = if let Some(composing_text) = composing_text {
            Candidates::from(composing_text)
        } else {
            anyhow::bail!("composing_text is None");
        };

        Ok(candidates)
    }

    /// 最初の文節の読みを `surface_count` 文字にして変換し直す（Shift+←→）
    #[tracing::instrument]
    pub fn set_segment(&mut self, surface_count: i32) -> anyhow::Result<Candidates> {
        let request = tonic::Request::new(shared::proto::SetSegmentRequest { surface_count });
        let response = self
            .runtime
            .clone()
            .block_on(self.azookey_client.set_segment(request))?;
        let composing_text = response.into_inner().composing_text;

        let candidates = if let Some(composing_text) = composing_text {
            Candidates::from(composing_text)
        } else {
            anyhow::bail!("composing_text is None");
        };

        Ok(candidates)
    }

    #[tracing::instrument]
    pub fn commit_candidate(&mut self, text: String) -> anyhow::Result<()> {
        let request = tonic::Request::new(shared::proto::CommitCandidateRequest { text });
        self.runtime
            .clone()
            .block_on(self.azookey_client.commit_candidate(request))?;

        Ok(())
    }

    // 候補の学習だけを忘れさせ、取り直した候補を返す
    #[tracing::instrument]
    pub fn forget_candidate(&mut self, text: String) -> anyhow::Result<Candidates> {
        let request = tonic::Request::new(shared::proto::ForgetCandidateRequest { text });
        let response = self
            .runtime
            .clone()
            .block_on(self.azookey_client.forget_candidate(request))?;
        let composing_text = response.into_inner().composing_text;

        let candidates = if let Some(composing_text) = composing_text {
            Candidates::from(composing_text)
        } else {
            anyhow::bail!("composing_text is None");
        };

        Ok(candidates)
    }

    /// 確定済みの文字列を読みに戻して候補を得る。読みが推定できなければ候補は空
    #[tracing::instrument]
    pub fn start_reconversion(&mut self, text: String) -> anyhow::Result<Candidates> {
        let request = tonic::Request::new(shared::proto::StartReconversionRequest { text });
        let response = self
            .runtime
            .clone()
            .block_on(self.azookey_client.start_reconversion(request))?;
        let composing_text = response
            .into_inner()
            .composing_text
            .context("composing_text is None")?;

        Ok(Candidates::from(composing_text))
    }

    /// 打ち間違いを直した「もしかして」を取る（Zenzai が無効なら空）
    #[tracing::instrument]
    pub fn request_typo_correction(&mut self) -> anyhow::Result<Vec<TypoCandidate>> {
        let request = tonic::Request::new(shared::proto::RequestTypoCorrectionRequest {});
        let response = self
            .runtime
            .clone()
            .block_on(self.azookey_client.request_typo_correction(request))?;
        Ok(response
            .into_inner()
            .corrections
            .into_iter()
            .map(|c| TypoCandidate {
                text: c.text,
                hiragana: c.hiragana,
                corresponding_count: c.corresponding_count,
            })
            .collect())
    }

    pub fn set_context(&mut self, context: String) -> anyhow::Result<()> {
        let request = tonic::Request::new(shared::proto::SetContextRequest { context });
        let _response = self
            .runtime
            .clone()
            .block_on(self.azookey_client.set_context(request))?;

        Ok(())
    }
}

// implement methods to interact with candidate window server
impl IPCService {
    #[tracing::instrument]
    pub fn show_window(&mut self) -> anyhow::Result<()> {
        let request = tonic::Request::new(shared::proto::EmptyResponse {});
        self.runtime
            .clone()
            .block_on(self.window_client.show_window(request))?;

        Ok(())
    }

    #[tracing::instrument]
    pub fn hide_window(&mut self) -> anyhow::Result<()> {
        let request = tonic::Request::new(shared::proto::EmptyResponse {});
        self.runtime
            .clone()
            .block_on(self.window_client.hide_window(request))?;

        Ok(())
    }

    #[tracing::instrument]
    pub fn set_window_position(
        &mut self,
        top: i32,
        left: i32,
        bottom: i32,
        right: i32,
    ) -> anyhow::Result<()> {
        let request = tonic::Request::new(shared::proto::SetPositionRequest {
            position: Some(shared::proto::WindowPosition {
                top,
                left,
                bottom,
                right,
            }),
        });
        self.runtime
            .clone()
            .block_on(self.window_client.set_window_position(request))?;

        Ok(())
    }

    #[tracing::instrument]
    /// 候補ウィンドウに、変換候補・読み・予測・もしかしてを送る。
    /// `segment_length` は Shift+←→ で区切った最初の文節の読みの文字数（区切っていなければ 0）
    pub fn set_candidates(
        &mut self,
        candidates: &Candidates,
        segment_length: i32,
    ) -> anyhow::Result<()> {
        let request = tonic::Request::new(shared::proto::SetCandidateRequest {
            candidates: candidates.texts.clone(),
            hiragana: candidates.hiragana.clone(),
            segment_length,
            predictions: candidates
                .predictions
                .iter()
                .map(|p| p.text.clone())
                .collect(),
            typo_corrections: candidates
                .typos
                .iter()
                .map(|t| shared::proto::TypoCandidate {
                    text: t.text.clone(),
                    hiragana: t.hiragana.clone(),
                })
                .collect(),
        });
        self.runtime
            .clone()
            .block_on(self.window_client.set_candidate(request))?;

        Ok(())
    }

    #[tracing::instrument]
    pub fn set_selection(
        &mut self,
        kind: shared::proto::SelectionKind,
        index: i32,
    ) -> anyhow::Result<()> {
        let request = tonic::Request::new(shared::proto::SetSelectionRequest {
            index,
            kind: kind as i32,
        });
        self.runtime
            .clone()
            .block_on(self.window_client.set_selection(request))?;

        Ok(())
    }

    #[tracing::instrument]
    pub fn set_input_mode(&mut self, mode: &str) -> anyhow::Result<()> {
        let request = tonic::Request::new(shared::proto::SetInputModeRequest {
            mode: mode.to_string(),
        });
        self.runtime
            .clone()
            .block_on(self.window_client.set_input_mode(request))?;

        Ok(())
    }
}
