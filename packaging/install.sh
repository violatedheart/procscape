#!/bin/sh
set -eu

usage() {
    printf '%s\n' \
        'Usage: sh packaging/install.sh [--start]' \
        '' \
        'Build and install ProcScape, its app launcher, and its user service.' \
        '  --start     Also enable wallpaper updates now and at login.' \
        '  -h, --help  Show this help.' \
        '' \
        'Run as your regular desktop user. Install build dependencies from README.md first.'
}

start_service=false
for option in "$@"; do
    case "$option" in
        --start) start_service=true ;;
        -h|--help) usage; exit 0 ;;
        *) printf 'Unknown option: %s\n' "$option" >&2; usage >&2; exit 2 ;;
    esac
done

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
cd "$script_dir/.."
for tool in cargo pkg-config systemctl; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        printf 'Missing %s. Install the dependencies listed in README.md, then run this installer again.\n' "$tool" >&2
        exit 1
    fi
done
if ! pkg-config --exists gtk4; then
    printf '%s\n' 'GTK 4 development files are missing. Install gtk4-devel (Fedora) or libgtk-4-dev (Debian), then try again.' >&2
    exit 1
fi
if ! systemctl --user show-environment >/dev/null 2>&1; then
    printf '%s\n' 'Cannot connect to your desktop user service manager. Run this installer from your logged-in desktop session, without sudo.' >&2
    exit 1
fi
printf '%s\n' 'Building and installing ProcScape. The first build may take a few minutes.'
install_root="${CARGO_INSTALL_ROOT:-${CARGO_HOME:-$HOME/.cargo}}"
# Use the same explicit root for installation and launchers. Cargo's bin
# directory need not be present in the caller's PATH.
cargo install --locked --path . --features gui --force --root "$install_root"
install_root=$(CDPATH= cd -- "$install_root" && pwd)
mkdir -p "${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user" "${XDG_DATA_HOME:-$HOME/.local/share}/applications"
# Quote executable paths in both launcher formats and escape sed replacements.
launcher_root=$(printf '%s' "$install_root" | sed 's/[\\"`$]/\\&/g; s/%/%%/g')
replacement_root=$(printf '%s' "$launcher_root" | sed 's/[\\&|]/\\&/g')
sed "s|^ExecStart=.*|ExecStart=\"$replacement_root/bin/procscape\" __daemon|" packaging/procscape.service > "${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user/procscape.service"
sed "s|^Exec=.*|Exec=\"$replacement_root/bin/procscape-gui\"|" packaging/procscape.desktop > "${XDG_DATA_HOME:-$HOME/.local/share}/applications/procscape.desktop"
systemctl --user daemon-reload
if [ "$start_service" = true ]; then
    # Move a manually started daemon under systemd, and load the new binary
    # when reinstalling an already-running service.
    if [ -x "$install_root/bin/procscape" ]; then
        "$install_root/bin/procscape" stop
    fi
    systemctl --user enable --now procscape.service
fi
printf '\n%s\n' 'Installed. Open ProcScape from your application menu.'
if [ "$start_service" = true ]; then
    printf '%s\n' 'Wallpaper service enabled now and at login. Use Start / resume in the app if updates were paused.'
else
    printf '%s\n' 'Choose Start / resume in the app to begin. Turn on Start on login in its settings for automatic startup.'
fi
printf '%s\n' "Optional terminal access: add $install_root/bin to PATH."
