#!/bin/sh
# Flatpak entry (218): the home's own dirs, not ~/.var/app/…: the settings, tabs, trash,
# thumbnails and the rclone we install are the same as a .deb install's. $PATH is the host's:
# every program we start runs there (`core::host`). The host's data dirs for .desktop files and
# icons, under /run/host.
export XDG_CONFIG_HOME="$HOME/.config" XDG_DATA_HOME="$HOME/.local/share" \
    XDG_STATE_HOME="$HOME/.local/state" XDG_CACHE_HOME="$HOME/.cache"
host_path=$(flatpak-spawn --host printenv PATH) && export PATH="$host_path"
host_data=$(flatpak-spawn --host printenv XDG_DATA_DIRS) || host_data=
host_data=$(printf %s "${host_data:-/usr/local/share:/usr/share}" | sed 's,\(^\|:\)/usr,\1/run/host/usr,g')
export XDG_DATA_DIRS="$XDG_DATA_DIRS:$host_data"
exec /app/bin/shagoff-commander "$@"
