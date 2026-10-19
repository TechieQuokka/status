use crate::output::{Output, Text, dim, gb, rate};
use crate::util::{self, Error, Result, read, read_u64, usage};
use serde_json::json;
use std::collections::HashMap;
use std::fs;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::Path;
use std::time::{Duration, Instant};

pub struct Interface {
    pub name: String,
    pub state: String,
    pub kind: &'static str,
    pub default: bool,
}

pub fn run(words: &[String]) -> Result<Output> {
    let ifaces = interfaces();
    match words.first().map(String::as_str) {
        Some("list") => return only(words, list(&ifaces)),
        Some("wifi") => return only(words, wifi(&ifaces)),
        Some("connection") => return only(words, connection()),
        Some("public") => return only(words, public()),
        _ => {}
    }
    let (target, rest) = match words.first().map(String::as_str) {
        None | Some("traffic") => ("1", words),
        Some(w) => (w, &words[1..]),
    };
    let (index, iface) = select(&ifaces, target)?;
    match rest {
        [] => detail(iface, index),
        [w] if w == "traffic" => traffic(iface, index),
        [w] => Err(usage("network", format!("unknown subcommand '{w}'"))),
        _ => Err(usage("network", "too many arguments")),
    }
}

fn only(words: &[String], result: Result<Output>) -> Result<Output> {
    if words.len() > 1 {
        return Err(usage("network", format!("'{}' takes no arguments", words[0])));
    }
    result
}

/// Selects an interface by number (numeric) or by name.
fn select<'a>(ifaces: &'a [Interface], target: &str) -> Result<(usize, &'a Interface)> {
    let found = match target.parse::<usize>() {
        Ok(n) => n.checked_sub(1).and_then(|i| ifaces.get(i)).map(|f| (n, f)),
        Err(_) => ifaces.iter().enumerate().find(|(_, f)| f.name == target).map(|(i, f)| (i + 1, f)),
    };
    found.ok_or_else(|| Error::Usage(format!("interface '{target}' does not exist. Run 'status network list'.")))
}

/// Interface carrying the default route (lowest metric) and its gateway.
fn default_route() -> Option<(String, Ipv4Addr)> {
    fs::read_to_string("/proc/net/route")
        .ok()?
        .lines()
        .skip(1)
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            if f.len() < 7 || f[1] != "00000000" || f[7] != "00000000" {
                return None;
            }
            let gw = u32::from_str_radix(f[2], 16).ok()?;
            let metric: u32 = f[6].parse().ok()?;
            Some((metric, f[0].to_string(), Ipv4Addr::from(gw.swap_bytes())))
        })
        .min_by_key(|r| r.0)
        .map(|(_, name, gw)| (name, gw))
}

/// Interfaces (except loopback), numbered from 1: default route first, then by name.
pub fn interfaces() -> Vec<Interface> {
    let default = default_route().map(|r| r.0);
    let mut list: Vec<Interface> = fs::read_dir("/sys/class/net")
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            if name == "lo" {
                return None;
            }
            let p = e.path();
            let kind = if p.join("wireless").exists() {
                "wireless"
            } else if p.join("device").exists() {
                "wired"
            } else {
                "virtual"
            };
            Some(Interface {
                state: read(p.join("operstate")).unwrap_or_default(),
                default: default.as_deref() == Some(name.as_str()),
                name,
                kind,
            })
        })
        .collect();
    list.sort_by(|a, b| (!a.default, &a.name).cmp(&(!b.default, &b.name)));
    list
}

/// Addresses per interface from getifaddrs: (address, prefix length).
pub fn addresses() -> HashMap<String, Vec<(IpAddr, u32)>> {
    let mut out: HashMap<String, Vec<(IpAddr, u32)>> = HashMap::new();
    let mut head: *mut libc::ifaddrs = std::ptr::null_mut();
    if unsafe { libc::getifaddrs(&mut head) } != 0 {
        return out;
    }
    let mut cur = head;
    while !cur.is_null() {
        let ifa = unsafe { &*cur };
        cur = ifa.ifa_next;
        if ifa.ifa_addr.is_null() {
            continue;
        }
        let name = unsafe { std::ffi::CStr::from_ptr(ifa.ifa_name) }.to_string_lossy().into_owned();
        let family = unsafe { (*ifa.ifa_addr).sa_family } as i32;
        let (addr, mask_bits) = match family {
            libc::AF_INET => unsafe {
                let a = &*(ifa.ifa_addr as *const libc::sockaddr_in);
                let m = if !ifa.ifa_netmask.is_null() { (*(ifa.ifa_netmask as *const libc::sockaddr_in)).sin_addr.s_addr.count_ones() } else { 0 };
                (IpAddr::V4(Ipv4Addr::from(u32::from_be(a.sin_addr.s_addr))), m)
            },
            libc::AF_INET6 => unsafe {
                let a = &*(ifa.ifa_addr as *const libc::sockaddr_in6);
                let m = if !ifa.ifa_netmask.is_null() { {
                        (*(ifa.ifa_netmask as *const libc::sockaddr_in6))
                            .sin6_addr
                            .s6_addr
                            .iter()
                            .map(|b| b.count_ones())
                            .sum()
                    } } else { 0 };
                (IpAddr::V6(Ipv6Addr::from(a.sin6_addr.s6_addr)), m)
            },
            _ => continue,
        };
        out.entry(name).or_default().push((addr, mask_bits));
    }
    unsafe { libc::freeifaddrs(head) };
    out
}

pub fn ipv4(addrs: &HashMap<String, Vec<(IpAddr, u32)>>, name: &str) -> Option<String> {
    addrs
        .get(name)?
        .iter()
        .find(|(a, _)| a.is_ipv4())
        .map(|(a, _)| a.to_string())
}

fn list(ifaces: &[Interface]) -> Result<Output> {
    let addrs = addresses();
    let rows: Vec<Vec<String>> = ifaces
        .iter()
        .enumerate()
        .map(|(i, f)| {
            vec![
                (i + 1).to_string(),
                f.name.clone(),
                f.state.clone(),
                ipv4(&addrs, &f.name).unwrap_or("-".into()),
                f.kind.into(),
                if f.default { "[default]".into() } else { String::new() },
            ]
        })
        .collect();
    let mut t = Text::new();
    t.table(&["#", "Name", "State", "IPv4", "Kind", ""], &rows);
    let list: Vec<_> = ifaces
        .iter()
        .enumerate()
        .map(|(i, f)| {
            json!({ "number": i + 1, "name": f.name, "state": f.state,
                    "ipv4": ipv4(&addrs, &f.name), "kind": f.kind, "default": f.default })
        })
        .collect();
    Ok(t.finish(json!(list)))
}

fn header(f: &Interface, index: usize) -> String {
    let mark = if f.default { "  [default]" } else { "" };
    format!("[{index}] {}  {}  {}{mark}", f.name, f.kind, f.state)
}

fn detail(f: &Interface, index: usize) -> Result<Output> {
    let p = Path::new("/sys/class/net").join(&f.name);
    let addrs = addresses();
    let mine = addrs.get(&f.name).cloned().unwrap_or_default();
    let fmt = |v4: bool| -> Vec<String> {
        mine.iter().filter(|(a, _)| a.is_ipv4() == v4).map(|(a, m)| format!("{a}/{m}")).collect()
    };
    let (v4, v6) = (fmt(true), fmt(false));
    let mac = read(p.join("address")).unwrap_or("-".into());
    // speed reads -1 or fails when the link is down or virtual.
    let speed = read(p.join("speed")).and_then(|s| s.parse::<i64>().ok()).filter(|s| *s > 0);
    let mtu = read(p.join("mtu")).unwrap_or("-".into());
    let driver = fs::read_link(p.join("device/driver"))
        .ok()
        .and_then(|d| d.file_name()?.to_str().map(String::from));
    let gateway = default_route().filter(|r| r.0 == f.name).map(|r| r.1.to_string());

    let mut t = Text::new();
    t.title(&header(f, index))
        .kv("IPv4", if v4.is_empty() { "-".into() } else { v4.join(", ") });
    for (i, a) in v6.iter().enumerate() {
        t.kv(if i == 0 { "IPv6" } else { "" }, a);
    }
    if let Some(g) = &gateway {
        t.kv("Gateway", g);
    }
    t.kv("MAC", &mac)
        .kv("Speed", speed.map(|s| format!("{s} Mb/s")).unwrap_or("-".into()))
        .kv("MTU", &mtu)
        .kv("Driver", driver.clone().unwrap_or("-".into()));
    Ok(t.finish(json!({
        "number": index, "name": f.name, "state": f.state, "kind": f.kind, "default": f.default,
        "ipv4": v4, "ipv6": v6, "gateway": gateway, "mac": mac, "speed_mbps": speed,
        "mtu": mtu.parse::<u32>().ok(), "driver": driver,
    })))
}

fn traffic(f: &Interface, index: usize) -> Result<Output> {
    let stat = |k: &str| read_u64(format!("/sys/class/net/{}/statistics/{k}", f.name)).unwrap_or(0);
    let (rx0, tx0) = (stat("rx_bytes"), stat("tx_bytes"));
    let start = Instant::now();
    std::thread::sleep(util::SAMPLE);
    let (rx1, tx1) = (stat("rx_bytes"), stat("tx_bytes"));
    let secs = start.elapsed().as_secs_f64();
    let (rx_rate, tx_rate) = ((rx1 - rx0) as f64 / secs, (tx1 - tx0) as f64 / secs);
    let errors = stat("rx_errors") + stat("tx_errors");
    let drops = stat("rx_dropped") + stat("tx_dropped");
    let mut t = Text::new();
    t.title(&header(f, index))
        .kv("Receive", format!("{:>10}   {}", rate(rx_rate), dim(&format!("total {}", gb(rx1)))))
        .kv("Transmit", format!("{:>10}   {}", rate(tx_rate), dim(&format!("total {}", gb(tx1)))))
        .kv("Errors", util::thousands(errors))
        .kv("Dropped", util::thousands(drops));
    Ok(t.finish(json!({
        "rx_bytes_per_sec": rx_rate.round(), "tx_bytes_per_sec": tx_rate.round(),
        "rx_total": rx1, "tx_total": tx1, "errors": errors, "dropped": drops,
    })))
}

fn wifi(ifaces: &[Interface]) -> Result<Output> {
    let Some(f) = ifaces.iter().find(|f| f.kind == "wireless") else {
        return Err(Error::Unavailable("no wireless interface".into()));
    };
    let link = util::run("iw", &["dev", &f.name, "link"]).unwrap_or_default();
    let field = |key: &str| {
        link.lines().find_map(|l| l.trim().strip_prefix(key).map(|v| v.trim().to_string()))
    };
    let (ssid, freq, signal) = (field("SSID:"), field("freq:"), field("signal:"));
    let mut t = Text::new();
    t.title(&format!("{}  {}", f.name, f.state));
    if ssid.is_none() {
        t.line("Not connected");
    } else {
        t.kv("SSID", ssid.clone().unwrap_or_default())
            .kv("Frequency", freq.as_ref().map(|v| format!("{v} MHz")).unwrap_or("-".into()))
            .kv("Signal", signal.clone().unwrap_or("-".into()));
    }
    Ok(t.finish(json!({ "interface": f.name, "ssid": ssid, "frequency_mhz": freq, "signal": signal })))
}

/// Maps socket inodes to the owning (pid, name); the flag is set when some
/// processes could not be inspected for lack of permission.
fn socket_owners() -> (HashMap<u64, (u32, String)>, bool) {
    let mut out = HashMap::new();
    let mut partial = false;
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
            let t = target.to_string_lossy();
            if let Some(inode) = t.strip_prefix("socket:[").and_then(|s| s.strip_suffix(']'))
                && let Ok(inode) = inode.parse() {
                    out.entry(inode).or_insert_with(|| (pid, util::comm(pid)));
                }
        }
    }
    (out, partial)
}

/// Decodes "0100007F:0035" (IPv4) or a 32-digit IPv6 form from /proc/net.
fn decode(s: &str) -> Option<String> {
    let (addr, port) = s.split_once(':')?;
    let port = u16::from_str_radix(port, 16).ok()?;
    let ip = match addr.len() {
        8 => IpAddr::V4(Ipv4Addr::from(u32::from_str_radix(addr, 16).ok()?.swap_bytes())),
        32 => {
            let mut bytes = [0u8; 16];
            for i in 0..4 {
                let word = u32::from_str_radix(&addr[i * 8..i * 8 + 8], 16).ok()?.swap_bytes();
                bytes[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
            }
            let v6 = Ipv6Addr::from(bytes);
            v6.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(IpAddr::V6(v6))
        }
        _ => return None,
    };
    Some(match ip {
        IpAddr::V6(v6) => format!("[{v6}]:{port}"),
        v4 => format!("{v4}:{port}"),
    })
}

fn tcp_state(code: &str) -> &'static str {
    match code {
        "01" => "ESTABLISHED",
        "02" => "SYN_SENT",
        "03" => "SYN_RECV",
        "04" => "FIN_WAIT1",
        "05" => "FIN_WAIT2",
        "06" => "TIME_WAIT",
        "07" => "CLOSE",
        "08" => "CLOSE_WAIT",
        "09" => "LAST_ACK",
        "0A" => "LISTEN",
        "0B" => "CLOSING",
        _ => "UNKNOWN",
    }
}

fn connection() -> Result<Output> {
    let (owners, partial) = socket_owners();
    let mut conns = Vec::new();
    for proto in ["tcp", "tcp6", "udp", "udp6"] {
        let Ok(content) = fs::read_to_string(format!("/proc/net/{proto}")) else { continue };
        for line in content.lines().skip(1) {
            let f: Vec<&str> = line.split_whitespace().collect();
            if f.len() < 10 {
                continue;
            }
            let state = if proto.starts_with("tcp") {
                tcp_state(f[3])
            } else if f[3] == "01" {
                "CONNECTED"
            } else {
                "BOUND"
            };
            if state == "TIME_WAIT" {
                continue;
            }
            let (Some(local), Some(remote)) = (decode(f[1]), decode(f[2])) else { continue };
            let owner = f[9].parse::<u64>().ok().and_then(|i| owners.get(&i).cloned());
            conns.push((proto.trim_end_matches('6').to_uppercase(), local, remote, state, owner));
        }
    }
    let rank = |s: &str| match s {
        "LISTEN" | "BOUND" => 0,
        "ESTABLISHED" | "CONNECTED" => 1,
        _ => 2,
    };
    conns.sort_by(|a, b| (rank(a.3), &a.0, &a.1).cmp(&(rank(b.3), &b.0, &b.1)));
    let rows: Vec<Vec<String>> = conns
        .iter()
        .map(|(p, l, r, s, o)| {
            let remote = if r.ends_with(":0") { "-".to_string() } else { r.clone() };
            let owner = o.as_ref().map(|(pid, n)| format!("{n} ({pid})")).unwrap_or("-".into());
            vec![p.clone(), l.clone(), remote, s.to_string(), owner]
        })
        .collect();
    let mut t = Text::new();
    t.table(&["Proto", "Local", "Remote", "State", "Process"], &rows);
    if partial {
        t.line(&dim(&format!("Other users' processes are hidden; {}", util::permission_hint())));
    }
    let list: Vec<_> = conns
        .iter()
        .map(|(p, l, r, s, o)| {
            json!({ "protocol": p, "local": l, "remote": r, "state": s,
                    "pid": o.as_ref().map(|x| x.0), "process": o.as_ref().map(|x| x.1.clone()) })
        })
        .collect();
    Ok(t.finish(json!(list)))
}

fn fetch(url: &str) -> Option<String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(5)))
        .build()
        .into();
    let body = agent.get(url).call().ok()?.body_mut().read_to_string().ok()?;
    let ip = body.trim();
    ip.parse::<IpAddr>().is_ok().then(|| ip.to_string())
}

fn public() -> Result<Output> {
    let v4 = fetch("https://api.ipify.org");
    let v6 = fetch("https://api6.ipify.org");
    if v4.is_none() && v6.is_none() {
        return Err(Error::Unavailable("could not reach api.ipify.org".into()));
    }
    let mut t = Text::new();
    t.kv("IPv4", v4.clone().unwrap_or("-".into()))
        .kv("IPv6", v6.clone().unwrap_or("-".into()));
    Ok(t.finish(json!({ "ipv4": v4, "ipv6": v6 })))
}
