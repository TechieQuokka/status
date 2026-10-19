use crate::output::{Output, Text, bar, dim, pct, round1};
use crate::util::{self, Error, Result, read, read_u64, usage};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::time::Instant;

pub fn run(words: &[String]) -> Result<Output> {
    match words {
        [] => usage_view(),
        [w] => match w.as_str() {
            "usage" => usage_view(),
            "core" => core(),
            "temperature" => temperature(),
            "info" => info(),
            "fan" => fan(),
            "process" => process(),
            "power" => power(),
            other => Err(usage("cpu", format!("unknown subcommand '{other}'"))),
        },
        _ => Err(usage("cpu", "too many arguments")),
    }
}

/// Jiffies of one /proc/stat cpu line: (busy, total, [user, system, iowait]).
#[derive(Clone, Copy, Default)]
struct Times {
    busy: u64,
    total: u64,
    user: u64,
    system: u64,
    iowait: u64,
}

fn read_stat() -> Vec<(String, Times)> {
    std::fs::read_to_string("/proc/stat")
        .unwrap_or_default()
        .lines()
        .filter(|l| l.starts_with("cpu"))
        .filter_map(|l| {
            let mut f = l.split_whitespace();
            let name = f.next()?.to_string();
            let v: Vec<u64> = f.take(8).filter_map(|x| x.parse().ok()).collect();
            if v.len() < 8 {
                return None;
            }
            let total: u64 = v.iter().sum();
            let idle = v[3] + v[4];
            Some((name, Times {
                busy: total - idle,
                total,
                user: v[0] + v[1],
                system: v[2] + v[5] + v[6],
                iowait: v[4],
            }))
        })
        .collect()
}

/// Samples /proc/stat twice and returns usage per line ("cpu", "cpu0", ...) as
/// (total %, user %, system %, iowait %).
fn sample() -> Vec<(String, [f64; 4])> {
    let a = read_stat();
    std::thread::sleep(util::SAMPLE);
    let b = read_stat();
    a.iter()
        .zip(b.iter())
        .map(|((name, x), (_, y))| {
            let dt = (y.total - x.total).max(1) as f64;
            let p = |a: u64, b: u64| (b - a) as f64 * 100.0 / dt;
            (name.clone(), [p(x.busy, y.busy), p(x.user, y.user), p(x.system, y.system), p(x.iowait, y.iowait)])
        })
        .collect()
}

pub fn total_usage() -> f64 {
    sample().first().map(|(_, v)| v[0]).unwrap_or(0.0)
}

fn usage_view() -> Result<Output> {
    let s = sample();
    let [total, user, system, iowait] = s.first().map(|(_, v)| *v).unwrap_or_default();
    let load = read("/proc/loadavg").unwrap_or_default();
    let load: Vec<&str> = load.split_whitespace().take(3).collect();
    let mut t = Text::new();
    t.kv("Usage", format!("{}   {}", bar(total), pct(total)))
        .kv("User", pct(user))
        .kv("System", pct(system))
        .kv("I/O wait", pct(iowait))
        .kv("Load", format!("{}   {}", load.join("  "), dim("1, 5, 15 min")));
    Ok(t.finish(json!({
        "usage_percent": round1(total), "user_percent": round1(user),
        "system_percent": round1(system), "iowait_percent": round1(iowait),
        "load": load.iter().filter_map(|l| l.parse::<f64>().ok()).collect::<Vec<_>>(),
    })))
}

fn core() -> Result<Output> {
    let s = sample();
    let mut rows = Vec::new();
    let mut list = Vec::new();
    for (name, v) in s.iter().skip(1) {
        let id = name.trim_start_matches("cpu");
        rows.push(vec![id.to_string(), bar(v[0]), pct(v[0])]);
        list.push(json!({ "cpu": id.parse::<u32>().unwrap_or(0), "usage_percent": round1(v[0]) }));
    }
    let mut t = Text::new();
    t.table(&["CPU", "Usage", ""], &rows);
    Ok(t.finish(json!(list)))
}

/// Returns coretemp readings as (label, °C), package first.
pub fn temperatures() -> Vec<(String, f64)> {
    let mut out = Vec::new();
    for dir in util::hwmon_dirs() {
        if read(dir.join("name")).as_deref() != Some("coretemp") {
            continue;
        }
        let mut readings: Vec<(u32, String, f64)> = std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let file = e.file_name().into_string().ok()?;
                let n: u32 = file.strip_prefix("temp")?.strip_suffix("_input")?.parse().ok()?;
                let milli = read_u64(e.path())?;
                let label = read(dir.join(format!("temp{n}_label"))).unwrap_or(format!("temp{n}"));
                Some((n, label, milli as f64 / 1000.0))
            })
            .collect();
        readings.sort_by_key(|r| r.0);
        out.extend(readings.into_iter().map(|(_, l, c)| (l, c)));
    }
    out
}

pub fn package_temperature() -> Option<f64> {
    let temps = temperatures();
    temps
        .iter()
        .find(|(l, _)| l.starts_with("Package"))
        .or(temps.first())
        .map(|(_, c)| *c)
}

fn temperature() -> Result<Output> {
    let temps = temperatures();
    if temps.is_empty() {
        return Err(Error::Unavailable("no coretemp sensor".into()));
    }
    let mut t = Text::new();
    for (label, c) in &temps {
        t.kv(label, format!("{c:.0}°C"));
    }
    let list: Vec<_> = temps.iter().map(|(l, c)| json!({ "label": l, "celsius": c })).collect();
    Ok(t.finish(json!(list)))
}

fn cache_sizes() -> Vec<(String, String)> {
    let base = "/sys/devices/system/cpu/cpu0/cache";
    let mut out: Vec<(String, String)> = std::fs::read_dir(base)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("index"))
        .filter_map(|e| {
            let p = e.path();
            let level = read(p.join("level"))?;
            let kind = match read(p.join("type"))?.as_str() {
                "Data" => "d",
                "Instruction" => "i",
                _ => "",
            };
            let kb: u64 = read(p.join("size"))?.trim_end_matches('K').parse().ok()?;
            let size = if kb >= 1024 { format!("{} MB", round1(kb as f64 / 1024.0)) } else { format!("{kb} KB") };
            Some((format!("L{level}{kind}"), size))
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

fn info() -> Result<Output> {
    let cpuinfo = std::fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
    let field = |key: &str| {
        cpuinfo
            .lines()
            .find_map(|l| {
                let (k, v) = l.split_once(':')?;
                (k.trim() == key).then(|| v.trim().to_string())
            })
            .unwrap_or_default()
    };
    let (model, vendor) = (field("model name"), field("vendor_id"));
    let mut threads = 0;
    let mut cores = HashSet::new();
    let mut phys = String::new();
    for l in cpuinfo.lines() {
        let Some((k, v)) = l.split_once(':') else { continue };
        match k.trim() {
            "processor" => threads += 1,
            "physical id" => phys = v.trim().into(),
            "core id" => {
                cores.insert((phys.clone(), v.trim().to_string()));
            }
            _ => {}
        }
    }
    let freq = |f: &str| read_u64(format!("/sys/devices/system/cpu/cpu0/cpufreq/{f}"));
    let ghz = |khz: Option<u64>| khz.map(|k| format!("{:.2} GHz", k as f64 / 1e6)).unwrap_or("-".into());
    let current: Vec<u64> = (0..threads)
        .filter_map(|i| read_u64(format!("/sys/devices/system/cpu/cpu{i}/cpufreq/scaling_cur_freq")))
        .collect();
    let avg = (!current.is_empty()).then(|| current.iter().sum::<u64>() / current.len() as u64);
    let governor = read("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor").unwrap_or("-".into());
    let caches = cache_sizes();
    let mut t = Text::new();
    t.kv("Model", &model)
        .kv("Vendor", &vendor)
        .kv("Cores", format!("{} cores / {threads} threads", cores.len()))
        .kv("Base clock", ghz(freq("base_frequency")))
        .kv("Max clock", ghz(freq("cpuinfo_max_freq")))
        .kv("Current", format!("{}   {}", ghz(avg), dim("average")))
        .kv("Governor", &governor)
        .kv("Cache", caches.iter().map(|(l, s)| format!("{l} {s}")).collect::<Vec<_>>().join(", "));
    Ok(t.finish(json!({
        "model": model, "vendor": vendor, "cores": cores.len(), "threads": threads,
        "base_khz": freq("base_frequency"), "max_khz": freq("cpuinfo_max_freq"),
        "current_khz": avg, "governor": governor,
        "cache": caches.iter().map(|(l, s)| json!({ "level": l, "size": s })).collect::<Vec<_>>(),
    })))
}

fn fan() -> Result<Output> {
    let mut fans = Vec::new();
    for dir in util::hwmon_dirs() {
        let chip = read(dir.join("name")).unwrap_or_default();
        for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let file = e.file_name().to_string_lossy().into_owned();
            let Some(n) = file.strip_prefix("fan").and_then(|f| f.strip_suffix("_input")) else {
                continue;
            };
            let Some(rpm) = read_u64(e.path()) else { continue };
            let label = read(dir.join(format!("fan{n}_label"))).unwrap_or(format!("fan{n}"));
            fans.push((chip.clone(), label, rpm));
        }
    }
    if fans.is_empty() {
        return Err(Error::Unavailable("no fan sensor exposed by the kernel".into()));
    }
    fans.sort();
    let rows: Vec<Vec<String>> = fans
        .iter()
        .map(|(c, l, r)| vec![c.clone(), l.clone(), format!("{r} RPM")])
        .collect();
    let mut t = Text::new();
    t.table(&["Chip", "Fan", "Speed"], &rows);
    let list: Vec<_> = fans.iter().map(|(c, l, r)| json!({ "chip": c, "fan": l, "rpm": r })).collect();
    Ok(t.finish(json!(list)))
}

fn proc_ticks() -> HashMap<u32, u64> {
    util::pids()
        .into_iter()
        .filter_map(|pid| {
            let stat = read(format!("/proc/{pid}/stat"))?;
            // Fields after the parenthesised comm, which may contain spaces.
            let rest = &stat[stat.rfind(')')? + 2..];
            let f: Vec<&str> = rest.split_whitespace().collect();
            let ticks = f.get(11)?.parse::<u64>().ok()? + f.get(12)?.parse::<u64>().ok()?;
            Some((pid, ticks))
        })
        .collect()
}

fn process() -> Result<Output> {
    let a = proc_ticks();
    let start = Instant::now();
    std::thread::sleep(util::SAMPLE);
    let b = proc_ticks();
    let elapsed = start.elapsed().as_secs_f64();
    let hz = util::clock_ticks();
    let mut procs: Vec<(u32, String, f64)> = b
        .iter()
        .filter_map(|(pid, t)| {
            let d = t.checked_sub(*a.get(pid)?)?;
            Some((*pid, util::comm(*pid), d as f64 / hz / elapsed * 100.0))
        })
        .filter(|p| p.2 > 0.0)
        .collect();
    procs.sort_by(|x, y| y.2.total_cmp(&x.2));
    procs.truncate(10);
    let mut t = Text::new();
    if procs.is_empty() {
        t.line("No process used CPU during the sample");
    } else {
        let rows: Vec<Vec<String>> = procs
            .iter()
            .map(|(pid, n, p)| vec![pid.to_string(), n.clone(), format!("{p:.1}%")])
            .collect();
        t.table(&["PID", "Name", "CPU"], &rows);
    }
    let list: Vec<_> = procs
        .iter()
        .map(|(pid, n, p)| json!({ "pid": pid, "name": n, "cpu_percent": round1(*p) }))
        .collect();
    Ok(t.finish(json!(list)))
}

fn power() -> Result<Output> {
    let base = std::path::Path::new("/sys/class/powercap");
    let mut zones: Vec<std::path::PathBuf> = std::fs::read_dir(base)
        .map_err(|_| Error::Unavailable("no powercap interface".into()))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("intel-rapl:"))
                && p.join("energy_uj").exists()
        })
        .collect();
    zones.sort();
    if zones.is_empty() {
        return Err(Error::Unavailable("no Intel RAPL zone".into()));
    }
    let energy = |z: &std::path::PathBuf| -> Result<u64> {
        let s = std::fs::read_to_string(z.join("energy_uj"))?;
        s.trim().parse().map_err(|_| Error::Failed("bad energy_uj".into()))
    };
    let first: Vec<u64> = zones.iter().map(energy).collect::<Result<_>>()?;
    let start = Instant::now();
    std::thread::sleep(util::SAMPLE);
    let second: Vec<u64> = zones.iter().map(energy).collect::<Result<_>>()?;
    let elapsed = start.elapsed().as_secs_f64();
    let mut t = Text::new();
    let mut list = Vec::new();
    for (i, z) in zones.iter().enumerate() {
        let range = read_u64(z.join("max_energy_range_uj")).unwrap_or(u64::MAX);
        let delta = if second[i] >= first[i] { second[i] - first[i] } else { range - first[i] + second[i] };
        let watts = delta as f64 / 1e6 / elapsed;
        let name = read(z.join("name")).unwrap_or_default();
        let label = match name.as_str() {
            n if n.starts_with("package") => "Package".to_string(),
            "core" => "  Cores".into(),
            "uncore" => "  Uncore (iGPU)".into(),
            "dram" => "  DRAM".into(),
            n => format!("  {n}"),
        };
        let limit = read_u64(z.join("constraint_0_power_limit_uw"))
            .filter(|_| name.starts_with("package"));
        let value = match limit {
            Some(l) => format!("{watts:.1} W   {}", dim(&format!("limit {:.0} W", l as f64 / 1e6))),
            None => format!("{watts:.1} W"),
        };
        t.kv(&label, value);
        list.push(json!({ "zone": name, "watts": round1(watts), "limit_watts": limit.map(|l| l as f64 / 1e6) }));
    }
    Ok(t.finish(json!(list)))
}
