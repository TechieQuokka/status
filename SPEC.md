# status — Specification

Version: 1.1.0

A Rust CLI that prints the **current** state of this machine. Every command
takes a one-shot snapshot and exits; nothing refreshes continuously.

Target: the author's own Linux PC only (Ubuntu, Intel CPU, NVIDIA GPU).
Vendor-specific implementations are acceptable.

## Common rules

- Command names are full words; widely known abbreviations stay as-is
  (`cpu`, `gpu`, `io`, `smart`).
- Each group is split into subcommands. Omitting the subcommand shows the
  group's representative view.
- Groups with multiple devices use numbers. Device `1` is fixed (see each
  group); the rest are ordered by name. Omitting the number means `1`.
- Values that need sampling (usage, rates, power) are measured over a short
  interval (~0.5 s) and printed once.
- Sizes are shown in decimal GB. Rates are shown in MB/s.
- `--json` prints the same data as JSON instead of text.
- Color is used only when stdout is a terminal (and `NO_COLOR` is unset).
- Privileged values are read through file capabilities granted by
  `install.sh` (`cap_sys_rawio`, `cap_dac_read_search`, `cap_sys_ptrace`), so
  no sudo is needed. If a read is actually denied, the value is shown as
  `requires permission (run install.sh)`; everything else is still shown.
- If a sensor does not exist, the value is shown as `not available`.

## Help

Help is hierarchical. `help`, `--help` and `-h` are equivalent.

- `status help` — overview: one line per group.
- `status <group> help` — every subcommand of the group with usage and examples.

`--version` / `-V` prints the version.

## Command tree

### status

Summary: one line per group (disk 1 usage, CPU usage and temperature, memory
usage, network 1 address, GPU 1 usage and temperature, uptime).

### status disk

Disk 1 is the physical disk that holds `/`. Other disks follow by device name.

| Command | Output |
|---|---|
| `status disk` | Same as `status disk 1` |
| `status disk list` | Number, device, model, size, type, `[OS]` marker |
| `status disk <n>` | Capacity: size, used, free, root-reserved, held open by deleted files (with process names) |
| `status disk <n> --scan` | Above plus preallocated-but-unwritten space (walks the filesystems) |
| `status disk <n> partition` | Per-partition filesystem, mount point, size, used, free |
| `status disk <n> smart` | Health, temperature, power-on hours, wear, data written |
| `status disk <n> io` | Current read/write rate and IOPS |
| `status disk help` | Help |

### status cpu

| Command | Output |
|---|---|
| `status cpu` | Same as `status cpu usage` |
| `status cpu usage` | Total usage (%) |
| `status cpu core` | Per-core usage (%) |
| `status cpu temperature` | Package and per-core temperature |
| `status cpu info` | Model, cores/threads, base/max clock, cache |
| `status cpu fan` | Fan speeds (RPM) if any sensor exposes them |
| `status cpu process` | Top processes by CPU usage |
| `status cpu power` | Package power (W) via Intel RAPL |
| `status cpu help` | Help |

### status memory

| Command | Output |
|---|---|
| `status memory` | Same as `status memory usage` |
| `status memory usage` | Total, used, free, available |
| `status memory swap` | Swap total/used/free, per device or file |
| `status memory cache` | Buffers, page cache, shared, reclaimable |
| `status memory process` | Top processes by resident memory |
| `status memory info` | RAM modules: slot, size, type, speed |
| `status memory help` | Help |

### status network

Interface 1 is the one carrying the default route. Others follow by name.
An interface can be given by number or by name (numeric → number).

| Command | Output |
|---|---|
| `status network` | Same as `status network 1` |
| `status network list` | Number, name, state, IPv4, kind, `[default]` marker |
| `status network <n\|name>` | IPv4/IPv6, MAC, link speed, MTU |
| `status network <n\|name> traffic` | Current RX/TX rate and totals since boot |
| `status network wifi` | SSID, signal, frequency |
| `status network connection` | Open TCP/UDP sockets with owning process |
| `status network public` | Public IP (the only command that contacts the internet) |
| `status network help` | Help |

### status gpu

NVIDIA only (NVML). GPU 1 is NVML device index 0.

| Command | Output |
|---|---|
| `status gpu` | Same as `status gpu 1` |
| `status gpu list` | Number, model, driver version |
| `status gpu <n>` | Usage: GPU core and memory controller |
| `status gpu <n> memory` | VRAM total, used, free |
| `status gpu <n> temperature` | Temperature |
| `status gpu <n> fan` | Fan speed (%) |
| `status gpu <n> power` | Current power draw and limit (W) |
| `status gpu <n> info` | Model, driver, CUDA version, PCIe link, clocks |
| `status gpu <n> process` | Processes using VRAM |
| `status gpu help` | Help |

### status system

| Command | Output |
|---|---|
| `status system` | Same as `status system info` |
| `status system info` | Hostname, distribution, kernel, architecture |
| `status system uptime` | Uptime and boot time |
| `status system board` | Motherboard vendor/model, BIOS version/date |
| `status system user` | Logged-in users and sessions |
| `status system service` | Failed systemd services |
| `status system update` | Number of upgradable apt packages |
| `status system help` | Help |

## Implementation notes

- Data comes from `/proc`, `/sys` and ioctls wherever possible.
- External programs are used only where no stable kernel interface exists:
  `systemctl` (services), `who` (sessions), `apt-get` (updates).
- SMART on SATA uses ATA PASS-THROUGH via `SG_IO`.
- Preallocated space uses `FIEMAP` and sums `FIEMAP_EXTENT_UNWRITTEN` extents.
- RAM module info parses SMBIOS type 17 from `/sys/firmware/dmi/tables/DMI`.
- Crates: `libc`, `serde`, `serde_json`, `nvml-wrapper`, `ureq`.
