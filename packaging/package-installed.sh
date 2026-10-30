#!/bin/sh
# Package transactions run outside the user's desktop session. Update the
# app-menu cache without enabling, starting, or stopping any user's service.
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database /usr/share/applications >/dev/null 2>&1 || true
fi
exit 0
