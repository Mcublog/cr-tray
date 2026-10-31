//! Источники снимка состояния.
//!
//! Трей и `--once` работают через трейт [`Source`], поэтому заглушка `Demo`
//! и реальный клиент `ApiSource` взаимозаменяемы без правок UI.

use std::sync::Mutex;

use crate::api::{self, Credits, Usage};
use crate::cbr::{self, Rate};
use crate::clock;
use crate::state::{balance_status, resolve_pay_rate, Snapshot, Status};

/// Что-то, что умеет отдать текущий снимок состояния счёта.
pub trait Source: Send + Sync {
    fn snapshot(&self) -> Snapshot;
}

/// Заглушка на фейковых данных (этапы 1–2).
pub struct Demo {
    pub status: Status,
    pub fixed_rate: Option<f64>,
    pub markup: f64,
}

impl Demo {
    pub fn new(status: Status, fixed_rate: Option<f64>, markup: f64) -> Self {
        Self {
            status,
            fixed_rate,
            markup,
        }
    }

    pub fn snapshot_for(&self, status: Status) -> Snapshot {
        Snapshot::demo_with(status, self.fixed_rate, self.markup)
    }
}

impl Source for Demo {
    fn snapshot(&self) -> Snapshot {
        self.snapshot_for(self.status)
    }
}

/// Реальные данные: `/v1/usage`, `/v1/credits` и курс ЦБ.
pub struct ApiSource {
    api: api::Client,
    cbr: cbr::Cbr,
    tz: String,
    fixed_rate: Option<f64>,
    markup: f64,
    /// Порог «мало средств», ₽; 0 — не предупреждать.
    warn_rub: f64,
    /// Последний удачный снимок: при ошибке показываем его же, но с ошибкой.
    last: Mutex<Option<Snapshot>>,
}

impl ApiSource {
    pub fn new(
        key: impl Into<String>,
        tz: impl Into<String>,
        fixed_rate: Option<f64>,
        markup: f64,
        warn_rub: f64,
    ) -> Self {
        Self {
            api: api::Client::new(key),
            cbr: cbr::Cbr::new(),
            tz: tz.into(),
            fixed_rate,
            markup,
            warn_rub,
            last: Mutex::new(None),
        }
    }

    fn build(&self) -> Result<Snapshot, String> {
        let bounds = clock::day_bounds(&self.tz, jiff::Timestamp::now())?;

        // Два независимых запроса — параллельно, чтобы опрос занимал один таймаут.
        let (usage, credits) = std::thread::scope(|s| {
            let usage = s.spawn(|| self.api.usage(&bounds.start, &bounds.end, &self.tz));
            let credits = s.spawn(|| self.api.credits());
            (usage.join(), credits.join())
        });
        let usage = join(usage)?;
        let credits = join(credits)?;
        let rate = self.cbr.rate(&bounds.day)?;

        Ok(build_snapshot(
            usage,
            credits,
            rate,
            &bounds.time,
            self.fixed_rate,
            self.markup,
            self.warn_rub,
        ))
    }
}

fn join<T>(result: std::thread::Result<Result<T, api::ApiError>>) -> Result<T, String> {
    match result {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => Err("поток запроса аварийно завершился".into()),
    }
}

impl Source for ApiSource {
    fn snapshot(&self) -> Snapshot {
        match self.build() {
            Ok(snap) => {
                *self.last.lock().unwrap() = Some(snap.clone());
                snap
            }
            Err(e) => {
                eprintln!("cr-tray: обновление не удалось: {e}");
                let mut last = self.last.lock().unwrap();
                match last.as_mut() {
                    Some(prev) => {
                        prev.status = Status::Error;
                        prev.error = Some(e);
                        prev.clone()
                    }
                    None => Snapshot::empty(Status::Error, Some(e)),
                }
            }
        }
    }
}

/// Собирает снимок из ответов API и курса ЦБ.
fn build_snapshot(
    usage: Usage,
    credits: Credits,
    rate: Rate,
    updated: &str,
    fixed_rate: Option<f64>,
    markup: f64,
    warn_rub: f64,
) -> Snapshot {
    let pay_rate = resolve_pay_rate(rate.value, fixed_rate, markup);
    let balance_rub = credits.total * pay_rate;
    Snapshot {
        spent_rub: usage.cost * pay_rate,
        spent_usd: usage.cost,
        balance_rub,
        balance_usd: credits.total,
        requests: usage.requests,
        tokens: usage.tokens,
        rate: rate.value,
        rate_date: rate.date,
        pay_rate,
        pay_markup: fixed_rate.is_none().then_some(markup),
        updated: updated.to_string(),
        status: balance_status(balance_rub, warn_rub),
        error: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_source_returns_snapshot() {
        let src = Demo::new(Status::Ok, Some(90.0), 12.0);
        let snap = src.snapshot();
        assert_eq!(snap.pay_rate, 90.0);
        assert_eq!(snap.pay_markup, None);
        assert_eq!(snap.status, Status::Ok);
    }

    #[test]
    fn demo_source_keeps_requested_status() {
        let src = Demo::new(Status::Error, None, 12.0);
        let snap = src.snapshot();
        assert_eq!(snap.status, Status::Error);
        assert!(snap.error.is_some());
    }

    #[test]
    fn demo_source_can_override_status() {
        let src = Demo::new(Status::Ok, None, 12.0);
        assert_eq!(src.snapshot_for(Status::Warn).status, Status::Warn);
    }

    #[test]
    fn demo_source_is_usable_as_trait_object() {
        let src: Box<dyn Source> = Box::new(Demo::new(Status::Ok, None, 12.0));
        assert_eq!(src.snapshot().status, Status::Ok);
    }

    #[test]
    fn snapshot_scales_money_by_pay_rate() {
        let snap = build_snapshot(
            Usage {
                cost: 0.20293022,
                requests: 310,
                tokens: 65_019_461,
            },
            Credits {
                total: 2.70341345,
                used: 0.24088051,
            },
            Rate {
                value: 80.5573,
                date: "31.10.2026".into(),
            },
            "16:12",
            None,
            12.0,
            50.0,
        );
        let pay = 80.5573f64 * 1.12;
        assert!((snap.pay_rate - pay).abs() < 1e-9);
        assert!((snap.spent_rub - 0.20293022 * pay).abs() < 1e-9);
        assert!((snap.balance_rub - 2.70341345 * pay).abs() < 1e-9);
        assert_eq!(snap.requests, 310);
        assert_eq!(snap.tokens, 65_019_461);
        assert_eq!(snap.updated, "16:12");
        assert_eq!(snap.status, Status::Ok);
        assert_eq!(snap.pay_markup, Some(12.0));
    }

    #[test]
    fn fixed_rate_overrides_markup() {
        let snap = build_snapshot(
            Usage::default(),
            Credits::default(),
            Rate {
                value: 80.0,
                date: "31.10.2026".into(),
            },
            "10:00",
            Some(92.5),
            12.0,
            50.0,
        );
        assert_eq!(snap.pay_rate, 92.5);
        assert_eq!(snap.pay_markup, None);
    }

    #[test]
    fn low_balance_turns_status_yellow() {
        let snap = build_snapshot(
            Usage::default(),
            Credits {
                total: 0.5,
                used: 0.0,
            },
            Rate {
                value: 80.0,
                date: "31.10.2026".into(),
            },
            "10:00",
            None,
            12.0,
            50.0,
        );
        // 0,5 $ × 89,6 = 44,8 ₽ < 50 ₽.
        assert_eq!(snap.status, Status::Warn);
        assert!(snap.error.is_none());
    }

    #[test]
    fn threshold_zero_keeps_status_green() {
        let snap = build_snapshot(
            Usage::default(),
            Credits {
                total: 0.1,
                used: 0.0,
            },
            Rate {
                value: 80.0,
                date: "31.10.2026".into(),
            },
            "10:00",
            None,
            12.0,
            0.0,
        );
        assert_eq!(snap.status, Status::Ok);
    }
}