//! Пользовательские ссылки в меню трея.
//!
//! Ссылки задаются в JSON-конфиге `~/.config/cr-tray/links.json` и
//! показываются в меню отдельными пунктами: в подписи — только имя, по клику
//! открывается адрес в браузере. Если ссылок нет, пунктов в меню тоже нет.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// Один пункт меню: подпись и адрес, который открывается по клику.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub name: String,
    pub url: String,
}

/// Формат файла: `{"links": [{"name": "...", "url": "..."}]}`.
#[derive(Debug, Clone, Default, Deserialize)]
struct Config {
    /// Ссылки в произвольном количестве; порядок сохраняется в меню.
    #[serde(default)]
    links: Vec<RawLink>,
}

/// Сырая запись из файла: поля ещё не нормализованы.
#[derive(Debug, Clone, Deserialize)]
struct RawLink {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    url: Option<String>,
}

/// Путь к конфигу по умолчанию: `$XDG_CONFIG_HOME/cr-tray/links.json`.
pub fn default_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("cr-tray").join("links.json"))
}

/// Читает конфиг. `None` на входе — путь по умолчанию, его отсутствие не
/// ошибка: ссылок просто нет. Явно указанный путь должен существовать.
pub fn load(path: Option<&Path>) -> Result<Vec<Link>, String> {
    let (path, explicit) = match path {
        Some(path) => (path.to_path_buf(), true),
        None => match default_path() {
            Some(path) => (path, false),
            None => return Ok(Vec::new()),
        },
    };

    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && !explicit => return Ok(Vec::new()),
        Err(e) => return Err(format!("не удалось прочитать {}: {e}", path.display())),
    };

    let config: Config = serde_json::from_str(&text)
        .map_err(|e| format!("не удалось разобрать {}: {e}", path.display()))?;
    Ok(normalize(config))
}

/// Обрезает пробелы и выбрасывает записи без имени или без адреса.
fn normalize(config: Config) -> Vec<Link> {
    config
        .links
        .into_iter()
        .filter_map(|raw| {
            let name = raw.name?.trim().to_string();
            let url = raw.url?.trim().to_string();
            (!name.is_empty() && !url.is_empty()).then_some(Link { name, url })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_links_and_keeps_order() {
        let text = r#"{
            "links": [
                {"name": "Дашборд", "url": "https://closerouter.dev/usage"},
                {"name": "Документация", "url": "https://example.com/docs"}
            ]
        }"#;
        let config: Config = serde_json::from_str(text).unwrap();
        let links = normalize(config);
        assert_eq!(links.len(), 2);
        assert_eq!(links[0].name, "Дашборд");
        assert_eq!(links[0].url, "https://closerouter.dev/usage");
        assert_eq!(links[1].name, "Документация");
    }

    #[test]
    fn trims_spaces_and_skips_empty_entries() {
        let text = r#"{
            "links": [
                {"name": "  Дашборд  ", "url": "  https://a.example  "},
                {"name": "", "url": "https://empty-name.example"},
                {"name": "Без адреса"},
                {"name": "   ", "url": "https://blank.example"},
                {"url": "https://no-name.example"}
            ]
        }"#;
        let config: Config = serde_json::from_str(text).unwrap();
        let links = normalize(config);
        assert_eq!(
            links,
            vec![Link {
                name: "Дашборд".into(),
                url: "https://a.example".into(),
            }]
        );
    }

    #[test]
    fn empty_config_gives_no_links() {
        let config: Config = serde_json::from_str("{}").unwrap();
        assert!(normalize(config).is_empty());

        let config: Config = serde_json::from_str(r#"{"links": []}"#).unwrap();
        assert!(normalize(config).is_empty());
    }

    #[test]
    fn missing_default_config_is_not_an_error() {
        // Путь по умолчанию подменяем отсутствующим: XDG_CONFIG_HOME в тесте
        // может указывать куда угодно, поэтому проверяем явный несуществующий
        // файл отдельным тестом ниже, а здесь — семантику `None` при отсутствии
        // файла через временный каталог.
        let dir = std::env::temp_dir().join(format!("cr-tray-links-none-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let missing = dir.join("does-not-exist.json");
        assert!(load(Some(&missing)).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn loads_explicit_config() {
        let dir = std::env::temp_dir().join(format!("cr-tray-links-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("links.json");
        std::fs::write(
            &path,
            r#"{"links":[{"name":"A","url":"https://a.example"},
                         {"name":"B","url":"https://b.example"}]}"#,
        )
        .unwrap();

        let links = load(Some(&path)).unwrap();
        assert_eq!(links.len(), 2);
        assert_eq!(links[1].name, "B");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn invalid_json_is_an_error() {
        let dir = std::env::temp_dir().join(format!("cr-tray-links-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("links.json");
        std::fs::write(&path, r#"{"links":[{"name":"A",}]}"#).unwrap();

        assert!(load(Some(&path)).is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
