#!/bin/sh
set -eu
install_root="${CARGO_INSTALL_ROOT:-${CARGO_HOME:-$HOME/.cargo}}"
# Use the installed binary directly: Cargo's bin directory may not be in PATH.
if [ -x "$install_root/bin/procscape" ]; then
    "$install_root/bin/procscape" restore || true
    "$install_root/bin/procscape" stop || true
fi
systemctl --user disable --now procscape.service 2>/dev/null || true
rm -f "${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user/procscape.service" "${XDG_DATA_HOME:-$HOME/.local/share}/applications/procscape.desktop"
systemctl --user daemon-reload
cargo uninstall --root "$install_root" procscape || true
printf '%s\n' 'ProcScape binaries and launchers removed. Saved settings and personal wallpaper backups were kept.'
