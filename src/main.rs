//! cr-tray — расход CloseRouter за сегодня в системном трее KDE Plasma.

mod api;
mod cbr;
mod clock;
mod format;
mod icon;
mod links;
mod lock;
mod mail;
mod poll;
mod source;
mod state;
mod tray;

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use ksni::blocking::TrayMethods;

use crate::source::Source;
use crate::state::Status;
use crate::tray::CrTray;

const HELP: &str = "\
cr-tray — расход CloseRouter за сегодня в трее KDE Plasma

Использование:
  cr-tray [--once] [--api-key KEY] [--timezone TZ] [--interval SEC]
          [--pay-rate RUB] [--pay-markup PCT]
          [--mail-config PATH] [--mail-interval SEC] [--no-mail]
          [--links-config PATH]
  cr-tray --demo [--demo-state ok|warn|error] [--demo-cycle] [--dump-icons DIR]

Флаги:
  --once             напечатать снимок в stdout и выйти (без трея)
  --api-key <KEY>    API-ключ CloseRouter (приоритетнее окружения)
  --timezone <TZ>    таймзона для границ дня (по умолчанию системная)
  --interval <SEC>   период опроса API, секунды (по умолчанию 60)
  --pay-rate <RUB>   курс списания CloseRouter, ₽ за $ (приоритетнее наценки)
  --pay-markup <PCT> наценка к курсу ЦБ в процентах (по умолчанию 12)
  --warn-balance-rub <RUB>  жёлтая иконка при балансе ниже порога
                             (по умолчанию 50, 0 — выключить)
  --mail-config <PATH>      конфиг почты (по умолчанию ~/.config/cr-tray/mail.json)
  --mail-interval <SEC>     период опроса IMAP, секунды (по умолчанию 300)
  --no-mail                 не опрашивать почту
  --links-config <PATH>     конфиг ссылок в меню (по умолчанию ~/.config/cr-tray/links.json)
  --demo             фейковые данные, без сети и ключа
  --demo-state <S>   фиксированное состояние иконки: ok, warn, error
  --demo-cycle       переключать состояния каждые 5 секунд
  --dump-icons <DIR> сохранить пиксмапы иконок в PPM и выйти
  -h, --help         эта справка

Окружение:
  CLOSEROUTER_API_KEY — API-ключ, если не задан флагом
  CR_PAY_RATE, CR_PAY_MARKUP — курс списания, флаги приоритетнее
  CR_INTERVAL — период опроса, секунды
  CR_WARN_BALANCE_RUB — порог «мало средств», ₽
  CR_MAIL_CONFIG — путь к конфигу почты
  CR_MAIL_INTERVAL — период опроса IMAP, секунды
  CR_LINKS_CONFIG — путь к конфигу ссылок в меню

Ключ также читается из ~/.config/cr-tray/api_key.
Почта — из ~/.config/cr-tray/mail.json (см. README).
";

struct Args {
    /// Демо-режим: без сети и API-ключа.
    demo: bool,
    state: Status,
    cycle: bool,
    once: bool,
    dump: Option<PathBuf>,
    api_key: Option<String>,
    timezone: String,
    /// Период опроса API, секунды.
    interval: u64,
    /// Фиксированный курс списания, ₽ за $.
    pay_rate: Option<f64>,
    /// Наценка к курсу ЦБ, процентов.
    pay_markup: f64,
    /// Порог «мало средств», ₽; 0 — не предупреждать.
    warn_rub: f64,
    /// Явный путь к конфигу почты.
    mail_config: Option<PathBuf>,
    /// Период опроса почты; `None` — из конфига или по умолчанию.
    mail_interval: Option<u64>,
    /// Полностью выключить почту.
    no_mail: bool,
    /// Явный путь к конфигу ссылок в меню.
    links_config: Option<PathBuf>,
}

/// Минимальный период опроса: чаще незачем, API можно и притормозить.
const MIN_INTERVAL: u64 = 5;

fn system_timezone() -> String {
    jiff::tz::TimeZone::system()
        .iana_name()
        .map(|name| name.to_string())
        .unwrap_or_else(|| "Europe/Moscow".into())
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        demo: false,
        state: Status::Ok,
        cycle: false,
        once: false,
        dump: None,
        api_key: None,
        timezone: system_timezone(),
        interval: std::env::var("CR_INTERVAL")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(60)
            .max(MIN_INTERVAL),
        pay_rate: std::env::var("CR_PAY_RATE")
            .ok()
            .and_then(|v| v.parse().ok()),
        pay_markup: std::env::var("CR_PAY_MARKUP")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(state::DEFAULT_MARKUP_PERCENT),
        warn_rub: std::env::var("CR_WARN_BALANCE_RUB")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(state::DEFAULT_WARN_BALANCE_RUB),
        mail_config: std::env::var_os("CR_MAIL_CONFIG").map(PathBuf::from),
        mail_interval: std::env::var("CR_MAIL_INTERVAL")
            .ok()
            .and_then(|v| v.parse().ok()),
        no_mail: false,
        links_config: std::env::var_os("CR_LINKS_CONFIG").map(PathBuf::from),
    };

    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let (key, inline) = match arg.split_once('=') {
            Some((k, v)) => (k.to_string(), Some(v.to_string())),
            None => (arg.clone(), None),
        };
        let mut value = |name: &str| -> Result<String, String> {
            inline
                .clone()
                .or_else(|| it.next())
                .ok_or_else(|| format!("{name}: нужно значение"))
        };
        match key.as_str() {
            "--demo" => args.demo = true,
            "--demo-state" => {
                let v = value("--demo-state")?;
                args.demo = true;
                args.state = match v.as_str() {
                    "ok" => Status::Ok,
                    "warn" => Status::Warn,
                    "error" => Status::Error,
                    other => return Err(format!("неизвестное состояние: {other}")),
                };
            }
            "--demo-cycle" => {
                args.demo = true;
                args.cycle = true;
            }
            "--once" => args.once = true,
            "--api-key" => args.api_key = Some(value("--api-key")?),
            "--timezone" => args.timezone = value("--timezone")?,
            "--interval" => {
                let v = value("--interval")?;
                args.interval = v
                    .parse::<u64>()
                    .map_err(|_| format!("--interval: ожидается число секунд, получено {v}"))?
                    .max(MIN_INTERVAL);
            }
            "--dump-icons" => args.dump = Some(PathBuf::from(value("--dump-icons")?)),
            "--pay-rate" => {
                let v = value("--pay-rate")?;
                args.pay_rate = Some(
                    v.parse()
                        .map_err(|_| format!("--pay-rate: ожидается число, получено {v}"))?,
                );
            }
            "--pay-markup" => {
                let v = value("--pay-markup")?;
                args.pay_markup = v
                    .parse()
                    .map_err(|_| format!("--pay-markup: ожидается число, получено {v}"))?;
            }
            "--warn-balance-rub" => {
                let v = value("--warn-balance-rub")?;
                args.warn_rub = v.parse().map_err(|_| {
                    format!("--warn-balance-rub: ожидается число рублей, получено {v}")
                })?;
            }
            "--mail-config" => {
                args.mail_config = Some(PathBuf::from(value("--mail-config")?));
            }
            "--mail-interval" => {
                let v = value("--mail-interval")?;
                args.mail_interval = Some(v.parse::<u64>().map_err(|_| {
                    format!("--mail-interval: ожидается число секунд, получено {v}")
                })?);
            }
            "--no-mail" => args.no_mail = true,
            "--links-config" => {
                args.links_config = Some(PathBuf::from(value("--links-config")?));
            }
            "-h" | "--help" => {
                print!("{HELP}");
                std::process::exit(0);
            }
            other => return Err(format!("неизвестный аргумент: {other}")),
        }
    }
    Ok(args)
}

/// Ключ: флаг → `CLOSEROUTER_API_KEY` → `~/.config/cr-tray/api_key`.
fn api_key(flag: Option<String>) -> Option<String> {
    let clean = |s: String| {
        let trimmed = s.trim().to_string();
        (!trimmed.is_empty()).then_some(trimmed)
    };
    flag.and_then(clean)
        .or_else(|| std::env::var("CLOSEROUTER_API_KEY").ok().and_then(clean))
        .or_else(|| std::fs::read_to_string(key_file()?).ok().and_then(clean))
}

fn key_file() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("cr-tray").join("api_key"))
}

fn source_for(args: &Args) -> Result<Box<dyn Source>, String> {
    if args.demo {
        return Ok(Box::new(source::Demo::new(
            args.state,
            args.pay_rate,
            args.pay_markup,
        )));
    }
    let key = api_key(args.api_key.clone()).ok_or_else(|| {
        "не задан API-ключ: укажите --api-key, CLOSEROUTER_API_KEY или ~/.config/cr-tray/api_key"
            .to_string()
    })?;

    let mail = if args.no_mail {
        None
    } else {
        match mail::load(args.mail_config.as_deref()) {
            Ok(Some(config)) if !config.accounts.is_empty() => {
                Some(mail::Checker::new(config, args.mail_interval))
            }
            Ok(_) => None,
            Err(e) => return Err(format!("почта: {e}")),
        }
    };

    Ok(Box::new(source::ApiSource::new(
        key,
        args.timezone.clone(),
        args.pay_rate,
        args.pay_markup,
        args.warn_rub,
        mail,
    )))
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("cr-tray: {e}\n\n{HELP}");
            return ExitCode::FAILURE;
        }
    };

    let src: Arc<dyn Source> = match source_for(&args) {
        Ok(s) => Arc::from(s),
        Err(e) => {
            eprintln!("cr-tray: {e}");
            return ExitCode::FAILURE;
        }
    };
    let snap = src.snapshot();

    if args.once {
        println!("{}", format::report(&snap));
        return if snap.status == Status::Error {
            ExitCode::FAILURE
        } else {
            ExitCode::SUCCESS
        };
    }

    if let Some(dir) = &args.dump {
        return match icon::dump_ppm(&snap, dir) {
            Ok(()) => {
                println!("иконки сохранены в {}", dir.display());
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("cr-tray: не удалось сохранить иконки: {e}");
                ExitCode::FAILURE
            }
        };
    }

    // Ссылки нужны только самому меню: `--once` и `--dump-icons` до сюда
    // не доходят, а битый конфиг не должен мешать проверке данных.
    let links = match links::load(args.links_config.as_deref()) {
        Ok(links) => links,
        Err(e) => {
            eprintln!("cr-tray: {e}");
            return ExitCode::FAILURE;
        }
    };

    // Один экземпляр трея на сессию: второй только мешал бы.
    let _instance = match lock::InstanceLock::acquire() {
        Ok(l) => l,
        Err(e) => {
            eprintln!("cr-tray: {e}");
            return ExitCode::FAILURE;
        }
    };

    let mut tray = CrTray::new(snap, links);
    let (wake_tx, wake_rx) = mpsc::channel();
    tray.wake = Some(wake_tx);

    let handle = match tray.spawn() {
        Ok(h) => h,
        Err(e) => {
            eprintln!("cr-tray: не удалось зарегистрировать иконку в трее: {e}");
            eprintln!("проверьте, что plasmashell запущен и есть org.kde.StatusNotifierWatcher");
            return ExitCode::FAILURE;
        }
    };

    eprintln!(
        "cr-tray: иконка добавлена в трей, опрос каждые {} с. Меню — клик по иконке.",
        args.interval
    );

    if args.cycle {
        let states = [Status::Ok, Status::Warn, Status::Error];
        let mut i = 0usize;
        loop {
            thread::sleep(Duration::from_secs(5));
            i = (i + 1) % states.len();
            let next = source::Demo::new(states[i], args.pay_rate, args.pay_markup).snapshot();
            let label = states[i].label();
            if handle
                .update(move |tray: &mut CrTray| tray.snap = next)
                .is_none()
            {
                eprintln!("cr-tray: трей больше не обслуживается, выходим");
                return ExitCode::FAILURE;
            }
            eprintln!("cr-tray: состояние → {label}");
        }
    }

    poll::run(src, handle, Duration::from_secs(args.interval), wake_rx);
    ExitCode::SUCCESS
}
