use std::fmt;

/// Состояние трея, влияет на цвет иконки.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Всё в порядке.
    Ok,
    /// Данные есть, но баланс ниже порога.
    Warn,
    /// Последнее обновление не удалось, показаны устаревшие данные.
    Error,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Status::Ok => "норма",
            Status::Warn => "мало средств",
            Status::Error => "ошибка обновления",
        }
    }
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// Снимок состояния счёта, который отображает трей.
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub spent_rub: f64,
    pub spent_usd: f64,
    pub balance_rub: f64,
    pub balance_usd: f64,
    pub requests: u64,
    pub tokens: u64,
    /// Курс ЦБ, справочно.
    pub rate: f64,
    pub rate_date: String,
    /// Курс, по которому реально списываются деньги при оплате.
    pub pay_rate: f64,
    /// Наценка к ЦБ в процентах; `None` — курс задан фиксированным числом.
    pub pay_markup: Option<f64>,
    pub updated: String,
    pub status: Status,
    pub error: Option<String>,
}

/// Наценка CloseRouter к курсу ЦБ по умолчанию, процентов.
pub const DEFAULT_MARKUP_PERCENT: f64 = 12.0;

/// Порог «мало средств» по умолчанию, ₽. `0` — не предупреждать.
pub const DEFAULT_WARN_BALANCE_RUB: f64 = 50.0;

/// Статус по балансу: `Warn`, если порог задан и денег меньше него.
pub fn balance_status(balance_rub: f64, warn_rub: f64) -> Status {
    if warn_rub > 0.0 && balance_rub < warn_rub {
        Status::Warn
    } else {
        Status::Ok
    }
}

/// Курс списания: фиксированный, если задан, иначе ЦБ с наценкой.
pub fn resolve_pay_rate(cbr: f64, fixed: Option<f64>, markup_percent: f64) -> f64 {
    fixed.unwrap_or_else(|| cbr * (1.0 + markup_percent / 100.0))
}

impl Snapshot {
    /// Пустой снимок: данных ещё не было, показать нечего.
    pub fn empty(status: Status, error: Option<String>) -> Self {
        Self {
            spent_rub: 0.0,
            spent_usd: 0.0,
            balance_rub: 0.0,
            balance_usd: 0.0,
            requests: 0,
            tokens: 0,
            rate: 0.0,
            rate_date: "—".into(),
            pay_rate: 0.0,
            pay_markup: None,
            updated: "—".into(),
            status,
            error,
        }
    }

    /// То же, но с настраиваемым курсом списания.
    pub fn demo_with(status: Status, fixed_rate: Option<f64>, markup_percent: f64) -> Self {
        const SPENT_USD: f64 = 0.10950569;
        const BALANCE_USD: f64 = 2.79683798;
        const CBR_RATE: f64 = 80.5573;

        let pay_rate = resolve_pay_rate(CBR_RATE, fixed_rate, markup_percent);
        Self {
            spent_rub: SPENT_USD * pay_rate,
            spent_usd: SPENT_USD,
            balance_rub: BALANCE_USD * pay_rate,
            balance_usd: BALANCE_USD,
            requests: 204,
            tokens: 54_008_392,
            rate: CBR_RATE,
            rate_date: "31.10.2026".into(),
            pay_rate,
            pay_markup: fixed_rate.is_none().then_some(markup_percent),
            updated: "16:12".into(),
            status,
            error: match status {
                Status::Error => Some("нет связи с api.closerouter.dev".into()),
                _ => None,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pay_rate_prefers_fixed_value() {
        assert_eq!(resolve_pay_rate(80.0, Some(92.5), 12.0), 92.5);
    }

    #[test]
    fn pay_rate_applies_markup() {
        let rate = resolve_pay_rate(80.0, None, 12.0);
        assert!((rate - 89.6).abs() < 1e-9);
    }

    #[test]
    fn demo_uses_pay_rate_for_rubles() {        let s = Snapshot::demo_with(Status::Ok, None, DEFAULT_MARKUP_PERCENT);
        assert!((s.pay_rate - 80.5573 * 1.12).abs() < 1e-9);
        assert!((s.spent_rub - s.spent_usd * s.pay_rate).abs() < 1e-9);
        assert_eq!(s.pay_markup, Some(12.0));
    }

    #[test]
    fn balance_below_threshold_is_warn() {
        assert_eq!(balance_status(49.99, 50.0), Status::Warn);
        assert_eq!(balance_status(50.0, 50.0), Status::Ok);
        assert_eq!(balance_status(243.55, 50.0), Status::Ok);
    }

    #[test]
    fn zero_threshold_disables_warning() {
        assert_eq!(balance_status(0.0, 0.0), Status::Ok);
        assert_eq!(balance_status(1.0, 0.0), Status::Ok);
    }
}