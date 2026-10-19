mod cpu;
mod disk;
mod gpu;
mod help;
mod memory;
mod network;
mod output;
mod smart;
mod summary;
mod system;
mod util;

use output::Output;
use util::{Error, Result};

const GROUPS: [&str; 6] = ["disk", "cpu", "memory", "network", "gpu", "system"];

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    let mut json = false;
    let mut scan = false;
    let mut words = Vec::new();
    for arg in &args {
        match arg.as_str() {
            "--json" => json = true,
            "--scan" => scan = true,
            "-h" | "--help" => words.push("help".to_string()),
            "-V" | "--version" => {
                println!("status {}", env!("CARGO_PKG_VERSION"));
                return;
            }
            other => words.push(other.to_string()),
        }
    }
    output::init_color(json);

    match dispatch(&words, scan) {
        Ok(out) if json => println!(
            "{}",
            serde_json::to_string_pretty(&out.json).unwrap_or_default()
        ),
        Ok(out) => print!("{}", out.text),
        Err(e) => {
            if json {
                println!("{}", serde_json::json!({ "error": e.to_string() }));
            } else {
                eprintln!("status: {e}");
            }
            std::process::exit(match e {
                Error::Usage(_) => 2,
                _ => 1,
            });
        }
    }
}

fn dispatch(words: &[String], scan: bool) -> Result<Output> {
    let Some((group, rest)) = words.split_first() else {
        return reject_scan(scan, summary::run());
    };
    if group == "help" {
        return Ok(help::overview());
    }
    if !GROUPS.contains(&group.as_str()) {
        return Err(Error::Usage(format!(
            "unknown command '{group}'. Run 'status help'."
        )));
    }
    if rest.iter().any(|w| w == "help") {
        return Ok(help::group(group));
    }
    if scan && group != "disk" {
        return Err(Error::Usage(
            "--scan is only valid with 'status disk <n>'".into(),
        ));
    }
    match group.as_str() {
        "disk" => disk::run(rest, scan),
        "cpu" => cpu::run(rest),
        "memory" => memory::run(rest),
        "network" => network::run(rest),
        "gpu" => gpu::run(rest),
        _ => system::run(rest),
    }
}

fn reject_scan(scan: bool, result: Result<Output>) -> Result<Output> {
    if scan {
        return Err(Error::Usage(
            "--scan is only valid with 'status disk <n>'".into(),
        ));
    }
    result
}
