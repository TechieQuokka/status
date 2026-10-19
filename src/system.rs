use crate::output::{Output, Text, dim};
use crate::util::{self, Error, Result, read, usage};
use serde_json::json;

pub fn run(words: &[String]) -> Result<Output> {
    match words {
        [] => info(),
        [w] => match w.as_str() {
            "info" => info(),
            "uptime" => uptime(),
            "board" => board(),
            "user" => user(),
            "service" => service(),
            "update" => update(),
            other => Err(usage("system", format!("unknown subcommand '{other}'"))),
        },
        _ => Err(usage("system", "too many arguments")),
    }
}

pub fn hostname() -> String {
    read("/proc/sys/kernel/hostname").unwrap_or_default()
}

fn distribution() -> String {
    std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|s| {
            s.lines()
                .find_map(|l| l.strip_prefix("PRETTY_NAME="))
                .map(|v| v.trim_matches('"').to_string())
        })
        .unwrap_or_else(|| "unknown".into())
}

pub fn uptime_secs() -> u64 {
    read("/proc/uptime")
        .and_then(|s| s.split_whitespace().next()?.parse::<f64>().ok())
        .unwrap_or(0.0) as u64
}

fn info() -> Result<Output> {
    let (host, distro) = (hostname(), distribution());
    let kernel = read("/proc/sys/kernel/osrelease").unwrap_or_default();
    let arch = std::env::consts::ARCH;
    let mut t = Text::new();
    t.kv("Hostname", &host)
        .kv("Distribution", &distro)
        .kv("Kernel", &kernel)
        .kv("Architecture", arch);
    Ok(t.finish(json!({
        "hostname": host, "distribution": distro, "kernel": kernel, "architecture": arch,
    })))
}

fn uptime() -> Result<Output> {
    let secs = uptime_secs();
    let boot = std::fs::read_to_string("/proc/stat")
        .ok()
        .and_then(|s| s.lines().find_map(|l| l.strip_prefix("btime ")?.trim().parse::<i64>().ok()))
        .unwrap_or(0);
    let mut t = Text::new();
    t.kv("Uptime", util::duration(secs))
        .kv("Boot time", util::local_time(boot));
    Ok(t.finish(json!({ "uptime_seconds": secs, "boot_time": boot })))
}

fn board() -> Result<Output> {
    let dmi = |f: &str| read(format!("/sys/class/dmi/id/{f}")).unwrap_or_else(|| "-".into());
    let fields = [
        ("System", "system", format!("{} {}", dmi("sys_vendor"), dmi("product_name"))),
        ("Board", "board", format!("{} {}", dmi("board_vendor"), dmi("board_name"))),
        ("BIOS", "bios_version", format!("{} {}", dmi("bios_vendor"), dmi("bios_version"))),
        ("BIOS date", "bios_date", dmi("bios_date")),
    ];
    let mut t = Text::new();
    let mut obj = serde_json::Map::new();
    for (label, key, value) in fields {
        t.kv(label, &value);
        obj.insert(key.into(), json!(value));
    }
    Ok(t.finish(obj.into()))
}

fn user() -> Result<Output> {
    let out = util::run("who", &[]).ok_or_else(|| Error::Unavailable("'who' failed".into()))?;
    let mut rows = Vec::new();
    let mut list = Vec::new();
    for line in out.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 4 {
            continue;
        }
        let from = f.get(4).map(|s| s.trim_matches(|c| c == '(' || c == ')')).unwrap_or("");
        let since = format!("{} {}", f[2], f[3]);
        rows.push(vec![f[0].to_string(), f[1].to_string(), since.clone(), from.to_string()]);
        list.push(json!({ "user": f[0], "terminal": f[1], "since": since, "from": from }));
    }
    let mut t = Text::new();
    if rows.is_empty() {
        t.line("No logged-in sessions");
    } else {
        t.table(&["User", "Terminal", "Since", "From"], &rows);
    }
    Ok(t.finish(json!(list)))
}

fn service() -> Result<Output> {
    let out = util::run(
        "systemctl",
        &["--failed", "--plain", "--no-legend", "--no-pager"],
    )
    .ok_or_else(|| Error::Unavailable("'systemctl' failed".into()))?;
    let units: Vec<(String, String)> = out
        .lines()
        .filter_map(|l| {
            let mut f = l.split_whitespace();
            let unit = f.next()?.to_string();
            let desc = f.skip(3).collect::<Vec<_>>().join(" ");
            Some((unit, desc))
        })
        .collect();
    let mut t = Text::new();
    if units.is_empty() {
        t.line("No failed services");
    } else {
        t.title(&format!("{} failed", units.len()));
        let rows: Vec<Vec<String>> = units.iter().map(|(u, d)| vec![u.clone(), d.clone()]).collect();
        t.table(&["Unit", "Description"], &rows);
    }
    let list: Vec<_> = units.iter().map(|(u, d)| json!({ "unit": u, "description": d })).collect();
    Ok(t.finish(json!({ "failed": list })))
}

fn update() -> Result<Output> {
    let out = util::run(
        "apt-get",
        &["-s", "-o", "Debug::NoLocking=1", "upgrade"],
    )
    .ok_or_else(|| Error::Unavailable("'apt-get' failed".into()))?;
    let installs: Vec<&str> = out.lines().filter(|l| l.starts_with("Inst ")).collect();
    let security = installs.iter().filter(|l| l.contains("-security")).count();
    let mut t = Text::new();
    t.kv("Upgradable", installs.len().to_string())
        .kv("Security", security.to_string())
        .line(&dim("Based on local package lists ('sudo apt update' to refresh)"));
    Ok(t.finish(json!({ "upgradable": installs.len(), "security": security })))
}
