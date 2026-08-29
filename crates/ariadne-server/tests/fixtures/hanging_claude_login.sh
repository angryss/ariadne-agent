#!/bin/sh
printf '%s\n' "$$" > "$0.pid"
exec sleep 600
