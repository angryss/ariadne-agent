#!/bin/sh
if [ "$1" = "login" ] && [ "$2" = "--with-api-key" ]; then
  IFS= read -r key
  [ "$key" = "test-credential" ]
  exit $?
fi
if [ "$1" = "app-server" ]; then
  IFS= read -r _initialize
  printf '%s\n' '{"id":1,"result":{"userAgent":"fake"}}'
  IFS= read -r _initialized
  IFS= read -r _account
  printf '%s\n' '{"id":2,"result":{"account":{"type":"apiKey"},"requiresOpenaiAuth":true}}'
  exit 0
fi
exit 2
