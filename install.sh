#!/usr/bin/env bash
# Builds and installs status, then grants the capabilities it needs so that
# every value can be read without sudo:
#   cap_sys_rawio        SMART via ATA pass-through (status disk smart)
#   cap_dac_read_search  root-only files: RAPL energy, SMBIOS table, /dev/sdX
#   cap_sys_ptrace       other users' /proc/<pid>/fd (held open, connections)
# Re-run this after every rebuild: replacing the binary drops its capabilities.
set -euo pipefail

cd "$(dirname "$0")"

bin="${CARGO_HOME:-$HOME/.cargo}/bin/status"
caps="cap_sys_rawio,cap_dac_read_search,cap_sys_ptrace+ep"

cargo install --path . --force

echo "Granting capabilities to $bin (sudo password may be asked)"
sudo setcap "$caps" "$bin"

getcap "$bin"
"$bin" --version
