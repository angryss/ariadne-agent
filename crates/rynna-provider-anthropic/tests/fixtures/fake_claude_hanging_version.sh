#!/bin/sh
if [ "$1" = "--version" ]; then
  printf '%s\n' "$$" > "$RYNNA_TEST_PID"
  exec sleep 600
fi
exit 99
