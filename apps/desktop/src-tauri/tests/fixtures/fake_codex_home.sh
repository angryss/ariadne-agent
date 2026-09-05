#!/bin/sh
marker_directory=${CODEX_HOME%/*}
printf '%s' "$CODEX_HOME" > "$marker_directory/codex-home"
[ "$1" = "app-server" ] || exit 2
IFS= read -r initialize
printf '%s\n' '{"id":1,"result":{"userAgent":"fake"}}'
IFS= read -r initialized
IFS= read -r account
printf '%s\n' '{"id":2,"result":{"account":{"type":"apiKey"},"requiresOpenaiAuth":true}}'
