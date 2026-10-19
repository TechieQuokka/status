use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// Interval used for values that are derived from two readings.
pub const SAMPLE: Duration = Duration::from_millis(500);

pub enum Error {
    Usage(String),
    Unavailable(String),
    Permission,
    Failed(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Usage(m) | Error::Failed(m) => write!(f, "{m}"),
            Error::Unavailable(m) => write!(f, "not available ({m})"),
            Error::Permission => write!(f, "{}", permission_hint()),
        }
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        match e.kind() {
            io::ErrorKind::PermissionDenied => Error::Permission,
            _ => Error::Failed(e.to_string()),
        }
    }
}

pub fn usage(group: &str, msg: impl fmt::Display) -> Error {
    Error::Usage(format!("{msg}. Run 'status {group} help'."))
}

/// Shown when the binary lacks the capabilities granted by install.sh.
pub fn permission_hint() -> String {
    "requires permission (run install.sh)".into()
}

pub fn is_denied(e: &io::Error) -> bool {
    e.kind() == io::ErrorKind::PermissionDenied
}

pub fn read(path: impl AsRef<Path>) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

pub fn read_u64(path: impl AsRef<Path>) -> Option<u64> {
    read(path)?.parse().ok()
}

pub fn pids() -> Vec<u32> {
    let Ok(dir) = fs::read_dir("/proc") else {
        return Vec::new();
    };
    dir.flatten()
        .filter_map(|e| e.file_name().to_str()?.parse().ok())
        .collect()
}

pub fn comm(pid: u32) -> String {
    read(format!("/proc/{pid}/comm")).unwrap_or_else(|| "?".into())
}

pub fn hwmon_dirs() -> Vec<PathBuf> {
    let Ok(dir) = fs::read_dir("/sys/class/hwmon") else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = dir.flatten().map(|e| e.path()).collect();
    dirs.sort();
    dirs
}

/// Runs a program and returns its stdout when it exits successfully.
pub fn run(program: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(program).args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

pub fn clock_ticks() -> f64 {
    unsafe { libc::sysconf(libc::_SC_CLK_TCK) as f64 }
}

/// Encodes a device number the same way glibc's makedev does, to compare with st_dev.
pub fn makedev(major: u64, minor: u64) -> u64 {
    ((major & 0xffff_f000) << 32)
        | ((major & 0xfff) << 8)
        | ((minor & 0xffff_ff00) << 12)
        | (minor & 0xff)
}

pub fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

pub fn duration(secs: u64) -> String {
    let (d, h, m) = (secs / 86400, secs % 86400 / 3600, secs % 3600 / 60);
    match d {
        0 => format!("{h}h {m}m"),
        _ => format!("{d}d {h}h {m}m"),
    }
}

/// Formats a Unix timestamp in local time as "YYYY-MM-DD HH:MM:SS".
pub fn local_time(epoch: i64) -> String {
    unsafe {
        let t: libc::time_t = epoch as libc::time_t;
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            tm.tm_year + 1900,
            tm.tm_mon + 1,
            tm.tm_mday,
            tm.tm_hour,
            tm.tm_min,
            tm.tm_sec
        )
    }
}
