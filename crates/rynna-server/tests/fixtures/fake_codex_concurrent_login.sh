#!/bin/sh
[ "$1" = "login" ] && [ "$2" = "--with-api-key" ] || exit 2
IFS= read -r key
printf '%s\n' "$key" >> "$CODEX_HOME/../login.log"
sleep 0.2
