#!/bin/sh
if [ "$1" = "--version" ]; then
  printf '%s\n' "$$" > "$ARIADNE_TEST_PID"
  exec sleep 600
fi
exit 99
