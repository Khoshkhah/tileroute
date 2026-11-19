#!/bin/bash
# Run a command; stop it when the free disk space goes under GUARD_GB (default 30) GB.
"$@" &
pid=$!
while kill -0 $pid 2>/dev/null; do
  free=$(df -g / | awk "NR==2{print \$4}")
  if [ "$free" -lt "${GUARD_GB:-30}" ]; then echo "GUARD: only $free GB free, stop" >&2; kill -TERM $pid; break; fi
  sleep 2
done
wait $pid
