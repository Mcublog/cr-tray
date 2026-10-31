//! Курс ЦБ РФ: XML в windows-1251 и кэш на диск.
//!
//! Эндпоинт: `https://www.cbr.ru/scripts/XML_daily.asp`. В выходные ЦБ отдаёт
//! курс предыдущего рабочего дня, поэтому дату курса всегда показываем рядом.

use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

const URL: &str = "https://www.cbr.ru/scripts/XML_daily.asp";
const TIMEOUT: Duration = Duration::from_secs(10);
const USER_AGENT: &str = "cr-tray/0.1";

/// Курс USD к рублю на дату.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rate {
    /// Сколько рублей за доллар.
    pub value: f64,
    /// Дата курса в формате ДД.ММ.ГГГГ.
    pub date: String,
}

/// Курс с диска: значение плюс локальный день, когда он был получен.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Cached {
    rate: Rate,
    /// Локальная дата `ГГГГ-ММ-ДД` на момент запроса.
    day: String,
}

pub struct Cbr {
    cache: Option<PathBuf>,
    agent: ureq::Agent,
}

impl Cbr {
    pub fn new() -> Self {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(TIMEOUT))
            .user_agent(USER_AGENT)
            .build()
            .into();
        Self {
            cache: cache_path(),
            agent,
        }
    }

    /// Курс на сегодня. Свежий кэш за текущий день не перезапрашивается;
    /// при сетевой ошибке возвращается последний сохранённый курс.
    pub fn rate(&self, today: &str) -> Result<Rate, String> {
        let cached = self.load();
        if let Some(c) = &cached {
            if c.day == today {
                return Ok(c.rate.clone());
            }
        }

        match self.fetch() {
            Ok(rate) => {
                self.save(&Cached {
                    rate: rate.clone(),
                    day: today.to_string(),
                });
                Ok(rate)
            }
            Err(e) => match cached {
                Some(c) => {
                    eprintln!(
                        "cr-tray: ЦБ недоступен ({e}), использую курс от {}",
                        c.rate.date
                    );
                    Ok(c.rate)
                }
                None => Err(e),
            },
        }
    }

    fn fetch(&self) -> Result<Rate, String> {
        let mut resp = self
            .agent
            .get(URL)
            .call()
            .map_err(|e| match e {
                ureq::Error::StatusCode(code) => format!("HTTP {code}"),
                other => other.to_string(),
            })?;
        let bytes = resp
            .body_mut()
            .read_to_vec()
            .map_err(|e| format!("чтение ответа: {e}"))?;
        parse(&decode(&bytes))
    }

    fn load(&self) -> Option<Cached> {
        let path = self.cache.as_ref()?;
        let text = fs::read_to_string(path).ok()?;
        serde_json::from_str(&text).ok()
    }

    fn save(&self, cached: &Cached) {
        let Some(path) = self.cache.as_ref() else {
            return;
        };
        if let Some(dir) = path.parent() {
            if let Err(e) = fs::create_dir_all(dir) {
                eprintln!("cr-tray: не удалось создать {}: {e}", dir.display());
                return;
            }
        }
        match serde_json::to_string(cached) {
            Ok(text) => {
                if let Err(e) = fs::write(path, text) {
                    eprintln!("cr-tray: не удалось сохранить кэш ЦБ: {e}");
                }
            }
            Err(e) => eprintln!("cr-tray: не удалось сериализовать кэш ЦБ: {e}"),
        }
    }
}

impl Default for Cbr {
    fn default() -> Self {
        Self::new()
    }
}

/// `$XDG_CACHE_HOME/cr-tray/cbr.json` или `~/.cache/cr-tray/cbr.json`.
fn cache_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
    Some(base.join("cr-tray").join("cbr.json"))
}

/// Ответ ЦБ приходит в windows-1251.
pub fn decode(bytes: &[u8]) -> String {
    let (text, _, _) = encoding_rs::WINDOWS_1251.decode(bytes);
    text.into_owned()
}

/// Достаёт курс USD и дату из XML ЦБ.
pub fn parse(xml: &str) -> Result<Rate, String> {
    let doc = roxmltree::Document::parse(xml).map_err(|e| format!("XML: {e}"))?;
    let root = doc.root_element();
    let date = root
        .attribute("Date")
        .ok_or_else(|| "в ответе ЦБ нет даты".to_string())?
        .to_string();

    for valute in root.children().filter(|n| n.has_tag_name("Valute")) {
        let usd = valute
            .children()
            .find(|n| n.has_tag_name("CharCode"))
            .and_then(|n| n.text())
            .map(|t| t.trim() == "USD")
            .unwrap_or(false);
        if !usd {
            continue;
        }
        let raw = valute
            .children()
            .find(|n| n.has_tag_name("Value"))
            .and_then(|n| n.text())
            .ok_or_else(|| "у USD нет значения".to_string())?;
        let value = raw
            .trim()
            .replace(',', ".")
            .replace('\u{a0}', "")
            .parse::<f64>()
            .map_err(|_| format!("не удалось разобрать курс {raw:?}"))?;
        if !(value.is_finite() && value > 0.0) {
            return Err(format!("неправдоподобный курс {raw:?}"));
        }
        return Ok(Rate { value, date });
    }
    Err("в ответе ЦБ нет USD".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Реальный ответ ЦБ за 31.10.2026, как пришёл с сервера (cp1251).
    const CBR_XML: &[u8] = include_bytes!("../tests/fixtures/cbr.xml");

    #[test]
    fn decodes_windows_1251() {
        let text = decode(CBR_XML);
        assert!(text.contains("Доллар США"), "{}", &text[..200]);
        assert!(!text.contains('\u{fffd}'));
    }

    #[test]
    fn parses_usd_rate() {
        let rate = parse(&decode(CBR_XML)).unwrap();
        assert_eq!(rate.date, "31.10.2026");
        assert!((rate.value - 80.5573).abs() < 1e-9, "{}", rate.value);
    }

    #[test]
    fn missing_usd_is_an_error() {
        let xml = r#"<ValCurs Date="31.10.2026"><Valute ID="R01235">
            <CharCode>EUR</CharCode><Value>90,1234</Value></Valute></ValCurs>"#;
        assert!(parse(xml).is_err());
    }

    #[test]
    fn malformed_value_is_an_error() {
        let xml = r#"<ValCurs Date="31.10.2026"><Valute ID="R01235">
            <CharCode>USD</CharCode><Value>—</Value></Valute></ValCurs>"#;
        assert!(parse(xml).is_err());
    }

    #[test]
    fn cache_roundtrip() {
        let dir = std::env::temp_dir().join(format!("cr-tray-test-{}", std::process::id()));
        let path = dir.join("cbr.json");
        let cached = Cached {
            rate: Rate {
                value: 80.5573,
                date: "31.10.2026".into(),
            },
            day: "2026-10-31".into(),
        };
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, serde_json::to_string(&cached).unwrap()).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let back: Cached = serde_json::from_str(&text).unwrap();
        assert_eq!(back.rate, cached.rate);
        assert_eq!(back.day, cached.day);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
