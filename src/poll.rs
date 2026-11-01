//! Фоновый опрос API.
//!
//! Поток спит `interval`, просыпается по таймеру или по ручному обновлению
//! из меню, забирает снимок и обновляет трей. При ошибках пауза растёт
//! 60 → 120 → 240 → 300 с, при успехе возвращается к `interval`.

use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::Arc;
use std::time::Duration;

use ksni::blocking::Handle;

use crate::source::Source;
use crate::state::Status;
use crate::tray::CrTray;

/// Максимальная пауза при ошибках.
pub const MAX_BACKOFF: Duration = Duration::from_secs(300);

/// Зачем поток опроса разбудили.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wake {
    /// Ручное обновление в меню: забрать данные вне очереди, включая почту.
    Refresh,
    /// «Выход» в меню: закончить работу штатно, чтобы снять лок-файл.
    Quit,
}

/// Пауза до следующего опроса.
pub fn next_delay(current: Duration, interval: Duration, ok: bool) -> Duration {
    if ok {
        interval
    } else {
        (current * 2).clamp(interval, MAX_BACKOFF)
    }
}

/// Крутит опрос, пока трей обслуживается. `wake` приходит из меню.
pub fn run(
    src: Arc<dyn Source>,
    handle: Handle<CrTray>,
    interval: Duration,
    wake: Receiver<Wake>,
) {
    let mut delay = interval;
    loop {
        let manual = match wake.recv_timeout(delay) {
            Ok(Wake::Refresh) => true,
            Ok(Wake::Quit) => return,
            Err(RecvTimeoutError::Timeout) => false,
            Err(RecvTimeoutError::Disconnected) => return,
        };

        let snap = if manual {
            src.refresh()
        } else {
            src.snapshot()
        };
        let ok = snap.status != Status::Error;
        let status = snap.status;
        let updated = snap.updated.clone();
        if handle
            .update(move |tray: &mut CrTray| tray.snap = snap)
            .is_none()
        {
            eprintln!("cr-tray: трей больше не обслуживается, выходим");
            return;
        }
        eprintln!("cr-tray: обновлено в {updated} ({status})");

        delay = next_delay(delay, interval, ok);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    #[test]
    fn success_returns_to_interval() {
        assert_eq!(next_delay(secs(300), secs(60), true), secs(60));
        assert_eq!(next_delay(secs(60), secs(30), true), secs(30));
    }

    #[test]
    fn errors_grow_and_cap() {
        let i = secs(60);
        assert_eq!(next_delay(secs(60), i, false), secs(120));
        assert_eq!(next_delay(secs(120), i, false), secs(240));
        assert_eq!(next_delay(secs(240), i, false), secs(300));
        assert_eq!(next_delay(secs(300), i, false), secs(300));
    }

    #[test]
    fn backoff_never_below_interval() {
        assert_eq!(next_delay(secs(60), secs(120), false), secs(120));
    }
}
