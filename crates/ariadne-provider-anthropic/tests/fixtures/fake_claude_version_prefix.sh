#!/bin/sh
if [ "$1" = "--version" ]; then
  printf '%s\n' '2.1.223-unreviewed'
  exit 0
fi
exit 99
