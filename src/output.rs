use serde_json::Value;
use std::io::IsTerminal;
use std::sync::OnceLock;

static COLOR: OnceLock<bool> = OnceLock::new();

pub fn init_color(json: bool) {
    let enabled =
        !json && std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none();
    let _ = COLOR.set(enabled);
}

fn paint(code: &str, s: &str) -> String {
    if *COLOR.get().unwrap_or(&false) {
        format!("\x1b[{code}m{s}\x1b[0m")
    } else {
        s.to_string()
    }
}

pub fn bold(s: &str) -> String {
    paint("1", s)
}

pub fn dim(s: &str) -> String {
    paint("2", s)
}

pub fn accent(s: &str) -> String {
    paint("36", s)
}

pub fn gb(bytes: u64) -> String {
    let v = bytes as f64 / 1e9;
    match v {
        0.0 => "0 GB".into(),
        v if v < 0.01 => "<0.01 GB".into(),
        v if v < 10.0 => format!("{v:.2} GB"),
        v => format!("{v:.1} GB"),
    }
}

pub fn rate(bytes_per_sec: f64) -> String {
    format!("{:.1} MB/s", bytes_per_sec / 1e6)
}

pub fn pct(p: f64) -> String {
    format!("{p:.0}%")
}

pub fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

pub fn bar(p: f64) -> String {
    const WIDTH: usize = 20;
    let filled = ((p.clamp(0.0, 100.0) / 100.0) * WIDTH as f64).round() as usize;
    format!(
        "{}{}",
        accent(&"█".repeat(filled)),
        dim(&"░".repeat(WIDTH - filled))
    )
}

/// Visible width of a string, ignoring ANSI escape sequences.
fn width(s: &str) -> usize {
    let mut n = 0;
    let mut escape = false;
    for c in s.chars() {
        match (escape, c) {
            (false, '\x1b') => escape = true,
            (true, 'm') => escape = false,
            (true, _) => {}
            (false, _) => n += 1,
        }
    }
    n
}

fn pad(s: &str, w: usize) -> String {
    format!("{s}{}", " ".repeat(w.saturating_sub(width(s))))
}

pub struct Output {
    pub text: String,
    pub json: Value,
}

#[derive(Default)]
pub struct Text {
    buf: String,
}

impl Text {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn title(&mut self, s: &str) -> &mut Self {
        self.buf += &format!(" {}\n", bold(s));
        self
    }

    pub fn line(&mut self, s: &str) -> &mut Self {
        self.buf += &format!(" {s}\n");
        self
    }

    pub fn kv(&mut self, key: &str, value: impl AsRef<str>) -> &mut Self {
        self.buf += &format!(" {}{}\n", pad(key, 15), value.as_ref());
        self
    }

    pub fn table(&mut self, header: &[&str], rows: &[Vec<String>]) -> &mut Self {
        let cols = header.len();
        let mut widths: Vec<usize> = header.iter().map(|h| width(h)).collect();
        for row in rows {
            for (i, cell) in row.iter().enumerate().take(cols) {
                widths[i] = widths[i].max(width(cell));
            }
        }
        let render = |cells: Vec<String>| {
            let last = cells.len().saturating_sub(1);
            let line: Vec<String> = cells
                .iter()
                .enumerate()
                .map(|(i, c)| if i == last { c.clone() } else { pad(c, widths[i]) })
                .collect();
            format!(" {}\n", line.join("  ").trim_end())
        };
        self.buf += &render(header.iter().map(|h| dim(h)).collect());
        for row in rows {
            self.buf += &render(row.clone());
        }
        self
    }

    pub fn finish(self, json: Value) -> Output {
        Output {
            text: self.buf,
            json,
        }
    }
}
