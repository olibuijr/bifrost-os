#!/usr/bin/env bash
# Run one root command in the live installer VM over its serial socket (autologin root shell).
# usage: vm-live-sh.sh 'cmd' [wait_seconds] [tail_lines]
SOCK="$(cd "$(dirname "$0")/.." && pwd)/out/arch-vm/installer-serial.sock"
(printf '%s\n' "$1 2>&1 | cut -c1-200; echo __END__"; sleep "${2:-4}") | timeout $((${2:-4}+6)) socat -t$((${2:-4}+2)) - UNIX-CONNECT:"$SOCK" \
  | tr -d '\033' | sed 's/\[[0-9;?]*[a-zA-Z]//g' | grep -a -v '^$' | tail -"${3:-25}"
