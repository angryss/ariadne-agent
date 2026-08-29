#!/bin/sh
if [ "$1" = "--version" ]; then
  printf '%s\n' '2.1.223 (Claude Code)'
  exit 9
fi
exit 99
