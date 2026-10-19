use crate::output::{Output, Text, bar, dim, gb, pct, rate, round1};
use crate::smart;
use crate::util::{self, Error, Result, read, read_u64, usage};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::path::{Path, PathBuf};
use std::time::Instant;

pub struct Disk {
    pub name: String,
    pub model: String,
    pub size: u64,
    pub kind: &'static str,
    pub os: bool,
}

pub fn run(words: &[String], scan: bool) -> Result<Output> {
    let disks = disks();
    if words.first().is_some_and(|w| w == "list") {
        if words.len() > 1 || scan {
            return Err(usage("disk", "'list' takes no arguments"));
        }
        return list(&disks);
    }
    let (index, rest) = match words.first().and_then(|w| w.parse::<usize>().ok()) {
        Some(n) => (n, &words[1..]),
        None => (1, words),
    };
    let disk = index
        .checked_sub(1)
        .and_then(|i| disks.get(i))
        .ok_or_else(|| Error::Usage(format!("disk {index} does not exist. Run 'status disk list'.")))?;
    match rest {
        [] => capacity(disk, index, scan),
        [_] if scan => Err(usage("disk", "--scan is only valid with 'status disk <n>'")),
        [w] => match w.as_str() {
            "partition" => partition(disk, index),
            "smart" => smart::report(disk, &header(disk, index)),
            "io" => io(disk, index),
            other => Err(usage("disk", format!("unknown subcommand '{other}'"))),
        },
        _ => Err(usage("disk", "too many arguments")),
    }
}

/// Physical disks, numbered from 1: the disk holding '/' first, then by name.
pub fn disks() -> Vec<Disk> {
    let root: HashSet<String> = mounts()
        .iter()
        .find(|m| m.point == "/")
        .map(|m| base_devices(&m.dev).iter().map(|p| parent_disk(p)).collect())
        .unwrap_or_default();
    let mut list: Vec<Disk> = fs::read_dir("/sys/block")
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().join("device").exists())
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let p = e.path();
            let size = read_u64(p.join("size"))? * 512;
            if size == 0 {
                return None;
            }
            let kind = if name.starts_with("nvme") {
                "NVMe"
            } else if read(p.join("queue/rotational")).as_deref() == Some("1") {
                "HDD"
            } else {
                "SSD"
            };
            Some(Disk {
                model: udev_model(&p).or_else(|| read(p.join("device/model"))).unwrap_or_default(),
                os: root.contains(&name),
                name,
                size,
                kind,
            })
        })
        .collect();
    list.sort_by(|a, b| (!a.os, &a.name).cmp(&(!b.os, &b.name)));
    list
}

fn udev_model(sys: &Path) -> Option<String> {
    let dev = read(sys.join("dev"))?;
    fs::read_to_string(format!("/run/udev/data/b{dev}"))
        .ok()?
        .lines()
        .find_map(|l| l.strip_prefix("E:ID_MODEL="))
        .map(|m| m.replace('_', " "))
}

pub fn header(d: &Disk, index: usize) -> String {
    let os = if d.os { "  [OS]" } else { "" };
    format!("[{index}] {}  {}  {}{os}", d.name, d.model, d.kind)
}

fn list(disks: &[Disk]) -> Result<Output> {
    let rows: Vec<Vec<String>> = disks
        .iter()
        .enumerate()
        .map(|(i, d)| {
            vec![
                (i + 1).to_string(),
                d.name.clone(),
                d.model.clone(),
                gb(d.size),
                d.kind.into(),
                if d.os { "[OS]".into() } else { String::new() },
            ]
        })
        .collect();
    let mut t = Text::new();
    t.table(&["#", "Device", "Model", "Size", "Type", ""], &rows);
    let list: Vec<_> = disks
        .iter()
        .enumerate()
        .map(|(i, d)| {
            json!({ "number": i + 1, "device": d.name, "model": d.model, "size": d.size, "type": d.kind, "os": d.os })
        })
        .collect();
    Ok(t.finish(json!(list)))
}

pub struct Mount {
    pub dev: String,
    pub devno: u64,
    pub point: String,
    pub fstype: String,
}

fn unescape(s: &str) -> String {
    s.replace("\\040", " ").replace("\\011", "\t").replace("\\134", "\\")
}

/// One entry per mounted block-device filesystem (first mount point wins).
pub fn mounts() -> Vec<Mount> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for line in fs::read_to_string("/proc/self/mountinfo").unwrap_or_default().lines() {
        let Some((left, right)) = line.split_once(" - ") else { continue };
        let l: Vec<&str> = left.split_whitespace().collect();
        let r: Vec<&str> = right.split_whitespace().collect();
        if l.len() < 5 || r.len() < 2 {
            continue;
        }
        let Some((maj, min)) = l[2].split_once(':') else { continue };
        let (Ok(maj), Ok(min)) = (maj.parse::<u64>(), min.parse::<u64>()) else { continue };
        // Block devices normally show their own major; btrfs uses an anonymous one.
        let dev = fs::read_link(format!("/sys/dev/block/{maj}:{min}"))
            .ok()
            .filter(|_| maj != 0)
            .or_else(|| r[1].starts_with("/dev/").then(|| fs::canonicalize(r[1]).ok()).flatten())
            .and_then(|p| p.file_name()?.to_str().map(String::from));
        let Some(dev) = dev else { continue };
        let devno = util::makedev(maj, min);
        if seen.insert(devno) {
            out.push(Mount { dev, devno, point: unescape(l[4]), fstype: r[0].into() });
        }
    }
    out
}

/// Resolves stacked devices (LVM, LUKS, md) down to the partitions or disks below them.
fn base_devices(name: &str) -> Vec<String> {
    let slaves: Vec<String> = fs::read_dir(format!("/sys/class/block/{name}/slaves"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    if slaves.is_empty() {
        vec![name.to_string()]
    } else {
        slaves.iter().flat_map(|s| base_devices(s)).collect()
    }
}

fn parent_disk(name: &str) -> String {
    let p = PathBuf::from(format!("/sys/class/block/{name}"));
    if p.join("partition").exists()
        && let Some(parent) = fs::canonicalize(&p).ok().and_then(|c| {
            c.parent()?.file_name()?.to_str().map(String::from)
        }) {
            return parent;
        }
    name.to_string()
}

/// Partitions of a disk; a disk without a partition table is its own single entry.
fn partitions(disk: &str) -> Vec<(String, u64)> {
    let mut parts: Vec<(String, u64)> = fs::read_dir(format!("/sys/block/{disk}"))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().join("partition").exists())
        .filter_map(|e| {
            let size = read_u64(e.path().join("size"))? * 512;
            Some((e.file_name().into_string().ok()?, size))
        })
        .collect();
    parts.sort_by_key(|(n, _)| (n.len(), n.clone()));
    if parts.is_empty() {
        parts.push((disk.to_string(), read_u64(format!("/sys/block/{disk}/size")).unwrap_or(0) * 512));
    }
    parts
}

fn disk_mounts(disk: &str) -> Vec<Mount> {
    mounts()
        .into_iter()
        .filter(|m| base_devices(&m.dev).iter().any(|b| parent_disk(b) == disk))
        .collect()
}

#[derive(Default, Clone, Copy)]
pub struct Space {
    pub total: u64,
    pub used: u64,
    pub avail: u64,
    pub reserved: u64,
}

pub fn statvfs(path: &str) -> Option<Space> {
    let c = std::ffi::CString::new(path).ok()?;
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut s) } != 0 {
        return None;
    }
    let f = s.f_frsize;
    let (total, free, avail) = (s.f_blocks * f, s.f_bfree * f, s.f_bavail * f);
    Some(Space { total, used: total - free, avail, reserved: free - avail })
}

/// Summed filesystem usage of the disk's mounted filesystems.
pub fn usage_of(disk: &str) -> Space {
    disk_mounts(disk).iter().filter_map(|m| statvfs(&m.point)).fold(Space::default(), |a, s| Space {
        total: a.total + s.total,
        used: a.used + s.used,
        avail: a.avail + s.avail,
        reserved: a.reserved + s.reserved,
    })
}

struct Held {
    total: u64,
    by_process: Vec<(String, u64)>,
    partial: bool,
}

/// Space of deleted files that running processes still keep open.
fn held_open(devnos: &HashSet<u64>) -> Held {
    let mut seen = HashSet::new();
    let mut by_process: HashMap<String, u64> = HashMap::new();
    let (mut total, mut partial) = (0, false);
    for pid in util::pids() {
        let entries = match fs::read_dir(format!("/proc/{pid}/fd")) {
            Ok(e) => e,
            Err(e) => {
                partial |= util::is_denied(&e);
                continue;
            }
        };
        for e in entries.flatten() {
            let Ok(target) = fs::read_link(e.path()) else { continue };
            let target = target.to_string_lossy();
            if !target.ends_with(" (deleted)") || target.starts_with("/memfd:") {
                continue;
            }
            let Ok(meta) = fs::metadata(e.path()) else { continue };
            if !meta.is_file() || !devnos.contains(&meta.dev()) {
                continue;
            }
            if seen.insert((meta.dev(), meta.ino())) {
                let bytes = meta.blocks() * 512;
                total += bytes;
                *by_process.entry(util::comm(pid)).or_default() += bytes;
            }
        }
    }
    let mut by_process: Vec<(String, u64)> = by_process.into_iter().filter(|(_, b)| *b > 0).collect();
    by_process.sort_by_key(|p| std::cmp::Reverse(p.1));
    Held { total, by_process, partial }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct FiemapExtent {
    logical: u64,
    physical: u64,
    length: u64,
    reserved64: [u64; 2],
    flags: u32,
    reserved: [u32; 3],
}

const EXTENTS: usize = 64;

#[repr(C)]
struct Fiemap {
    start: u64,
    length: u64,
    flags: u32,
    mapped: u32,
    count: u32,
    reserved: u32,
    extents: [FiemapExtent; EXTENTS],
}

const FS_IOC_FIEMAP: libc::c_ulong = 0xC020_660B;
const FIEMAP_EXTENT_LAST: u32 = 0x1;
const FIEMAP_EXTENT_UNWRITTEN: u32 = 0x800;

/// Bytes of a file that are allocated but never written (preallocated).
fn unwritten(path: &Path) -> std::io::Result<u64> {
    let c = std::ffi::CString::new(path.as_os_str().as_encoded_bytes())?;
    let fd = unsafe { libc::open(c.as_ptr(), libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let mut map: Box<Fiemap> = Box::new(unsafe { std::mem::zeroed() });
    let (mut start, mut sum) = (0u64, 0u64);
    let result = loop {
        map.start = start;
        map.length = u64::MAX - start;
        map.flags = 0;
        map.count = EXTENTS as u32;
        map.mapped = 0;
        if unsafe { libc::ioctl(fd, FS_IOC_FIEMAP as _, &mut *map as *mut Fiemap) } != 0 {
            break Err(std::io::Error::last_os_error());
        }
        if map.mapped == 0 {
            break Ok(sum);
        }
        let ext = &map.extents[..map.mapped as usize];
        for e in ext {
            if e.flags & FIEMAP_EXTENT_UNWRITTEN != 0 {
                sum += e.length;
            }
        }
        let last = ext[ext.len() - 1];
        if last.flags & FIEMAP_EXTENT_LAST != 0 {
            break Ok(sum);
        }
        start = last.logical + last.length;
    };
    unsafe { libc::close(fd) };
    result
}

struct Scan {
    bytes: u64,
    files: u64,
    denied: u64,
}

/// Walks a filesystem without crossing into other devices and sums unwritten extents.
fn scan_preallocated(root: &str, devno: u64, scan: &mut Scan) {
    let mut stack = vec![PathBuf::from(root)];
    while let Some(dir) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) => {
                scan.denied += u64::from(util::is_denied(&e));
                continue;
            }
        };
        for e in entries.flatten() {
            let Ok(meta) = fs::symlink_metadata(e.path()) else { continue };
            if meta.dev() != devno {
                continue;
            }
            let ft = meta.file_type();
            if ft.is_dir() {
                stack.push(e.path());
            } else if ft.is_file() && meta.blocks() > 0 && !ft.is_fifo() {
                // Filesystems without FIEMAP (e.g. vfat) simply contribute nothing.
                match unwritten(&e.path()) {
                    Ok(b) => {
                        scan.bytes += b;
                        scan.files += u64::from(b > 0);
                    }
                    Err(e) => scan.denied += u64::from(util::is_denied(&e)),
                }
            }
        }
    }
}

fn capacity(d: &Disk, index: usize, scan: bool) -> Result<Output> {
    let fs_mounts = disk_mounts(&d.name);
    let space = usage_of(&d.name);
    let devnos: HashSet<u64> = fs_mounts.iter().map(|m| m.devno).collect();
    let held = held_open(&devnos);
    let p = if space.total == 0 { 0.0 } else { space.used as f64 * 100.0 / space.total as f64 };

    let mut t = Text::new();
    t.title(&header(d, index)).kv("Size", gb(d.size));
    let mut j = json!({ "number": index, "device": d.name, "size": d.size });
    if fs_mounts.is_empty() {
        t.kv("Filesystem", dim("not mounted"));
    } else {
        t.kv("Filesystem", format!("{}   {}", gb(space.total), dim(&format!("{} mounted", fs_mounts.len()))))
            .kv("Used", format!("{}   {} {}", gb(space.used), bar(p), pct(p)))
            .kv("Free", gb(space.avail))
            .kv("Reserved", format!("{}   {}", gb(space.reserved), dim("root-only filesystem reserve")));
        j["filesystem"] = json!({
            "total": space.total, "used": space.used, "free": space.avail,
            "reserved": space.reserved, "used_percent": round1(p),
        });
    }

    let top: Vec<String> = held.by_process.iter().filter(|(_, b)| *b >= 10_000_000).take(3).map(|(n, b)| format!("{n} {}", gb(*b))).collect();
    let mut held_text = gb(held.total);
    if !top.is_empty() {
        held_text += &format!("   {}", top.join(", "));
    }
    if held.partial {
        held_text += &format!("   {}", dim(&format!("own processes only; {}", util::permission_hint())));
    }
    t.kv("Held open", held_text);
    j["held_open"] = json!({
        "total": held.total,
        "partial": held.partial,
        "processes": held.by_process.iter().map(|(n, b)| json!({ "name": n, "bytes": b })).collect::<Vec<_>>(),
    });

    if scan {
        let mut s = Scan { bytes: 0, files: 0, denied: 0 };
        for m in &fs_mounts {
            scan_preallocated(&m.point, m.devno, &mut s);
        }
        let mut text = format!("{}   {}", gb(s.bytes), dim(&format!("{} files", util::thousands(s.files))));
        if s.denied > 0 {
            text += &format!(
                "   {}",
                dim(&format!("{} unreadable skipped; {}", util::thousands(s.denied), util::permission_hint()))
            );
        }
        t.kv("Preallocated", text);
        j["preallocated"] = json!({ "total": s.bytes, "files": s.files, "denied": s.denied });
    }
    Ok(t.finish(j))
}

fn swap_devices() -> HashSet<String> {
    fs::read_to_string("/proc/swaps")
        .unwrap_or_default()
        .lines()
        .skip(1)
        .filter_map(|l| l.split_whitespace().next())
        .filter_map(|f| fs::canonicalize(f).ok()?.file_name()?.to_str().map(String::from))
        .flat_map(|n| base_devices(&n))
        .collect()
}

fn partition(d: &Disk, index: usize) -> Result<Output> {
    let all = mounts();
    let swaps = swap_devices();
    let mut rows = Vec::new();
    let mut list = Vec::new();
    for (name, size) in partitions(&d.name) {
        let on_part: Vec<&Mount> = all.iter().filter(|m| base_devices(&m.dev).contains(&name)).collect();
        if on_part.is_empty() {
            let note = if swaps.contains(&name) { "[swap]" } else { "not mounted" };
            rows.push(vec![name.clone(), gb(size), "-".into(), dim(note), String::new(), String::new(), String::new()]);
            list.push(json!({ "partition": name, "size": size, "mount": null, "swap": swaps.contains(&name) }));
            continue;
        }
        for (i, m) in on_part.iter().enumerate() {
            let s = statvfs(&m.point).unwrap_or_default();
            let p = if s.total == 0 { 0.0 } else { s.used as f64 * 100.0 / s.total as f64 };
            let (pname, psize) = if i == 0 { (name.clone(), gb(size)) } else { (String::new(), String::new()) };
            rows.push(vec![pname, psize, m.fstype.clone(), m.point.clone(), gb(s.used), gb(s.avail), pct(p)]);
            list.push(json!({
                "partition": name, "size": size, "filesystem": m.fstype, "mount": m.point,
                "used": s.used, "free": s.avail, "used_percent": round1(p),
            }));
        }
    }
    let mut t = Text::new();
    t.title(&header(d, index))
        .table(&["Partition", "Size", "Filesystem", "Mount", "Used", "Free", "Use"], &rows);
    Ok(t.finish(json!(list)))
}

fn read_io(name: &str) -> Result<Vec<u64>> {
    let s = read(format!("/sys/block/{name}/stat"))
        .ok_or_else(|| Error::Unavailable("no I/O statistics".into()))?;
    Ok(s.split_whitespace().filter_map(|v| v.parse().ok()).collect())
}

fn io(d: &Disk, index: usize) -> Result<Output> {
    let a = read_io(&d.name)?;
    let start = Instant::now();
    std::thread::sleep(util::SAMPLE);
    let b = read_io(&d.name)?;
    let secs = start.elapsed().as_secs_f64();
    if a.len() < 10 || b.len() < 10 {
        return Err(Error::Unavailable("unexpected I/O statistics format".into()));
    }
    let delta = |i: usize| b[i].saturating_sub(a[i]) as f64;
    let (read_bps, write_bps) = (delta(2) * 512.0 / secs, delta(6) * 512.0 / secs);
    let (read_iops, write_iops) = (delta(0) / secs, delta(4) / secs);
    let busy = (delta(9) / (secs * 1000.0) * 100.0).min(100.0);
    let mut t = Text::new();
    t.title(&header(d, index))
        .kv("Read", format!("{:>10}   {:.0} IOPS", rate(read_bps), read_iops))
        .kv("Write", format!("{:>10}   {:.0} IOPS", rate(write_bps), write_iops))
        .kv("Busy", format!("{}   {}", bar(busy), pct(busy)))
        .kv("Since boot", format!("read {}, written {}", gb(b[2] * 512), gb(b[6] * 512)));
    Ok(t.finish(json!({
        "read_bytes_per_sec": read_bps.round(), "write_bytes_per_sec": write_bps.round(),
        "read_iops": round1(read_iops), "write_iops": round1(write_iops), "busy_percent": round1(busy),
        "read_total": b[2] * 512, "written_total": b[6] * 512,
    })))
}
