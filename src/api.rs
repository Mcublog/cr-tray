//! Клиент публичного API CloseRouter.
//!
//! Используются два эндпоинта:
//! - `GET /v1/credits` — баланс аккаунта;
//! - `GET /v1/usage` — расход за период (передаём полные ISO-даты, см. docs/plan.md).

use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::Deserialize;

const DEFAULT_BASE: &str = "https://api.closerouter.dev/v1";
const TIMEOUT: Duration = Duration::from_secs(10);
const USER_AGENT: &str = "cr-tray/0.1";

/// Ошибка обращения к API.
#[derive(Debug)]
pub enum ApiError {
    /// Сетевая ошибка, таймаут, TLS.
    Transport(String),
    /// Ответ 4xx/5xx.
    Status(u16),
    /// Тело ответа не разобралось.
    Parse(String),
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApiError::Transport(e) => write!(f, "сеть: {e}"),
            ApiError::Status(401) => write!(f, "неверный API-ключ (401)"),
            ApiError::Status(429) => write!(f, "слишком много запросов (429)"),
            ApiError::Status(code) => write!(f, "HTTP {code}"),
            ApiError::Parse(e) => write!(f, "разбор ответа: {e}"),
        }
    }
}

impl std::error::Error for ApiError {}

/// Баланс аккаунта.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Credits {
    /// Сколько денег на счету, $.
    pub total: f64,
    /// Сколько уже потрачено за всё время, $.
    pub used: f64,
}

/// Расход за период.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Usage {
    pub cost: f64,
    pub requests: u64,
    pub tokens: u64,
}

pub struct Client {
    key: String,
    base: String,
    agent: ureq::Agent,
}

impl Client {
    pub fn new(key: impl Into<String>) -> Self {
        Self::with_base(key, DEFAULT_BASE)
    }

    pub fn with_base(key: impl Into<String>, base: impl Into<String>) -> Self {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(TIMEOUT))
            .user_agent(USER_AGENT)
            .build()
            .into();
        Self {
            key: key.into(),
            base: base.into(),
            agent,
        }
    }

    /// Баланс: `/v1/credits`.
    pub fn credits(&self) -> Result<Credits, ApiError> {
        let body = self.get("/credits", &[])?;
        parse_credits(&body)
    }

    /// Расход за период; границы — полные ISO-datetime с `Z`.
    pub fn usage(&self, start: &str, end: &str, timezone: &str) -> Result<Usage, ApiError> {
        let body = self.get(
            "/usage",
            &[
                ("start_date", start),
                ("end_date", end),
                ("timezone", timezone),
            ],
        )?;
        parse_usage(&body)
    }

    fn get(&self, path: &str, query: &[(&str, &str)]) -> Result<String, ApiError> {
        let mut url = format!("{}{path}", self.base.trim_end_matches('/'));
        if !query.is_empty() {
            url.push('?');
            for (i, (k, v)) in query.iter().enumerate() {
                if i > 0 {
                    url.push('&');
                }
                url.push_str(k);
                url.push('=');
                url.push_str(&encode(v));
            }
        }
        let resp = self
            .agent
            .get(&url)
            .header("Authorization", &format!("Bearer {}", self.key))
            .header("Accept", "application/json")
            .call()
            .map_err(|e| match e {
                ureq::Error::StatusCode(code) => ApiError::Status(code),
                other => ApiError::Transport(other.to_string()),
            })?;
        resp.into_body()
            .read_to_string()
            .map_err(|e| ApiError::Transport(e.to_string()))
    }
}

/// Минимальное процентное кодирование: параметры — даты, таймзоны, числа.
fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for b in value.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[derive(Deserialize)]
struct Envelope<T> {
    data: T,
}

#[derive(Deserialize)]
struct CreditsData {
    total_credits: f64,
    #[serde(default)]
    total_usage: f64,
}

#[derive(Deserialize)]
struct UsageData {
    totals: Totals,
}

#[derive(Deserialize, Default)]
struct Totals {
    #[serde(default)]
    cost: f64,
    #[serde(default)]
    requests: u64,
    #[serde(default)]
    total_tokens: u64,
}

fn parse_json<T: DeserializeOwned>(body: &str) -> Result<T, ApiError> {
    serde_json::from_str(body).map_err(|e| ApiError::Parse(e.to_string()))
}

pub fn parse_credits(body: &str) -> Result<Credits, ApiError> {
    let env: Envelope<CreditsData> = parse_json(body)?;
    Ok(Credits {
        total: env.data.total_credits,
        used: env.data.total_usage,
    })
}

pub fn parse_usage(body: &str) -> Result<Usage, ApiError> {
    let env: Envelope<UsageData> = parse_json(body)?;
    let t = env.data.totals;
    Ok(Usage {
        cost: t.cost,
        requests: t.requests,
        tokens: t.total_tokens,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const CREDITS: &str = include_str!("../tests/fixtures/credits.json");
    const USAGE: &str = include_str!("../tests/fixtures/usage.json");

    #[test]
    fn parses_credits() {
        let c = parse_credits(CREDITS).unwrap();
        assert!(c.total > 0.0);
        assert!(c.used >= 0.0);
    }

    #[test]
    fn parses_usage_totals() {
        let u = parse_usage(USAGE).unwrap();
        assert!(u.requests > 0);
        assert!(u.tokens > 0);
        assert!(u.cost > 0.0);
        // Токены — сумма всех категорий, cost — деньги, а не токены.
        assert!(u.tokens > u.requests);
        assert!(u.cost < 100.0);
    }

    #[test]
    fn api_error_reports_bad_key_and_limits() {
        assert_eq!(ApiError::Status(401).to_string(), "неверный API-ключ (401)");
        assert_eq!(
            ApiError::Status(429).to_string(),
            "слишком много запросов (429)"
        );
        assert_eq!(ApiError::Status(500).to_string(), "HTTP 500");
    }

    #[test]
    fn reports_parse_errors() {
        let err = parse_credits("{\"data\":{}}").unwrap_err();
        assert!(matches!(err, ApiError::Parse(_)), "{err}");
        assert!(parse_usage("not json").is_err());
    }

    #[test]
    fn error_envelope_is_reported() {
        let body = r#"{"error":{"code":"invalid_api_key","message":"Invalid API key"}}"#;
        assert!(parse_credits(body).is_err());
    }

    #[test]
    fn encodes_query_values() {
        assert_eq!(encode("Europe/Moscow"), "Europe%2FMoscow");
        assert_eq!(encode("2026-10-31T20:59:59Z"), "2026-10-31T20%3A59%3A59Z");
    }
}
