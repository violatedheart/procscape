#!/bin/sh
set -eu
procscape stop || true
systemctl --user disable --now procscape.service 2>/dev/null || true
rm -f "${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user/procscape.service" "${XDG_DATA_HOME:-$HOME/.local/share}/applications/procscape.desktop"
systemctl --user daemon-reload
cargo uninstall procscape || true
printf '%s\n' 'ProcScape binaries and launchers removed. Wallpaper image and settings were kept.'
