# status

A Rust CLI that prints a one-shot snapshot of this machine's state: disks,
CPU, memory, network, GPU and system.

```
$ status
 disk      sda       1842.8 GB / 1967.9 GB  (94%)
 cpu       36%       65°C
 memory    8.73 GB / 25.0 GB  (35%)
 network   enp3s0    192.168.0.18
 gpu       NVIDIA GeForce RTX 3060   17%   58°C
 system    Jarvis    up 3h 55m
```

Built for the author's own PC (Linux, Intel CPU, NVIDIA GPU). Other hardware
may work partially; values that cannot be read show `not available`.

## Install

Requires Rust 1.85+ (edition 2024).

```sh
./install.sh
```

This runs `cargo install --path . --force` and then grants the binary the
capabilities it needs (asks for the sudo password once). See
[Permissions](#permissions). Re-run it after every rebuild.

The NVIDIA proprietary driver is needed for `status gpu` (NVML is loaded at
runtime; everything else works without it).

## Usage

Every command prints the current state once and exits. Omitting a subcommand
shows the group's representative view, and omitting a number means `1`.

```
status                          Summary of every group
status help                     Overview
status <group> help             Every subcommand of a group
```

| Group | Subcommands | Default |
|---|---|---|
| `disk [n]` | `list`, `partition`, `smart`, `io`, `--scan` | capacity of disk 1 |
| `cpu` | `usage`, `core`, `temperature`, `info`, `fan`, `process`, `power` | `usage` |
| `memory` | `usage`, `swap`, `cache`, `process`, `info` | `usage` |
| `network [n\|name]` | `list`, `traffic`, `wifi`, `connection`, `public` | interface 1 detail |
| `gpu [n]` | `list`, `memory`, `temperature`, `fan`, `power`, `info`, `process` | usage of GPU 1 |
| `system` | `info`, `uptime`, `board`, `user`, `service`, `update` | `info` |

Numbering:

- **Disk 1** is always the disk that holds `/`; the others follow by device name.
- **Interface 1** is always the one carrying the default route; the others
  follow by name. Interfaces can also be given by name (`status network enp3s0`).

Examples:

```sh
status disk list
status disk 2 partition
status disk --scan           # also measure preallocated (unwritten) space
status cpu core
status network traffic
status gpu process
status memory --json
```

### Options

| Option | Effect |
|---|---|
| `--json` | Print JSON instead of text |
| `--scan` | With `status disk [n]`: walk the filesystems to find preallocated space (slow) |
| `-h`, `--help` | Same as `help` |
| `-V`, `--version` | Print version |

Color is used only when stdout is a terminal and `NO_COLOR` is unset.

### Disk capacity

`status disk` shows, besides size/used/free:

- **Reserved** — blocks the filesystem keeps for root (ext4 default 5%).
- **Held open** — space of deleted files that running processes still keep
  open; it is freed only when those processes close them.
- **Preallocated** (`--scan`) — space reserved with `fallocate` but never
  written, found via `FIEMAP` unwritten extents.

### Permissions

Some values normally need root: `disk smart`, `cpu power`, `memory info`, and
the full view of `disk` held-open space and `network connection` owners.
Instead of sudo, `install.sh` grants the binary three file capabilities:

| Capability | Used for |
|---|---|
| `cap_sys_rawio` | SMART via ATA pass-through |
| `cap_dac_read_search` | Root-only files: RAPL energy, SMBIOS table, `/dev/sdX` |
| `cap_sys_ptrace` | Other users' `/proc/<pid>/fd` (held open, connections) |

With them, every command works as a normal user. Without them (e.g. after a
plain `cargo install`), those values show `requires permission (run
install.sh)` and everything else is still shown.

Replacing the binary drops its capabilities, so re-run `./install.sh` after
every rebuild. Check them with `getcap ~/.cargo/bin/status`.

## How it works

Data is read from `/proc`, `/sys` and ioctls; external programs are used only
where no stable kernel interface exists.

| Data | Source |
|---|---|
| Disk usage, partitions, I/O | `/proc/self/mountinfo`, `statvfs`, `/sys/block` |
| Held open / preallocated | `/proc/*/fd`, `FS_IOC_FIEMAP` |
| SMART (SATA) | ATA PASS-THROUGH (16) over `SG_IO` |
| CPU usage, temperature, power | `/proc/stat`, `coretemp` hwmon, Intel RAPL |
| Memory, RAM modules | `/proc/meminfo`, SMBIOS type 17 |
| Network | `getifaddrs`, `/sys/class/net`, `/proc/net/*` |
| GPU | NVML (`nvml-wrapper`) |
| Services, sessions, updates | `systemctl`, `who`, `apt-get -s` |
| Public IP | `api.ipify.org` (the only network request) |

See [SPEC.md](SPEC.md) for the full specification.

## License

No license has been chosen yet.
