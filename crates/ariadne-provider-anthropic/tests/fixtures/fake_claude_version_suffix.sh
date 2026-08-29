#!/bin/sh
if [ "$1" = "--version" ]; then
  printf '%s\n' '2.1.223 (Claude Code)-unreviewed'
  exit 0
fi
exit 99
