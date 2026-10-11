#!/bin/bash
# Deploy the root shell files to Midget. Existing files are kept as <name>.prev.
set -euo pipefail
dir="$(cd "$(dirname "$0")" && pwd)"
for f in .bashrc .bash_profile; do
  scp -q -o BatchMode=yes "$dir/$f" "midget:/root/$f.new"
done
ssh -o BatchMode=yes midget 'cd /root && for f in .bashrc .bash_profile; do [ -e $f ] && cp -p $f $f.prev; mv $f.new $f; chmod 644 $f; done; ls -la /root/.bashrc /root/.bash_profile'
