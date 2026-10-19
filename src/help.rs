use crate::output::{Output, bold, dim};

pub fn overview() -> Output {
    render(&format!(
        "{} — show current system status (v{})

{}
  status                 Summary of every group
  status <command>       Representative view of a group
  status <command> help  Every subcommand of a group

{}
  disk      Storage devices (SSD, HDD, NVMe)
  cpu       Processor usage, temperature, fan, power
  memory    RAM and swap
  network   Network interfaces and connections
  gpu       NVIDIA graphics card
  system    OS, kernel, uptime, board, services

{}
  --json         Print JSON instead of text
  -h, --help     Same as 'help'
  -V, --version  Print version

Run 'status <command> help' for details.
",
        bold("status"),
        env!("CARGO_PKG_VERSION"),
        bold("Usage"),
        bold("Commands"),
        bold("Options"),
    ))
}

pub fn group(name: &str) -> Output {
    let (title, usage, notes, examples) = match name {
        "disk" => (
            "status disk — storage devices",
            "  status disk                    Same as 'status disk 1'
  status disk list               All disks with their numbers
  status disk <n>                Capacity: size, used, free, reserved, held open
  status disk <n> --scan         Capacity plus preallocated (unwritten) space
  status disk <n> partition      Per-partition usage and mount points
  status disk <n> smart          Health, temperature, wear, data written (root)
  status disk <n> io             Current read/write rate and IOPS",
            "  Disk 1 is always the disk that holds '/'. Others follow by device name.
  Omitting <n> means disk 1, e.g. 'status disk smart' = 'status disk 1 smart'.
  'Held open' is space of deleted files still kept open by running apps.
  '--scan' walks every file on the disk's filesystems and can take a while.",
            "  status disk list
  status disk 2 partition
  sudo status disk 1 --scan",
        ),
        "cpu" => (
            "status cpu — processor",
            "  status cpu                     Same as 'status cpu usage'
  status cpu usage               Total usage (%) and load average
  status cpu core                Per-core usage (%)
  status cpu temperature         Package and per-core temperature
  status cpu info                Model, cores/threads, clocks, cache
  status cpu fan                 Fan speeds (RPM), if a sensor exposes them
  status cpu process             Top processes by CPU usage
  status cpu power               Package power (W) via Intel RAPL (root)",
            "  Usage and power are measured over 0.5 s and printed once.
  Process usage is per core: 100% means one full core.",
            "  status cpu core
  sudo status cpu power",
        ),
        "memory" => (
            "status memory — RAM and swap",
            "  status memory                  Same as 'status memory usage'
  status memory usage            Total, used, free, available
  status memory swap             Swap total/used/free per device or file
  status memory cache            Buffers, page cache, shared, reclaimable
  status memory process          Top processes by resident memory
  status memory info             RAM modules: slot, size, type, speed (root)",
            "  'Available' is what apps can still get: free memory plus cache the
  kernel can drop. It is a better measure than 'free'.",
            "  status memory
  sudo status memory info",
        ),
        "network" => (
            "status network — network interfaces",
            "  status network                 Same as 'status network 1'
  status network list            All interfaces with their numbers
  status network <n|name>        Addresses, MAC, link speed, MTU
  status network <n|name> traffic  Current RX/TX rate and totals since boot
  status network wifi            SSID, signal, frequency
  status network connection      Open TCP/UDP sockets and their processes
  status network public          Public IP (contacts the internet)",
            "  Interface 1 is always the one carrying the default route.
  Others follow by name. An interface can be given by number or name.
  'connection' shows other users' processes only when run as root.",
            "  status network list
  status network enp3s0 traffic
  status network 2",
        ),
        "gpu" => (
            "status gpu — NVIDIA graphics card",
            "  status gpu                     Same as 'status gpu 1'
  status gpu list                All GPUs with their numbers
  status gpu <n>                 Usage: GPU core, memory controller, encoder, decoder
  status gpu <n> memory          VRAM total, used, free
  status gpu <n> temperature     Temperature and throttle thresholds
  status gpu <n> fan             Fan speed (%)
  status gpu <n> power           Current power draw and limit (W)
  status gpu <n> info            Model, driver, CUDA, PCIe link, clocks
  status gpu <n> process         Processes using VRAM",
            "  Requires the NVIDIA proprietary driver (NVML).
  Omitting <n> means GPU 1, e.g. 'status gpu memory' = 'status gpu 1 memory'.",
            "  status gpu memory
  status gpu 1 process",
        ),
        _ => (
            "status system — operating system",
            "  status system                  Same as 'status system info'
  status system info             Hostname, distribution, kernel, architecture
  status system uptime           Uptime and boot time
  status system board            Motherboard and BIOS
  status system user             Logged-in users and sessions
  status system service          Failed systemd services
  status system update           Upgradable apt packages",
            "  'update' uses the local package lists; run 'sudo apt update' to refresh them.",
            "  status system uptime
  status system update",
        ),
    };
    render(&format!(
        "{}\n\n{}\n{usage}\n\n{}\n{}\n\n{}\n{examples}\n",
        bold(title),
        bold("Usage"),
        bold("Notes"),
        dim(notes),
        bold("Examples"),
    ))
}

fn render(text: &str) -> Output {
    Output {
        text: text
            .lines()
            .map(|l| format!(" {l}\n"))
            .collect::<String>(),
        json: serde_json::json!({ "help": text }),
    }
}
