#!/bin/sh
[ "$1" = "--version" ] && { printf '%s\n' 'codex-cli 0.149.1'; exit 0; }
printf launched > "$CODEX_HOME/launched"
exit 9
