//! Реализация StatusNotifierItem: иконка, тултип и меню.

use std::process::Command;
use std::sync::mpsc::Sender;

use ksni::menu::{Disposition, StandardItem};
use ksni::{Category, Icon, MenuItem, ToolTip, Tray};

use crate::poll::Wake;
use crate::{format, icon, state::Snapshot};

pub struct CrTray {
    pub snap: Snapshot,
    /// Будит поток опроса: «Обновить» и «Выход» в меню.
    pub wake: Option<Sender<Wake>>,
}

impl CrTray {
    pub fn new(snap: Snapshot) -> Self {
        Self { snap, wake: None }
    }
}

fn info<T>(label: String) -> MenuItem<T> {
    StandardItem {
        label,
        enabled: false,
        ..Default::default()
    }
    .into()
}

impl Tray for CrTray {
    // Левый клик по иконке открывает меню: на Wayland Activate приходит не всегда.
    const MENU_ON_ACTIVATE: bool = true;

    fn id(&self) -> String {
        "cr-tray".into()
    }

    fn title(&self) -> String {
        "CloseRouter".into()
    }

    fn category(&self) -> Category {
        Category::ApplicationStatus
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        icon::icons(&self.snap)
    }

    fn tool_tip(&self) -> ToolTip {
        let s = &self.snap;
        let mut description = format!(
            "Сегодня: {} ₽ ({})<br/>\
             {} · {}<br/>\
             Баланс: {} ₽ ({})<br/>\
             Курс оплаты: {}<br/>\
             Курс ЦБ: {}<br/>\
             Обновлено: {}",
            format::rub(s.spent_rub),
            format::usd(s.spent_usd),
            format::plural_ru(s.requests, "запрос", "запроса", "запросов"),
            format::tokens(s.tokens),
            format::rub(s.balance_rub),
            format::usd_short(s.balance_usd),
            format::pay_rate(s.pay_rate, s.pay_markup),
            format::cbr_rate(s.rate, &s.rate_date),
            s.updated,
        );
        if let Some(err) = &s.error {
            description.push_str(&format!("<br/><b>Ошибка:</b> {err}"));
        }

        ToolTip {
            title: format!(
                "CloseRouter · {} ₽ за сегодня",
                format::rub(s.spent_rub)
            ),
            description,
            ..Default::default()
        }
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let s = &self.snap;
        let mut items = vec![
            info(format!(
                "Сегодня: {} ₽ ({})",
                format::rub(s.spent_rub),
                format::usd(s.spent_usd)
            )),
            info(format!(
                "{} · {}",
                format::plural_ru(s.requests, "запрос", "запроса", "запросов"),
                format::tokens(s.tokens)
            )),
            info(format!(
                "Баланс: {} ₽ ({})",
                format::rub(s.balance_rub),
                format::usd_short(s.balance_usd)
            )),
            info(format!(
                "Курс оплаты: {}",
                format::pay_rate(s.pay_rate, s.pay_markup)
            )),
            info(if s.rate > 0.0 {
                format!("Курс ЦБ: {} · {}", format::num(s.rate, 4), s.rate_date)
            } else {
                "Курс ЦБ: —".to_string()
            }),
            info(format!("Обновлено: {} · {}", s.updated, s.status.label())),
            MenuItem::Separator,
            StandardItem {
                label: "Обновить".into(),
                activate: Box::new(|tray: &mut CrTray| {
                    if let Some(tx) = &tray.wake {
                        // Поток опроса проснётся сразу и заберёт свежие данные.
                        let _ = tx.send(Wake::Refresh);
                    }
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Открыть дашборд".into(),
                activate: Box::new(|_: &mut CrTray| {
                    let _ = Command::new("xdg-open")
                        .arg("https://closerouter.dev/usage")
                        .spawn();
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Выход".into(),
                disposition: Disposition::Informative,
                activate: Box::new(|tray: &mut CrTray| match &tray.wake {
                    // Штатный выход: main вернётся из poll::run и снимет лок-файл.
                    // Небольшая задержка, чтобы ksni успел ответить на вызов меню.
                    Some(tx) => {
                        let tx = tx.clone();
                        std::thread::spawn(move || {
                            std::thread::sleep(std::time::Duration::from_millis(300));
                            let _ = tx.send(Wake::Quit);
                        });
                    }
                    None => std::process::exit(0),
                }),
                ..Default::default()
            }
            .into(),
        ];
        items.shrink_to_fit();
        items
    }
}