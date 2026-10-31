# Rust: установка, проверка, обновление, удаление

Инструкция для Ubuntu 26.04.1 LTS (x86_64). Rust ставим через `rustup` —
это официальный установщик, не требует `sudo`, версию можно менять и удалять
одной командой.

## 1. Что есть сейчас

Проверка перед установкой:

```bash
command -v rustc cargo rustup || echo "Rust не установлен"
```

Ожидаемо: `Rust не установлен`.

## 2. Установка через rustup

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

В интерактивном меню выбрать `1) Proceed with standard installation`.

Неинтерактивный вариант (сразу stable, без вопросов):

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
```

Минимальный профиль (без документации к стандартной библиотеке, экономит место):

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
```

Компоненты, которые понадобятся (можно добавить позже):

```bash
rustup component add rustfmt clippy
```

### Что установщик делает с системой

| Путь | Содержимое |
|---|---|
| `~/.cargo/bin/` | `cargo`, `rustc`, `rustup`, `rustfmt`, `clippy` (симлинки) |
| `~/.rustup/` | toolchains (stable и т.д.), компоненты |
| `~/.cargo/registry/` | скачанные крейты |
| `~/.cargo/git/` | git-зависимости |
| `~/.profile`, `~/.bashrc` | строка `source "$HOME/.cargo/env"` |

Системные файлы и `/usr/bin` не меняются, `sudo` не нужен.

### Размер на диске

| Что | Ориентир |
|---|---|
| Toolchain (minimal) | ~400–600 МБ |
| Toolchain (default, с docs) | ~1,2–1,5 ГБ |
| `~/.cargo/registry` после сборки проекта | ~100–300 МБ |
| `target/` одного проекта | ~200–500 МБ (удаляется `cargo clean`) |

## 3. Активация в текущей сессии

Установщик добавляет PATH в профиль, но текущий терминал его ещё не видит:

```bash
source "$HOME/.cargo/env"
```

Проверка:

```bash
rustc --version
cargo --version
rustup --version
```

Ожидаемо: stable-версия (`rustc` ≥ 1.85, этого достаточно для `ksni`/`zbus`).

Если `cargo: command not found` — либо не выполнен `source`, либо PATH не был
добавлен (проверить `grep cargo ~/.profile ~/.bashrc`).

## 4. Обновление

```bash
rustup update stable
rustup show            # активный toolchain
rustup component list --installed
```

## 5. Переустановка с нуля (если что-то сломалось)

```bash
rustup self uninstall
# затем повторить раздел 2
```

## 6. Удаление

### Штатно

```bash
rustup self uninstall
```

Команда удаляет `~/.rustup`, `~/.cargo` (включая скачанные крейты) и вычищает
строки `source "$HOME/.cargo/env"` из `~/.profile`, `~/.bashrc`, `~/.zshenv`.

### Если rustup уже удалён, а каталоги остались

```bash
rm -rf ~/.rustup ~/.cargo
```

Затем вручную убрать строку из профилей:

```bash
grep -n 'cargo/env' ~/.profile ~/.bashrc ~/.zshenv 2>/dev/null
```

Удалить найденные строки редактором. Проверить PATH в текущей сессии:

```bash
echo "$PATH" | tr ':' '\n' | grep -n cargo
```

Если путь остался — перезайти в сессию (или `exec $SHELL`).

### Проверка, что Rust удалён

```bash
command -v rustc cargo rustup || echo "Rust удалён"
ls -d ~/.cargo ~/.rustup 2>/dev/null || echo "каталоги удалены"
```

### Автодополнение

Если включали:

```bash
rm -f ~/.bash_completion.d/cargo ~/.config/fish/completions/cargo.fish
```

## 7. Альтернатива: системный пакет (не рекомендуется)

```bash
sudo apt install rustc cargo
sudo apt remove --purge rustc cargo
sudo apt autoremove
```

Минусы: версия из репозитория может быть старее требуемой `ksni`/`zbus`,
переключение версий и компонентов недоступно, удаление задевает системные
зависимости. Для этого проекта используем `rustup`.

## 8. Диагностика

| Симптом | Причина / решение |
|---|---|
| `cargo: command not found` | не выполнен `source "$HOME/.cargo/env"`; перезайти в терминал |
| Ошибки линковки `cc: not found` | `sudo apt install build-essential pkg-config` |
| Сборка падает на `openssl-sys` | в проекте используется `rustls`; проверить, что `reqwest` не тянет OpenSSL |
| Мало места | `cargo clean` в проекте, `rustup toolchain uninstall <old>` |
| Медленная первая сборка | нормально: компилируются все зависимости, дальше инкрементально |

Проверка наличия системного компилятора (нужен для линковки):

```bash
command -v cc gcc || sudo apt install build-essential
```
