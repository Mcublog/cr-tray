//! Непрочитанные письма Mail.ru и Яндекса по IMAP.
//!
//! Публичного HTTP API для чтения почты у обоих сервисов нет, поэтому
//! используем IMAP с паролем приложения. Клиент минимальный: TLS, `LOGIN`,
//! команда `ID` и `STATUS INBOX (UNSEEN)`. Ящик при этом не выбирается
//! и на сервере ничего не меняется.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::Deserialize;

use crate::state::MailboxCount;

/// Таймаут подключения и чтения ответа почтового сервера.
const TIMEOUT: Duration = Duration::from_secs(10);

/// Период опроса почты по умолчанию, секунды.
pub const DEFAULT_INTERVAL_SEC: u64 = 300;

/// Чаще минуты почту не опрашиваем.
pub const MIN_INTERVAL_SEC: u64 = 60;

/// Настройки почты из `mail.json`.
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    /// Период опроса, секунды. Меньше [`MIN_INTERVAL_SEC`] поднимается до минимума.
    #[serde(default = "default_interval")]
    pub interval_sec: u64,
    /// Ящики в произвольном количестве; порядок сохраняется в меню.
    #[serde(default)]
    pub accounts: Vec<Account>,
}

fn default_interval() -> u64 {
    DEFAULT_INTERVAL_SEC
}

fn default_folder() -> String {
    "INBOX".into()
}

/// Один ящик из конфига.
#[derive(Debug, Clone, Deserialize)]
pub struct Account {
    /// Подпись в меню; если не задана, показывается `login`.
    #[serde(default)]
    pub name: Option<String>,
    pub login: String,
    pub password: String,
    /// IMAP-сервер; по умолчанию определяется по домену адреса.
    #[serde(default)]
    pub host: Option<String>,
    /// Куда переходить по клику в меню; по умолчанию — веб-почта провайдера.
    #[serde(default)]
    pub url: Option<String>,
    /// Папка, по умолчанию `INBOX`.
    #[serde(default = "default_folder")]
    pub folder: String,
}

/// Путь к конфигу по умолчанию: `$XDG_CONFIG_HOME/cr-tray/mail.json`.
pub fn default_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("cr-tray").join("mail.json"))
}

/// Читает конфиг. `None` на входе — путь по умолчанию, его отсутствие не ошибка.
/// Явно указанный путь должен существовать.
pub fn load(path: Option<&Path>) -> Result<Option<Config>, String> {
    let (path, explicit) = match path {
        Some(path) => (path.to_path_buf(), true),
        None => match default_path() {
            Some(path) => (path, false),
            None => return Ok(None),
        },
    };

    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && !explicit => return Ok(None),
        Err(e) => return Err(format!("не удалось прочитать {}: {e}", path.display())),
    };
    warn_permissions(&path);

    let config: Config = serde_json::from_str(&text)
        .map_err(|e| format!("не удалось разобрать {}: {e}", path.display()))?;
    Ok(Some(config))
}

/// Пароли лежат в файле, поэтому права шире 600 — повод предупредить.
#[cfg(unix)]
fn warn_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = fs::metadata(path) {
        let mode = meta.permissions().mode() & 0o777;
        if mode & 0o077 != 0 {
            eprintln!(
                "cr-tray: {} доступен другим пользователям (права {:03o}); рекомендуем chmod 600",
                path.display(),
                mode
            );
        }
    }
}

#[cfg(not(unix))]
fn warn_permissions(_path: &Path) {}

/// IMAP-сервер по домену адреса. `None` — провайдер неизвестен.
pub fn provider_host(login: &str) -> Option<&'static str> {
    let domain = login.rsplit_once('@')?.1.to_ascii_lowercase();
    match domain.as_str() {
        "mail.ru" | "inbox.ru" | "list.ru" | "bk.ru" | "internet.ru" => Some("imap.mail.ru"),
        "yandex.ru" | "ya.ru" | "narod.ru" | "yandex.com" => Some("imap.yandex.ru"),
        _ => None,
    }
}

/// Один настроенный ящик, уже приведённый к параметрам подключения.
struct Target {
    name: String,
    login: String,
    password: String,
    host: String,
    folder: String,
    /// Ссылка на веб-почту для клика в меню.
    url: Option<String>,
    /// Ошибка конфигурации, если сервер определить не удалось.
    config_error: Option<String>,
}

impl Target {
    fn from_account(account: Account) -> Self {
        let name = account
            .name
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .unwrap_or(&account.login)
            .to_string();

        let host = account
            .host
            .as_deref()
            .map(str::trim)
            .filter(|host| !host.is_empty())
            .map(str::to_string);
        let (host, config_error) = match host {
            Some(host) => (host, None),
            None => match provider_host(&account.login) {
                Some(host) => (host.to_string(), None),
                None => (
                    String::new(),
                    Some(format!(
                        "не удалось определить IMAP-сервер для {}; укажите host в конфиге",
                        account.login
                    )),
                ),
            },
        };
        let url = account
            .url
            .as_deref()
            .map(str::trim)
            .filter(|url| !url.is_empty())
            .map(str::to_string)
            .or_else(|| inbox_url(&host));

        Self {
            name,
            login: account.login,
            password: account.password,
            host,
            folder: account.folder,
            url,
            config_error,
        }
    }
}

/// Веб-почта провайдера по умолчанию.
fn inbox_url(host: &str) -> Option<String> {
    match host {
        "imap.mail.ru" => Some("https://e.mail.ru/inbox/".into()),
        "imap.yandex.ru" => Some("https://mail.yandex.ru/".into()),
        _ => None,
    }
}

/// Опрашивает ящики и кэширует последние значения.
pub struct Checker {
    targets: Vec<Target>,
    interval: Duration,
    last: Mutex<Option<(Instant, Vec<MailboxCount>)>>,
}

impl Checker {
    /// `interval_override` (флаг/env) приоритетнее `interval_sec` из конфига.
    pub fn new(config: Config, interval_override: Option<u64>) -> Self {
        Self {
            targets: config
                .accounts
                .into_iter()
                .map(Target::from_account)
                .collect(),
            interval: Duration::from_secs(
                interval_override
                    .unwrap_or(config.interval_sec)
                    .max(MIN_INTERVAL_SEC),
            ),
            last: Mutex::new(None),
        }
    }

    /// Счётчики по всем ящикам. Между опросами возвращается кэш; при ошибке
    /// ящика сохраняется последнее удачное значение и добавляется причина.
    pub fn check(&self) -> Vec<MailboxCount> {
        self.check_inner(false)
    }

    /// Принудительный опрос: кэш игнорируется, данные забираются сразу.
    /// Используется при ручном обновлении из меню.
    pub fn check_now(&self) -> Vec<MailboxCount> {
        self.check_inner(true)
    }

    fn check_inner(&self, force: bool) -> Vec<MailboxCount> {
        if self.targets.is_empty() {
            return Vec::new();
        }

        let previous = self.last.lock().unwrap().clone();
        if let Some((at, counts)) = &previous {
            if !force && at.elapsed() < self.interval {
                return counts.clone();
            }
        }

        let mut fresh = self.fetch_all();
        let previous_counts = previous
            .as_ref()
            .map(|(_, counts)| counts.as_slice())
            .unwrap_or(&[]);
        for (i, count) in fresh.iter_mut().enumerate() {
            if count.unread.is_none() {
                if let Some(prev) = previous_counts.get(i) {
                    count.unread = prev.unread;
                }
            }
        }

        *self.last.lock().unwrap() = Some((Instant::now(), fresh.clone()));
        fresh
    }

    fn fetch_all(&self) -> Vec<MailboxCount> {
        std::thread::scope(|scope| {
            let handles: Vec<_> = self
                .targets
                .iter()
                .map(|target| {
                    let name = target.name.clone();
                    (name, scope.spawn(move || fetch_one(target)))
                })
                .collect();
            handles
                .into_iter()
                .map(|(name, handle)| match handle.join() {
                    Ok(count) => count,
                    Err(_) => MailboxCount {
                        name,
                        unread: None,
                        error: Some("поток опроса почты аварийно завершился".into()),
                        url: None,
                    },
                })
                .collect()
        })
    }
}

fn fetch_one(target: &Target) -> MailboxCount {
    match fetch_unread(target) {
        Ok(unread) => MailboxCount {
            name: target.name.clone(),
            unread: Some(unread),
            error: None,
            url: target.url.clone(),
        },
        Err(error) => MailboxCount {
            name: target.name.clone(),
            unread: None,
            error: Some(error),
            url: target.url.clone(),
        },
    }
}

fn fetch_unread(target: &Target) -> Result<u64, String> {
    if let Some(error) = &target.config_error {
        return Err(error.clone());
    }

    let addr = (target.host.as_str(), 993u16)
        .to_socket_addrs()
        .map_err(|e| format!("не удалось найти {}: {e}", target.host))?
        .next()
        .ok_or_else(|| format!("не удалось найти {}", target.host))?;

    let socket = TcpStream::connect_timeout(&addr, TIMEOUT).map_err(io_error)?;
    socket.set_read_timeout(Some(TIMEOUT)).map_err(io_error)?;
    socket.set_write_timeout(Some(TIMEOUT)).map_err(io_error)?;
    let _ = socket.set_nodelay(true);

    let mut imap = Imap::connect(&target.host, socket)?;
    imap.login(&target.login, &target.password)?;
    // Mail.ru и Яндекс просят клиентов представляться командой ID. Серверам,
    // которые её не знают, ответ BAD не мешает.
    let _ = imap.id();
    let unseen = imap.status(&target.folder)?;
    let _ = imap.logout();
    Ok(unseen)
}

fn tls_config() -> Arc<rustls::ClientConfig> {
    static CONFIG: OnceLock<Arc<rustls::ClientConfig>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            let mut roots = rustls::RootCertStore::empty();
            roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            let provider = Arc::new(rustls::crypto::ring::default_provider());
            let config = rustls::ClientConfig::builder_with_provider(provider)
                .with_safe_default_protocol_versions()
                .expect("ring provider supports the default TLS versions")
                .with_root_certificates(roots)
                .with_no_client_auth();
            Arc::new(config)
        })
        .clone()
}

struct Imap {
    reader: BufReader<rustls::StreamOwned<rustls::ClientConnection, TcpStream>>,
    next_tag: u32,
}

impl Imap {
    fn connect(host: &str, socket: TcpStream) -> Result<Self, String> {
        let server_name = rustls::pki_types::ServerName::try_from(host.to_string())
            .map_err(|e| format!("некорректное имя сервера {host}: {e}"))?;
        let connection = rustls::ClientConnection::new(tls_config(), server_name)
            .map_err(|e| format!("TLS: {e}"))?;
        let stream = rustls::StreamOwned::new(connection, socket);

        let mut imap = Self {
            reader: BufReader::new(stream),
            next_tag: 1,
        };
        let greeting = imap.read_line()?;
        let upper = greeting.to_ascii_uppercase();
        if !(upper.starts_with("* OK") || upper.starts_with("* PREAUTH")) {
            return Err(format!("сервер IMAP отказал: {greeting}"));
        }
        Ok(imap)
    }

    fn read_line(&mut self) -> Result<String, String> {
        let mut line = String::new();
        let read = self.reader.read_line(&mut line).map_err(io_error)?;
        if read == 0 {
            return Err("сервер IMAP закрыл соединение".into());
        }
        if line.len() > 64 * 1024 {
            return Err("слишком длинная строка ответа IMAP".into());
        }
        Ok(line.trim_end_matches(['\r', '\n']).to_string())
    }

    fn command(&mut self, command: &str) -> Result<Vec<String>, String> {
        let tag = format!("a{}", self.next_tag);
        self.next_tag = self.next_tag.saturating_add(1);

        {
            let stream = self.reader.get_mut();
            stream
                .write_all(format!("{tag} {command}\r\n").as_bytes())
                .map_err(io_error)?;
            stream.flush().map_err(io_error)?;
        }

        let mut lines = Vec::new();
        loop {
            let line = self.read_line()?;
            if let Some(rest) = line.strip_prefix(&tag) {
                if !rest.starts_with(' ') {
                    lines.push(line);
                    continue;
                }
                let rest = rest.trim_start();
                let ok = rest
                    .get(..2)
                    .is_some_and(|status| status.eq_ignore_ascii_case("OK"));
                return if ok {
                    Ok(lines)
                } else {
                    Err(status_error(rest))
                };
            }
            lines.push(line);
        }
    }

    fn login(&mut self, login: &str, password: &str) -> Result<(), String> {
        let command = format!("LOGIN {} {}", quote(login)?, quote(password)?);
        self.command(&command).map(|_| ())
    }

    fn id(&mut self) -> Result<(), String> {
        self.command("ID (\"name\" \"cr-tray\" \"version\" \"0.1\")")
            .map(|_| ())
    }

    fn status(&mut self, folder: &str) -> Result<u64, String> {
        let command = format!("STATUS {} (UNSEEN)", quote(folder)?);
        let lines = self.command(&command)?;
        parse_status(&lines).ok_or_else(|| "сервер не сообщил число непрочитанных".into())
    }

    fn logout(&mut self) -> Result<(), String> {
        self.command("LOGOUT").map(|_| ())
    }
}

/// Строка IMAP в кавычках; переводы строк недопустимы, иначе можно
/// подмешать лишнюю команду в протокол.
fn quote(value: &str) -> Result<String, String> {
    if value.contains(['\r', '\n', '\0']) {
        return Err("в логине или пароле есть недопустимые символы".into());
    }
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    Ok(format!("\"{escaped}\""))
}

fn status_error(rest: &str) -> String {
    let text = rest.trim();
    let upper = text.to_ascii_uppercase();
    if upper.contains("AUTHENTICATIONFAILED")
        || upper.contains("INVALID CREDENTIALS")
        || upper.contains("LOGIN FAILED")
        || upper.contains("AUTHORIZATION FAILED")
    {
        return "неверный логин или пароль приложения".into();
    }
    let short: String = text.chars().take(160).collect();
    format!("сервер IMAP: {short}")
}

fn io_error(error: std::io::Error) -> String {
    match error.kind() {
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => {
            "таймаут подключения к почтовому серверу".into()
        }
        _ => format!("сеть: {error}"),
    }
}

/// Достаёт `UNSEEN n` из ответа `* STATUS ... (UNSEEN n)`.
pub fn parse_status(lines: &[String]) -> Option<u64> {
    for line in lines {
        let upper = line.to_ascii_uppercase();
        if !upper.starts_with("* STATUS ") {
            continue;
        }
        let Some(pos) = upper.find("UNSEEN") else {
            continue;
        };
        // Не принимаем UNSEEN, если это часть другого слова.
        if line[..pos]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            continue;
        }
        let tail = &line[pos + "UNSEEN".len()..];
        let digits: String = tail
            .trim_start()
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        if digits.is_empty() {
            continue;
        }
        if let Ok(unread) = digits.parse() {
            return Some(unread);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_provider_by_domain() {
        assert_eq!(provider_host("me@mail.ru"), Some("imap.mail.ru"));
        assert_eq!(provider_host("me@inbox.ru"), Some("imap.mail.ru"));
        assert_eq!(provider_host("me@bk.ru"), Some("imap.mail.ru"));
        assert_eq!(provider_host("me@yandex.ru"), Some("imap.yandex.ru"));
        assert_eq!(provider_host("me@ya.ru"), Some("imap.yandex.ru"));
        assert_eq!(provider_host("me@example.com"), None);
        assert_eq!(provider_host("без-собаки"), None);
    }

    #[test]
    fn parses_unseen_from_status() {
        let lines = vec!["* STATUS \"INBOX\" (UNSEEN 12)".to_string()];
        assert_eq!(parse_status(&lines), Some(12));

        let lines = vec!["* STATUS INBOX (MESSAGES 100 UNSEEN 0)".to_string()];
        assert_eq!(parse_status(&lines), Some(0));

        let lines = vec!["* STATUS \"Моя папка\" (UNSEEN 7)".to_string()];
        assert_eq!(parse_status(&lines), Some(7));
    }

    #[test]
    fn ignores_unrelated_status_lines() {
        let lines = vec![
            "* OK [UNSEEN 3] first unseen".to_string(),
            "* STATUS INBOX (MESSAGES 4)".to_string(),
        ];
        assert_eq!(parse_status(&lines), None);
    }

    #[test]
    fn quotes_login_and_password() {
        assert_eq!(quote("me@mail.ru").unwrap(), "\"me@mail.ru\"");
        assert_eq!(quote("a\"b\\c").unwrap(), "\"a\\\"b\\\\c\"");
        assert!(quote("a\r\nB2 DELETE INBOX").is_err());
        assert!(quote("a\nb").is_err());
    }

    #[test]
    fn auth_failure_is_explained() {
        let text = status_error("[AUTHENTICATIONFAILED] Invalid credentials");
        assert_eq!(text, "неверный логин или пароль приложения");
    }

    #[test]
    fn account_keeps_name_or_falls_back_to_login() {
        let named = Target::from_account(Account {
            name: Some("Работа".into()),
            login: "me@mail.ru".into(),
            password: "secret".into(),
            host: None,
            url: None,
            folder: default_folder(),
        });
        assert_eq!(named.name, "Работа");
        assert_eq!(named.host, "imap.mail.ru");
        assert_eq!(named.url.as_deref(), Some("https://e.mail.ru/inbox/"));

        let fallback = Target::from_account(Account {
            name: None,
            login: "me@yandex.ru".into(),
            password: "secret".into(),
            host: None,
            url: None,
            folder: default_folder(),
        });
        assert_eq!(fallback.name, "me@yandex.ru");
        assert_eq!(fallback.host, "imap.yandex.ru");
        assert_eq!(fallback.url.as_deref(), Some("https://mail.yandex.ru/"));

        let custom = Target::from_account(Account {
            name: None,
            login: "me@crystals.ru".into(),
            password: "secret".into(),
            host: Some("imap.yandex.ru".into()),
            url: Some("https://mail.yandex.ru/for/crystals.ru".into()),
            folder: default_folder(),
        });
        assert_eq!(
            custom.url.as_deref(),
            Some("https://mail.yandex.ru/for/crystals.ru")
        );
    }

    #[test]
    fn unknown_provider_keeps_account_with_error() {
        let target = Target::from_account(Account {
            name: None,
            login: "me@example.com".into(),
            password: "secret".into(),
            host: None,
            url: None,
            folder: default_folder(),
        });
        assert!(target.config_error.is_some());
    }

    #[test]
    fn config_allows_any_number_of_accounts() {
        let text = r#"{
            "interval_sec": 120,
            "accounts": [
                {"login": "a@mail.ru", "password": "1"},
                {"name": "Работа", "login": "b@yandex.ru", "password": "2"},
                {"login": "c@mail.ru", "password": "3", "host": "imap.example.com"}
            ]
        }"#;
        let config: Config = serde_json::from_str(text).unwrap();
        assert_eq!(config.accounts.len(), 3);
        assert_eq!(config.interval_sec, 120);
        assert_eq!(config.accounts[2].host.as_deref(), Some("imap.example.com"));
    }

    #[test]
    fn interval_is_not_lower_than_minimum() {
        let config = Config {
            interval_sec: 5,
            accounts: Vec::new(),
        };
        let checker = Checker::new(config, None);
        assert_eq!(checker.interval, Duration::from_secs(MIN_INTERVAL_SEC));
    }

    #[test]
    fn loads_explicit_config_and_rejects_missing_one() {
        let dir = std::env::temp_dir().join(format!("cr-tray-mail-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("mail.json");
        std::fs::write(
            &path,
            r#"{"accounts":[{"login":"a@mail.ru","password":"1"},
                            {"login":"b@yandex.ru","password":"2"}]}"#,
        )
        .unwrap();

        let config = load(Some(&path)).unwrap().unwrap();
        assert_eq!(config.accounts.len(), 2);
        assert_eq!(config.interval_sec, DEFAULT_INTERVAL_SEC);

        let missing = dir.join("absent.json");
        assert!(load(Some(&missing)).is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_checker_needs_no_network() {
        let checker = Checker::new(
            Config {
                interval_sec: DEFAULT_INTERVAL_SEC,
                accounts: Vec::new(),
            },
            None,
        );
        assert!(checker.check().is_empty());
    }
}
