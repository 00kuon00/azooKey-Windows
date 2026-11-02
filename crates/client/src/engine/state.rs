use std::{
    collections::HashMap,
    sync::{LazyLock, Mutex, MutexGuard},
    time::{Duration, Instant},
};

use windows::{core::GUID, Win32::UI::TextServices::ITfContext};

use super::{input_mode::InputMode, ipc_service::IPCService};

#[derive(Debug)]
pub struct IMEState {
    pub ipc_service: Option<IPCService>,
    pub reconnect: Reconnect,
    pub input_mode: InputMode,
    pub cookies: HashMap<GUID, u32>,
    pub context: Option<ITfContext>,
}

pub static IME_STATE: LazyLock<Mutex<IMEState>> = LazyLock::new(|| {
    tracing::debug!("Creating IMEState");
    Mutex::new(IMEState {
        ipc_service: None,
        reconnect: Reconnect::default(),
        input_mode: InputMode::default(),
        cookies: HashMap::new(),
        context: None,
    })
});
unsafe impl Sync for IMEState {}
unsafe impl Send for IMEState {}

impl IMEState {
    pub fn get() -> anyhow::Result<MutexGuard<'static, IMEState>> {
        match IME_STATE.try_lock() {
            Ok(guard) => Ok(guard),
            Err(e) => anyhow::bail!("Failed to lock state: {:?}", e),
        }
    }

    // 変換エンジンにつながっていなければ、間隔を空けてつなぎ直してから返す（#36）。
    // 呼ぶのはキーの処理の入口だけ。編集セッションの中（update_pos・surrounded_text）では
    // つなぎにいかず、今までどおり「無ければ飛ばす」
    pub fn ipc_service_or_reconnect() -> anyhow::Result<Option<IPCService>> {
        let mut guard = Self::get()?;
        let state = &mut *guard;
        state
            .reconnect
            .ensure(&mut state.ipc_service, Instant::now(), IPCService::connect);
        Ok(state.ipc_service.clone())
    }
}

// 変換エンジンへの接続を、つながっていないときだけ・間隔を空けて試す
#[derive(Debug, Default)]
pub struct Reconnect {
    last_attempt: Option<Instant>,
}

impl Reconnect {
    // 試す間隔の下限。IPCService::new() は試すたびに tokio のランタイム（この PC で 32 スレッド）を
    // 作って捨て、それをアプリの UI スレッドで行うので、打鍵ごとには試さない。
    // 一方で、エンジンがパイプを作ってから次に押したキーで（長くても 1 秒待てば）つながってほしい。
    // 1 秒より短くしても、連打の間に作って捨てるランタイムが増えるだけになる
    pub const INTERVAL: Duration = Duration::from_secs(1);

    // 接続を試した時刻を残す（Activate で試して失敗したときなど）
    pub fn record_attempt(&mut self, now: Instant) {
        self.last_attempt = Some(now);
    }

    // slot が空で、前に試してから INTERVAL 以上たっていれば connect を試し、つながれば入れる
    pub fn ensure<T>(
        &mut self,
        slot: &mut Option<T>,
        now: Instant,
        connect: impl FnOnce() -> Option<T>,
    ) {
        if slot.is_some() {
            return;
        }

        if let Some(last_attempt) = self.last_attempt {
            if now.saturating_duration_since(last_attempt) < Self::INTERVAL {
                return;
            }
        }

        self.record_attempt(now);
        *slot = connect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    // ① 無い・つながらない → None のまま
    #[test]
    fn failed_connect_leaves_none() {
        let mut reconnect = Reconnect::default();
        let mut slot: Option<i32> = None;
        reconnect.ensure(&mut slot, Instant::now(), || None);
        assert_eq!(slot, None);
    }

    // ② 前に試してから間隔の内側なら、接続を試さない
    #[test]
    fn does_not_retry_within_interval() {
        let mut reconnect = Reconnect::default();
        let mut slot: Option<i32> = None;
        let start = Instant::now();
        reconnect.ensure(&mut slot, start, || None);
        reconnect.ensure(
            &mut slot,
            start + Reconnect::INTERVAL - Duration::from_millis(1),
            || panic!("間隔の内側で接続を試した"),
        );
        assert_eq!(slot, None);
    }

    // ③ 間隔を過ぎたら試し、つながれば入る
    #[test]
    fn retries_after_interval_and_stores_connection() {
        let mut reconnect = Reconnect::default();
        let mut slot: Option<i32> = None;
        let start = Instant::now();
        reconnect.ensure(&mut slot, start, || None);
        reconnect.ensure(&mut slot, start + Reconnect::INTERVAL, || Some(1));
        assert_eq!(slot, Some(1));
    }

    // ④ 入っていれば接続の関数を呼ばない(前に試した時刻にも関わらない)
    #[test]
    fn existing_connection_is_kept() {
        let mut reconnect = Reconnect::default();
        let mut slot = Some(1);
        reconnect.ensure(&mut slot, Instant::now(), || {
            panic!("つながっているのに試した")
        });
        assert_eq!(slot, Some(1));
    }

    // Activate で試して失敗した時刻も「前に試した時刻」に数える
    #[test]
    fn recorded_attempt_counts_toward_interval() {
        let mut reconnect = Reconnect::default();
        let mut slot: Option<i32> = None;
        let start = Instant::now();
        reconnect.record_attempt(start);
        reconnect.ensure(&mut slot, start, || panic!("間隔の内側で接続を試した"));
        assert_eq!(slot, None);
    }
}
