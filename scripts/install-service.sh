#!/bin/sh
# Установка и удаление systemd-юнита пользователя для cr-tray.
#
#   ./scripts/install-service.sh              # установить и запустить
#   ./scripts/install-service.sh uninstall    # остановить и удалить
#   ./scripts/install-service.sh status       # показать состояние
#
# Пути можно переопределить окружением:
#   CR_TRAY_BIN          бинарник        (~/.local/bin/cr-tray)
#   CR_TRAY_CONFIG_DIR   каталог настроек (~/.config/cr-tray)
#   CR_TRAY_UNIT_DIR     каталог юнитов  (~/.config/systemd/user)
#   CR_TRAY_DRY_RUN=1    только показать, что будет сделано
set -eu

BIN="${CR_TRAY_BIN:-$HOME/.local/bin/cr-tray}"
CONFIG_DIR="${CR_TRAY_CONFIG_DIR:-${XDG_CONFIG_HOME:-$HOME/.config}/cr-tray}"
UNIT_DIR="${CR_TRAY_UNIT_DIR:-${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user}"
UNIT="$UNIT_DIR/cr-tray.service"
ENV_FILE="$CONFIG_DIR/env"
DRY_RUN="${CR_TRAY_DRY_RUN:-0}"

say() { printf '%s\n' "$*"; }
run() {
    if [ "$DRY_RUN" = 1 ]; then
        printf '  + %s\n' "$*"
    else
        "$@"
    fi
}

systemctl_user() { systemctl --user "$@"; }

write_unit() {
    mkdir -p "$UNIT_DIR"
    cat > "$UNIT" <<EOF
[Unit]
Description=CloseRouter usage tray
After=plasma-workspace.target
PartOf=plasma-workspace.target

[Service]
Type=simple
ExecStart=$BIN
EnvironmentFile=-$ENV_FILE
Restart=on-failure
RestartSec=5

[Install]
WantedBy=plasma-workspace.target
EOF
}

write_env_template() {
    mkdir -p "$CONFIG_DIR"
    if [ -e "$ENV_FILE" ]; then
        say "настройки уже есть: $ENV_FILE"
        return
    fi
    cat > "$ENV_FILE" <<'EOF'
# Настройки cr-tray для systemd. Файл с правами 600: здесь может лежать ключ.
#
# CLOSEROUTER_API_KEY=sk-...
# CR_PAY_MARKUP=12
# CR_INTERVAL=60
# CR_WARN_BALANCE_RUB=50
# CR_PAY_RATE=90.22
EOF
    chmod 600 "$ENV_FILE"
    say "создан шаблон настроек: $ENV_FILE"
}

has_key() {
    [ -n "${CLOSEROUTER_API_KEY:-}" ] && return 0
    [ -s "$CONFIG_DIR/api_key" ] && return 0
    [ -f "$ENV_FILE" ] && grep -q '^CLOSEROUTER_API_KEY=..*' "$ENV_FILE" && return 0
    return 1
}

cmd_install() {
    if [ ! -x "$BIN" ]; then
        say "cr-tray: нет исполняемого файла $BIN" >&2
        say "положите бинарник туда:" >&2
        say "  cargo build --release && install -Dm755 target/release/cr-tray $BIN" >&2
        exit 1
    fi

    write_env_template
    write_unit
    say "юнит: $UNIT"
    say "  ExecStart=$BIN"
    say "  EnvironmentFile=$ENV_FILE"

    if ! has_key; then
        say "" >&2
        say "внимание: API-ключ не найден." >&2
        say "впишите CLOSEROUTER_API_KEY=... в $ENV_FILE (или положите ключ в $CONFIG_DIR/api_key)," >&2
        say "иначе сервис будет перезапускаться с ошибкой «не задан API-ключ»." >&2
    fi

    run systemctl --user daemon-reload
    run systemctl --user enable --now cr-tray.service
    say "готово. Проверить: $0 status"
}

cmd_uninstall() {
    if [ -f "$UNIT" ]; then
        run systemctl --user disable --now cr-tray.service || true
        run rm -f "$UNIT"
        run systemctl --user daemon-reload
        say "юнит удалён: $UNIT"
    else
        say "юнита нет: $UNIT"
    fi
    say "настройки и кэш оставлены: $CONFIG_DIR, ~/.cache/cr-tray"
}

cmd_status() {
    systemctl --user --no-pager status cr-tray.service || true
    say ""
    say "--- последние строки журнала ---"
    journalctl --user -u cr-tray.service -n 20 --no-pager || true
}

usage() {
    # Печатаем только шапку-комментарий до первой непустой не-комментарной строки.
    awk 'NR > 1 { if ($0 !~ /^#/) exit; sub(/^# ?/, ""); print }' "$0"
}

case "${1:-install}" in
    install) cmd_install ;;
    uninstall | remove) cmd_uninstall ;;
    status) cmd_status ;;
    -h | --help | help)
        usage
        ;;
    *)
        say "неизвестная команда: $1 (install | uninstall | status)" >&2
        exit 2
        ;;
esac
