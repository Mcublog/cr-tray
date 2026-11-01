//! Форматирование чисел для иконки, тултипа и меню.

/// Число с запятой в качестве десятичного разделителя.
pub fn num(value: f64, decimals: usize) -> String {
    format!("{value:.decimals$}").replace('.', ",")
}

/// Рубли с двумя знаками: `8,82`.
pub fn rub(value: f64) -> String {
    num(value, 2)
}

/// Доллары с четырьмя знаками: `$0,1095`.
pub fn usd(value: f64) -> String {
    format!("${}", num(value, 4))
}

/// Доллары с двумя знаками: `$2,80`.
pub fn usd_short(value: f64) -> String {
    format!("${}", num(value, 2))
}

/// Компактное число токенов: `54,0M`, `934,2k`, `512`.
pub fn tokens(value: u64) -> String {
    if value >= 1_000_000 {
        format!("{}M", num(value as f64 / 1e6, 1))
    } else if value >= 10_000 {
        format!("{}k", num(value as f64 / 1e3, 0))
    } else if value >= 1_000 {
        format!("{}k", num(value as f64 / 1e3, 1))
    } else {
        value.to_string()
    }
}

/// Русское склонение: `1 запрос`, `204 запроса`, `5 запросов`.
pub fn plural_ru(value: u64, one: &str, few: &str, many: &str) -> String {
    let n = value % 100;
    if (11..=14).contains(&n) {
        return format!("{value} {many}");
    }
    match value % 10 {
        1 => format!("{value} {one}"),
        2..=4 => format!("{value} {few}"),
        _ => format!("{value} {many}"),
    }
}

/// Подпись для иконки: сумма за сегодня в рублях, максимально коротко.
///
/// На 22 px больше четырёх знаков уже не читаются, поэтому крупные суммы
/// сжимаем: `12345` → `12k`, `1 234 567` → `1,2M`.
pub fn icon_label(spent_rub: f64) -> String {
    if spent_rub >= 1_000_000.0 {
        format!("{}M", num(spent_rub / 1e6, 1))
    } else if spent_rub >= 10_000.0 {
        format!("{}k", num(spent_rub / 1e3, 0))
    } else if spent_rub < 10.0 {
        num(spent_rub, 1)
    } else {
        num(spent_rub, 0)
    }
}

/// Наценка к ЦБ: `+12% к ЦБ` или `фиксированный`.
pub fn markup(markup_percent: Option<f64>) -> String {
    match markup_percent {
        Some(p) => format!("+{}% к ЦБ", num(p, 0)),
        None => "фиксированный".into(),
    }
}

/// Курс списания строкой: `90,22 ₽/$ (+12% к ЦБ)`; `—`, пока данных нет.
pub fn pay_rate(rate: f64, markup_percent: Option<f64>) -> String {
    if rate > 0.0 {
        format!("{} ₽/$ ({})", num(rate, 2), markup(markup_percent))
    } else {
        "—".into()
    }
}

/// Курс ЦБ строкой: `80,5573 (31.10.2026)`; `—`, пока данных нет.
pub fn cbr_rate(rate: f64, date: &str) -> String {
    if rate > 0.0 {
        format!("{} ({})", num(rate, 4), date)
    } else {
        "—".into()
    }
}

/// Строка ящика для меню: `me@mail.ru: 3`, при сбое — `· ошибка`.
pub fn mail_line(mailbox: &crate::state::MailboxCount) -> String {
    match (mailbox.unread, mailbox.error.is_some()) {
        (Some(unread), false) => format!("{}: {unread}", mailbox.name),
        (Some(unread), true) => format!("{}: {unread} · ошибка", mailbox.name),
        (None, true) => format!("{}: — · ошибка", mailbox.name),
        (None, false) => format!("{}: ?", mailbox.name),
    }
}

/// Многострочный текстовый отчёт для `--once` и журнала.
pub fn report(s: &crate::state::Snapshot) -> String {
    let mut out = format!(
        "CloseRouter\n\
         \x20 Сегодня:      {} ₽ ({})\n\
         \x20 Запросы:      {} · {} токенов\n\
         \x20 Баланс:       {} ₽ ({})\n\
         \x20 Курс оплаты:  {}\n\
         \x20 Курс ЦБ:      {}\n\
         \x20 Обновлено:    {}\n\
         \x20 Состояние:    {}",
        rub(s.spent_rub),
        usd(s.spent_usd),
        plural_ru(s.requests, "запрос", "запроса", "запросов"),
        tokens(s.tokens),
        rub(s.balance_rub),
        usd_short(s.balance_usd),
        pay_rate(s.pay_rate, s.pay_markup),
        cbr_rate(s.rate, &s.rate_date),
        s.updated,
        s.status.label(),
    );
    if let Some(err) = &s.error {
        out.push_str(&format!("\n  Ошибка:       {err}"));
    }
    if !s.mailboxes.is_empty() {
        out.push_str("\n  Почта:");
        for mailbox in &s.mailboxes {
            out.push_str(&format!("\n    {}", mail_line(mailbox)));
            if let Some(err) = &mailbox.error {
                out.push_str(&format!(" ({err})"));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rubles_use_comma() {
        assert_eq!(rub(8.82), "8,82");
        assert_eq!(rub(225.3), "225,30");
        assert_eq!(rub(0.0), "0,00");
    }

    #[test]
    fn dollars_use_comma_and_four_digits() {
        assert_eq!(usd(0.10950569), "$0,1095");
        assert_eq!(usd_short(2.79683798), "$2,80");
    }

    #[test]
    fn tokens_are_compact() {
        assert_eq!(tokens(54_008_392), "54,0M");
        assert_eq!(tokens(12_340), "12k");
        assert_eq!(tokens(1_500), "1,5k");
        assert_eq!(tokens(512), "512");
    }

    #[test]
    fn plural_is_correct() {
        assert_eq!(plural_ru(1, "запрос", "запроса", "запросов"), "1 запрос");
        assert_eq!(plural_ru(2, "запрос", "запроса", "запросов"), "2 запроса");
        assert_eq!(
            plural_ru(204, "запрос", "запроса", "запросов"),
            "204 запроса"
        );
        assert_eq!(
            plural_ru(11, "запрос", "запроса", "запросов"),
            "11 запросов"
        );
        assert_eq!(plural_ru(0, "запрос", "запроса", "запросов"), "0 запросов");
    }

    #[test]
    fn icon_label_is_short() {
        assert_eq!(icon_label(8.82), "8,8");
        assert_eq!(icon_label(9.99), "10,0");
        assert_eq!(icon_label(23.4), "23");
        assert_eq!(icon_label(226.0), "226");
        assert_eq!(icon_label(9999.0), "9999");
    }

    #[test]
    fn icon_label_compacts_large_sums() {
        assert_eq!(icon_label(10_000.0), "10k");
        assert_eq!(icon_label(12_345.0), "12k");
        assert_eq!(icon_label(99_499.0), "99k");
        assert_eq!(icon_label(1_234_567.0), "1,2M");
    }

    #[test]
    fn rates_show_dash_without_data() {
        assert_eq!(pay_rate(0.0, None), "—");
        assert_eq!(pay_rate(0.0, Some(12.0)), "—");
        assert_eq!(cbr_rate(0.0, "—"), "—");
    }

    #[test]
    fn rate_lines_are_readable() {
        assert_eq!(pay_rate(90.224176, Some(12.0)), "90,22 ₽/$ (+12% к ЦБ)");
        assert_eq!(pay_rate(92.5, None), "92,50 ₽/$ (фиксированный)");
        assert_eq!(cbr_rate(80.5573, "31.10.2026"), "80,5573 (31.10.2026)");
    }

    #[test]
    fn report_contains_key_lines() {
        let s = crate::state::Snapshot::demo_with(crate::state::Status::Ok, None, 12.0);
        let text = report(&s);
        assert!(text.contains("Сегодня:      9,88 ₽ ($0,1095)"), "{text}");
        assert!(text.contains("204 запроса · 54,0M токенов"), "{text}");
        assert!(
            text.contains("Курс оплаты:  90,22 ₽/$ (+12% к ЦБ)"),
            "{text}"
        );
        assert!(
            text.contains("Курс ЦБ:      80,5573 (31.10.2026)"),
            "{text}"
        );
        assert!(text.contains("Состояние:    норма"), "{text}");
        assert!(!text.contains("Ошибка"), "{text}");
    }

    #[test]
    fn mail_lines_show_counts_and_errors() {
        use crate::state::MailboxCount;
        let ok = MailboxCount {
            name: "me@mail.ru".into(),
            unread: Some(3),
            error: None,
            url: None,
        };
        assert_eq!(mail_line(&ok), "me@mail.ru: 3");

        let stale = MailboxCount {
            name: "Работа".into(),
            unread: Some(12),
            error: Some("таймаут".into()),
            url: None,
        };
        assert_eq!(mail_line(&stale), "Работа: 12 · ошибка");

        let failed = MailboxCount {
            name: "me@yandex.ru".into(),
            unread: None,
            error: Some("неверный пароль".into()),
            url: None,
        };
        assert_eq!(mail_line(&failed), "me@yandex.ru: — · ошибка");
    }

    #[test]
    fn report_lists_every_mailbox() {
        let s = crate::state::Snapshot::demo_with(crate::state::Status::Ok, None, 12.0);
        let text = report(&s);
        assert!(text.contains("Почта:"), "{text}");
        assert!(text.contains("me@mail.ru: 3"), "{text}");
        assert!(text.contains("me@yandex.ru: 12"), "{text}");
        assert!(text.contains("Архив: 0"), "{text}");
    }

    #[test]
    fn report_shows_error_line() {
        let s = crate::state::Snapshot::demo_with(crate::state::Status::Error, None, 12.0);
        let text = report(&s);
        assert!(text.contains("Состояние:    ошибка обновления"), "{text}");
        assert!(text.contains("Ошибка:"), "{text}");
    }
}
