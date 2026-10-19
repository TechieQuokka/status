use crate::output::{Output, Text, bar, dim, gb, pct, round1};
use crate::util::{self, Error, Result, usage};
use nvml_wrapper::Nvml;
use nvml_wrapper::enum_wrappers::device::{Clock, TemperatureSensor, TemperatureThreshold};
use nvml_wrapper::enums::device::UsedGpuMemory;
use serde_json::json;

pub fn run(words: &[String]) -> Result<Output> {
    let nvml = init()?;
    if words.first().is_some_and(|w| w == "list") {
        if words.len() > 1 {
            return Err(usage("gpu", "'list' takes no arguments"));
        }
        return list(&nvml);
    }
    let (index, rest) = match words.first().and_then(|w| w.parse::<u32>().ok()) {
        Some(n) => (n, &words[1..]),
        None => (1, words),
    };
    let count = nvml.device_count().map_err(nvml_error)?;
    if index == 0 || index > count {
        return Err(Error::Usage(format!("GPU {index} does not exist. Run 'status gpu list'.")));
    }
    let gpu = Gpu { nvml: &nvml, index };
    match rest {
        [] => gpu.usage(),
        [w] => match w.as_str() {
            "memory" => gpu.memory(),
            "temperature" => gpu.temperature(),
            "fan" => gpu.fan(),
            "power" => gpu.power(),
            "info" => gpu.info(),
            "process" => gpu.process(),
            other => Err(usage("gpu", format!("unknown subcommand '{other}'"))),
        },
        _ => Err(usage("gpu", "too many arguments")),
    }
}

pub fn init() -> Result<Nvml> {
    Nvml::init().map_err(|_| Error::Unavailable("NVIDIA driver (NVML) not found".into()))
}

fn nvml_error(e: nvml_wrapper::error::NvmlError) -> Error {
    match e {
        nvml_wrapper::error::NvmlError::NoPermission => Error::Permission,
        e => Error::Failed(format!("NVML: {e}")),
    }
}

fn list(nvml: &Nvml) -> Result<Output> {
    let driver = nvml.sys_driver_version().unwrap_or_default();
    let count = nvml.device_count().map_err(nvml_error)?;
    let mut rows = Vec::new();
    let mut list = Vec::new();
    for i in 0..count {
        let name = nvml.device_by_index(i).and_then(|d| d.name()).unwrap_or_default();
        rows.push(vec![(i + 1).to_string(), name.clone(), driver.clone()]);
        list.push(json!({ "number": i + 1, "name": name, "driver": driver }));
    }
    let mut t = Text::new();
    t.table(&["#", "Model", "Driver"], &rows);
    Ok(t.finish(json!(list)))
}

/// Usage, temperature and name of a GPU for the summary.
pub fn brief(nvml: &Nvml) -> Option<(String, u32, u32)> {
    let d = nvml.device_by_index(0).ok()?;
    Some((d.name().ok()?, d.utilization_rates().ok()?.gpu, d.temperature(TemperatureSensor::Gpu).ok()?))
}

struct Gpu<'a> {
    nvml: &'a Nvml,
    index: u32,
}

impl Gpu<'_> {
    fn device(&self) -> Result<nvml_wrapper::Device<'_>> {
        self.nvml.device_by_index(self.index - 1).map_err(nvml_error)
    }

    fn header(&self, t: &mut Text) -> Result<()> {
        let name = self.device()?.name().map_err(nvml_error)?;
        t.title(&format!("[{}] {name}", self.index));
        Ok(())
    }

    fn usage(&self) -> Result<Output> {
        let d = self.device()?;
        let u = d.utilization_rates().map_err(nvml_error)?;
        let enc = d.encoder_utilization().map(|e| e.utilization).ok();
        let dec = d.decoder_utilization().map(|e| e.utilization).ok();
        let mut t = Text::new();
        self.header(&mut t)?;
        let line = |v: u32| format!("{}   {}", bar(v as f64), pct(v as f64));
        t.kv("GPU", line(u.gpu)).kv("Memory bus", line(u.memory));
        if let Some(e) = enc {
            t.kv("Encoder", line(e));
        }
        if let Some(e) = dec {
            t.kv("Decoder", line(e));
        }
        Ok(t.finish(json!({
            "gpu_percent": u.gpu, "memory_percent": u.memory, "encoder_percent": enc, "decoder_percent": dec,
        })))
    }

    fn memory(&self) -> Result<Output> {
        let m = self.device()?.memory_info().map_err(nvml_error)?;
        let p = if m.total == 0 { 0.0 } else { m.used as f64 * 100.0 / m.total as f64 };
        let mut t = Text::new();
        self.header(&mut t)?;
        t.kv("Total", gb(m.total))
            .kv("Used", format!("{}   {} {}", gb(m.used), bar(p), pct(p)))
            .kv("Free", gb(m.free));
        Ok(t.finish(json!({ "total": m.total, "used": m.used, "free": m.free, "used_percent": round1(p) })))
    }

    fn temperature(&self) -> Result<Output> {
        let d = self.device()?;
        let temp = d.temperature(TemperatureSensor::Gpu).map_err(nvml_error)?;
        let slowdown = d.temperature_threshold(TemperatureThreshold::Slowdown).ok();
        let shutdown = d.temperature_threshold(TemperatureThreshold::Shutdown).ok();
        let c = |v: Option<u32>| v.map(|v| format!("{v}°C")).unwrap_or("-".into());
        let mut t = Text::new();
        self.header(&mut t)?;
        t.kv("Temperature", format!("{temp}°C"))
            .kv("Slowdown", c(slowdown))
            .kv("Shutdown", c(shutdown));
        Ok(t.finish(json!({ "celsius": temp, "slowdown_celsius": slowdown, "shutdown_celsius": shutdown })))
    }

    fn fan(&self) -> Result<Output> {
        let d = self.device()?;
        let n = d.num_fans().map_err(nvml_error)?;
        if n == 0 {
            return Err(Error::Unavailable("GPU reports no fans".into()));
        }
        let mut t = Text::new();
        self.header(&mut t)?;
        let mut list = Vec::new();
        for i in 0..n {
            let s = d.fan_speed(i).map_err(nvml_error)?;
            t.kv(&format!("Fan {}", i + 1), format!("{}   {}", bar(s as f64), pct(s as f64)));
            list.push(json!({ "fan": i + 1, "percent": s }));
        }
        Ok(t.finish(json!(list)))
    }

    fn power(&self) -> Result<Output> {
        let d = self.device()?;
        let draw = d.power_usage().map_err(nvml_error)? as f64 / 1000.0;
        let limit = d.enforced_power_limit().ok().map(|l| l as f64 / 1000.0);
        let mut t = Text::new();
        self.header(&mut t)?;
        t.kv("Power", format!("{draw:.1} W"));
        if let Some(l) = limit {
            let p = draw / l * 100.0;
            t.kv("Limit", format!("{l:.0} W   {} {}", bar(p), pct(p)));
        }
        Ok(t.finish(json!({ "watts": round1(draw), "limit_watts": limit })))
    }

    fn info(&self) -> Result<Output> {
        let d = self.device()?;
        let name = d.name().map_err(nvml_error)?;
        let driver = self.nvml.sys_driver_version().unwrap_or_default();
        let cuda = self.nvml.sys_cuda_driver_version().ok().map(|v| {
            format!(
                "{}.{}",
                nvml_wrapper::cuda_driver_version_major(v),
                nvml_wrapper::cuda_driver_version_minor(v)
            )
        });
        let vbios = d.vbios_version().unwrap_or_default();
        let link = |gen_: Option<u32>, width: Option<u32>| match (gen_, width) {
            (Some(g), Some(w)) => format!("Gen{g} x{w}"),
            _ => "-".into(),
        };
        let pcie_now = link(d.current_pcie_link_gen().ok(), d.current_pcie_link_width().ok());
        let pcie_max = link(d.max_pcie_link_gen().ok(), d.max_pcie_link_width().ok());
        let clock = |c: Clock| (d.clock_info(c).ok(), d.max_clock_info(c).ok());
        let (gfx, gfx_max) = clock(Clock::Graphics);
        let (mem, mem_max) = clock(Clock::Memory);
        let mhz = |now: Option<u32>, max: Option<u32>| match (now, max) {
            (Some(n), Some(m)) => format!("{n} MHz   {}", dim(&format!("max {m} MHz"))),
            (Some(n), None) => format!("{n} MHz"),
            _ => "-".into(),
        };
        let mut t = Text::new();
        self.header(&mut t)?;
        t.kv("Model", &name)
            .kv("Driver", &driver)
            .kv("CUDA", cuda.clone().unwrap_or("-".into()))
            .kv("VBIOS", &vbios)
            .kv("PCIe", format!("{pcie_now}   {}", dim(&format!("max {pcie_max}"))))
            .kv("Core clock", mhz(gfx, gfx_max))
            .kv("Memory clock", mhz(mem, mem_max));
        Ok(t.finish(json!({
            "number": self.index, "model": name, "driver": driver, "cuda": cuda, "vbios": vbios,
            "pcie_current": pcie_now, "pcie_max": pcie_max,
            "core_mhz": gfx, "core_max_mhz": gfx_max, "memory_mhz": mem, "memory_max_mhz": mem_max,
        })))
    }

    fn process(&self) -> Result<Output> {
        let d = self.device()?;
        let mut procs: Vec<(u32, &str, Option<u64>)> = Vec::new();
        let mut add = |list: Vec<nvml_wrapper::struct_wrappers::device::ProcessInfo>, kind: &'static str| {
            for p in list {
                if procs.iter().any(|x| x.0 == p.pid) {
                    continue;
                }
                let mem = match p.used_gpu_memory {
                    UsedGpuMemory::Used(b) => Some(b),
                    UsedGpuMemory::Unavailable => None,
                };
                procs.push((p.pid, kind, mem));
            }
        };
        add(d.running_compute_processes().unwrap_or_default(), "compute");
        add(d.running_graphics_processes().unwrap_or_default(), "graphics");
        procs.sort_by_key(|p| std::cmp::Reverse(p.2));
        let mut t = Text::new();
        self.header(&mut t)?;
        if procs.is_empty() {
            t.line("No processes using the GPU");
        } else {
            let rows: Vec<Vec<String>> = procs
                .iter()
                .map(|(pid, kind, mem)| {
                    vec![pid.to_string(), util::comm(*pid), kind.to_string(), mem.map(gb).unwrap_or("-".into())]
                })
                .collect();
            t.table(&["PID", "Name", "Type", "VRAM"], &rows);
        }
        let list: Vec<_> = procs
            .iter()
            .map(|(pid, kind, mem)| json!({ "pid": pid, "name": util::comm(*pid), "type": kind, "vram": mem }))
            .collect();
        Ok(t.finish(json!(list)))
    }
}
