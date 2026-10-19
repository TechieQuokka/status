use crate::output::{Output, Text, accent, dim, gb, pct, round1};
use crate::util::{self, Result};
use crate::{cpu, disk, gpu, memory, network, system};
use serde_json::json;

pub fn run() -> Result<Output> {
    let mut t = Text::new();
    let mut j = serde_json::Map::new();
    let mut row = |group: &str, text: String, value: serde_json::Value| {
        t.line(&format!("{}  {text}", accent(&format!("{group:<8}"))));
        j.insert(group.into(), value);
    };

    match disk::disks().first() {
        Some(d) => {
            let s = disk::usage_of(&d.name);
            let p = if s.total == 0 { 0.0 } else { s.used as f64 * 100.0 / s.total as f64 };
            row(
                "disk",
                format!("{:<10}{} / {}  {}", d.name, gb(s.used), gb(s.total), dim(&format!("({})", pct(p)))),
                json!({ "device": d.name, "used": s.used, "total": s.total, "used_percent": round1(p) }),
            );
        }
        None => row("disk", "-".into(), json!(null)),
    }

    let usage = cpu::total_usage();
    let temp = cpu::package_temperature();
    row(
        "cpu",
        format!("{:<10}{}", pct(usage), temp.map(|c| format!("{c:.0}°C")).unwrap_or_default()),
        json!({ "usage_percent": round1(usage), "temperature_celsius": temp }),
    );

    let (total, used) = memory::used();
    let p = if total == 0 { 0.0 } else { used as f64 * 100.0 / total as f64 };
    row(
        "memory",
        format!("{} / {}  {}", gb(used), gb(total), dim(&format!("({})", pct(p)))),
        json!({ "used": used, "total": total, "used_percent": round1(p) }),
    );

    match network::interfaces().into_iter().next() {
        Some(f) => {
            let ip = network::ipv4(&network::addresses(), &f.name);
            row(
                "network",
                format!("{:<10}{}", f.name, ip.clone().unwrap_or("-".into())),
                json!({ "interface": f.name, "ipv4": ip }),
            );
        }
        None => row("network", "-".into(), json!(null)),
    }

    match gpu::init().ok().as_ref().and_then(gpu::brief) {
        Some((name, u, c)) => row(
            "gpu",
            format!("{name}   {}   {c}°C", pct(u as f64)),
            json!({ "name": name, "usage_percent": u, "temperature_celsius": c }),
        ),
        None => row("gpu", dim("not available"), json!(null)),
    }

    let up = system::uptime_secs();
    row(
        "system",
        format!("{:<10}up {}", system::hostname(), util::duration(up)),
        json!({ "hostname": system::hostname(), "uptime_seconds": up }),
    );

    Ok(t.finish(j.into()))
}
