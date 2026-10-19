use crate::output::{Output, Text, bar, dim, gb, pct, round1};
use crate::util::{self, Error, Result, usage};
use serde_json::json;
use std::collections::HashMap;

pub fn run(words: &[String]) -> Result<Output> {
    match words {
        [] => usage_view(),
        [w] => match w.as_str() {
            "usage" => usage_view(),
            "swap" => swap(),
            "cache" => cache(),
            "process" => process(),
            "info" => info(),
            other => Err(usage("memory", format!("unknown subcommand '{other}'"))),
        },
        _ => Err(usage("memory", "too many arguments")),
    }
}

/// /proc/meminfo values in bytes.
pub fn meminfo() -> HashMap<String, u64> {
    std::fs::read_to_string("/proc/meminfo")
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let (k, v) = l.split_once(':')?;
            let kb: u64 = v.split_whitespace().next()?.parse().ok()?;
            Some((k.to_string(), kb * 1024))
        })
        .collect()
}

/// Returns (total, used) where used = total - available.
pub fn used() -> (u64, u64) {
    let m = meminfo();
    let total = m.get("MemTotal").copied().unwrap_or(0);
    (total, total.saturating_sub(m.get("MemAvailable").copied().unwrap_or(0)))
}

fn ratio(part: u64, whole: u64) -> f64 {
    if whole == 0 { 0.0 } else { part as f64 * 100.0 / whole as f64 }
}

fn usage_view() -> Result<Output> {
    let m = meminfo();
    let get = |k: &str| m.get(k).copied().unwrap_or(0);
    let (total, free, avail) = (get("MemTotal"), get("MemFree"), get("MemAvailable"));
    let used = total.saturating_sub(avail);
    let p = ratio(used, total);
    let mut t = Text::new();
    t.kv("Total", gb(total))
        .kv("Used", format!("{}   {} {}", gb(used), bar(p), pct(p)))
        .kv("Free", gb(free))
        .kv("Available", format!("{}   {}", gb(avail), dim("free + reclaimable cache")));
    Ok(t.finish(json!({
        "total": total, "used": used, "free": free, "available": avail, "used_percent": round1(p),
    })))
}

fn swap() -> Result<Output> {
    let m = meminfo();
    let total = m.get("SwapTotal").copied().unwrap_or(0);
    let used = total.saturating_sub(m.get("SwapFree").copied().unwrap_or(0));
    let p = ratio(used, total);
    let mut t = Text::new();
    if total == 0 {
        t.line("No swap configured");
        return Ok(t.finish(json!({ "total": 0, "devices": [] })));
    }
    t.kv("Total", gb(total))
        .kv("Used", format!("{}   {} {}", gb(used), bar(p), pct(p)))
        .kv("Free", gb(total - used))
        .line("");
    let mut rows = Vec::new();
    let mut devices = Vec::new();
    for line in std::fs::read_to_string("/proc/swaps").unwrap_or_default().lines().skip(1) {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 5 {
            continue;
        }
        let size = f[2].parse::<u64>().unwrap_or(0) * 1024;
        let used = f[3].parse::<u64>().unwrap_or(0) * 1024;
        rows.push(vec![f[0].into(), f[1].into(), gb(size), gb(used), f[4].into()]);
        devices.push(json!({ "name": f[0], "type": f[1], "size": size, "used": used, "priority": f[4] }));
    }
    t.table(&["Name", "Type", "Size", "Used", "Priority"], &rows);
    Ok(t.finish(json!({ "total": total, "used": used, "free": total - used, "devices": devices })))
}

fn cache() -> Result<Output> {
    let m = meminfo();
    let get = |k: &str| m.get(k).copied().unwrap_or(0);
    let fields = [
        ("Buffers", "buffers", get("Buffers")),
        ("Page cache", "page_cache", get("Cached")),
        ("Shared", "shared", get("Shmem")),
        ("Slab reclaim", "slab_reclaimable", get("SReclaimable")),
        ("Dirty", "dirty", get("Dirty")),
    ];
    let reclaimable = get("MemAvailable").saturating_sub(get("MemFree"));
    let mut t = Text::new();
    let mut obj = serde_json::Map::new();
    for (label, key, v) in fields {
        t.kv(label, gb(v));
        obj.insert(key.into(), json!(v));
    }
    t.kv("Reclaimable", format!("{}   {}", gb(reclaimable), dim("can be freed on demand")));
    obj.insert("reclaimable".into(), json!(reclaimable));
    Ok(t.finish(obj.into()))
}

fn process() -> Result<Output> {
    let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) } as u64;
    let (total, _) = used();
    let mut procs: Vec<(u32, String, u64)> = util::pids()
        .into_iter()
        .filter_map(|pid| {
            let statm = util::read(format!("/proc/{pid}/statm"))?;
            let rss: u64 = statm.split_whitespace().nth(1)?.parse().ok()?;
            Some((pid, util::comm(pid), rss * page))
        })
        .filter(|p| p.2 > 0)
        .collect();
    procs.sort_by_key(|p| std::cmp::Reverse(p.2));
    procs.truncate(10);
    let rows: Vec<Vec<String>> = procs
        .iter()
        .map(|(pid, name, rss)| vec![pid.to_string(), name.clone(), gb(*rss), pct(ratio(*rss, total))])
        .collect();
    let mut t = Text::new();
    t.table(&["PID", "Name", "Memory", "Share"], &rows);
    let list: Vec<_> = procs
        .iter()
        .map(|(pid, name, rss)| json!({ "pid": pid, "name": name, "resident": rss }))
        .collect();
    Ok(t.finish(json!(list)))
}

struct Module {
    slot: String,
    size_mb: u64,
    kind: String,
    speed: u16,
    configured: u16,
    vendor: String,
    part: String,
}

fn smbios_string(strings: &[&[u8]], index: u8) -> String {
    match index {
        0 => String::new(),
        i => strings
            .get(i as usize - 1)
            .map(|s| String::from_utf8_lossy(s).trim().to_string())
            .unwrap_or_default(),
    }
}

fn memory_type(code: u8) -> &'static str {
    match code {
        0x12 => "DDR",
        0x13 => "DDR2",
        0x18 => "DDR3",
        0x1A => "DDR4",
        0x1B => "LPDDR",
        0x1C => "LPDDR2",
        0x1D => "LPDDR3",
        0x1E => "LPDDR4",
        0x22 => "DDR5",
        0x23 => "LPDDR5",
        _ => "unknown",
    }
}

/// Parses SMBIOS type 17 (Memory Device) structures.
fn modules(table: &[u8]) -> Vec<Module> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 4 <= table.len() {
        let (kind, len) = (table[i], table[i + 1] as usize);
        if len < 4 || i + len > table.len() {
            break;
        }
        let s = &table[i..i + len];
        // String set follows the formatted area and ends with a double NUL.
        let mut end = i + len;
        while end + 1 < table.len() && !(table[end] == 0 && table[end + 1] == 0) {
            end += 1;
        }
        let strings: Vec<&[u8]> = table[i + len..end].split(|&b| b == 0).collect();
        let word = |o: usize| if o + 1 < len { u16::from_le_bytes([s[o], s[o + 1]]) } else { 0 };
        if kind == 17 && len >= 0x15 {
            let raw = word(0x0C);
            let size_mb = match raw {
                0 | 0xFFFF => 0,
                0x7FFF if len >= 0x20 => u32::from_le_bytes([s[0x1C], s[0x1D], s[0x1E], s[0x1F]]) as u64,
                r if r & 0x8000 != 0 => (r & 0x7FFF) as u64 / 1024,
                r => r as u64,
            };
            let byte = |o: usize| if o < len { s[o] } else { 0 };
            out.push(Module {
                slot: smbios_string(&strings, byte(0x10)),
                size_mb,
                kind: memory_type(byte(0x12)).into(),
                speed: word(0x15),
                configured: word(0x20),
                vendor: smbios_string(&strings, byte(0x17)),
                part: smbios_string(&strings, byte(0x1A)),
            });
        }
        if kind == 127 {
            break;
        }
        i = end + 2;
    }
    out
}

fn info() -> Result<Output> {
    let table = std::fs::read("/sys/firmware/dmi/tables/DMI")?;
    let mods = modules(&table);
    if mods.is_empty() {
        return Err(Error::Unavailable("no SMBIOS memory devices".into()));
    }
    let installed: u64 = mods.iter().map(|m| m.size_mb).sum();
    let mut rows = Vec::new();
    let mut list = Vec::new();
    for m in &mods {
        if m.size_mb == 0 {
            rows.push(vec![m.slot.clone(), dim("empty"), String::new(), String::new(), String::new()]);
        } else {
            let speed = if m.configured > 0 && m.configured != m.speed {
                format!("{} MT/s (rated {})", m.configured, m.speed)
            } else {
                format!("{} MT/s", m.speed)
            };
            rows.push(vec![
                m.slot.clone(),
                format!("{} GB", m.size_mb / 1024),
                m.kind.clone(),
                speed,
                format!("{} {}", m.vendor, m.part),
            ]);
        }
        list.push(json!({
            "slot": m.slot, "size_mb": m.size_mb, "type": m.kind, "speed_mts": m.speed,
            "configured_mts": m.configured, "manufacturer": m.vendor, "part_number": m.part,
        }));
    }
    let used_slots = mods.iter().filter(|m| m.size_mb > 0).count();
    let mut t = Text::new();
    t.kv("Installed", format!("{} GB in {used_slots} of {} slots", installed / 1024, mods.len()))
        .line("")
        .table(&["Slot", "Size", "Type", "Speed", "Module"], &rows);
    Ok(t.finish(json!({ "installed_mb": installed, "modules": list })))
}
