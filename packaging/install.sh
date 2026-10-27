#!/bin/sh
set -eu
cargo install --path . --features gui --force
mkdir -p "${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user" "${XDG_DATA_HOME:-$HOME/.local/share}/applications"
sed "s|%h/.cargo/bin/procscape|$(command -v procscape)|" packaging/procscape.service > "${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user/procscape.service"
cp packaging/procscape.desktop "${XDG_DATA_HOME:-$HOME/.local/share}/applications/procscape.desktop"
systemctl --user daemon-reload
printf '%s\n' 'Installed. Start with: procscape start' 'Optional: systemctl --user enable --now procscape.service'
