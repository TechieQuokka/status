//! SMART for SATA disks via SCSI ATA PASS-THROUGH (16) over the SG_IO ioctl.

use crate::disk::Disk;
use crate::output::{Output, Text, dim, gb};
use crate::util::{self, Error, Result};
use serde_json::json;

#[repr(C)]
struct SgIoHdr {
    interface_id: i32,
    dxfer_direction: i32,
    cmd_len: u8,
    mx_sb_len: u8,
    iovec_count: u16,
    dxfer_len: u32,
    dxferp: *mut libc::c_void,
    cmdp: *mut u8,
    sbp: *mut u8,
    timeout: u32,
    flags: u32,
    pack_id: i32,
    usr_ptr: *mut libc::c_void,
    status: u8,
    masked_status: u8,
    msg_status: u8,
    sb_len_wr: u8,
    host_status: u16,
    driver_status: u16,
    resid: i32,
    duration: u32,
    info: u32,
}

const SG_IO: libc::c_ulong = 0x2285;
const SG_DXFER_NONE: i32 = -1;
const SG_DXFER_FROM_DEV: i32 = -3;

const ATA_SMART: u8 = 0xB0;
const SMART_READ_DATA: u8 = 0xD0;
const SMART_RETURN_STATUS: u8 = 0xDA;

struct Device(i32);

impl Drop for Device {
    fn drop(&mut self) {
        unsafe { libc::close(self.0) };
    }
}

impl Device {
    fn open(name: &str) -> Result<Self> {
        let path = std::ffi::CString::new(format!("/dev/{name}")).unwrap();
        let fd = unsafe { libc::open(path.as_ptr(), libc::O_RDONLY | libc::O_NONBLOCK | libc::O_CLOEXEC) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(Self(fd))
    }

    /// Issues a SMART command. With `data`, reads one 512-byte sector; returns the sense buffer.
    fn smart(&self, feature: u8, data: Option<&mut [u8; 512]>) -> Result<[u8; 32]> {
        let mut cdb = [0u8; 16];
        cdb[0] = 0x85;
        if data.is_some() {
            // PIO data-in, read direction, length in sector count.
            cdb[1] = 4 << 1;
            cdb[2] = 0x0E;
        } else {
            // Non-data, with CK_COND so the device registers come back in sense.
            cdb[1] = 3 << 1;
            cdb[2] = 0x20;
        }
        cdb[4] = feature;
        cdb[6] = 1;
        cdb[10] = 0x4F;
        cdb[12] = 0xC2;
        cdb[14] = ATA_SMART;

        let mut sense = [0u8; 32];
        let (dir, len, ptr) = match data {
            Some(buf) => (SG_DXFER_FROM_DEV, 512, buf.as_mut_ptr() as *mut libc::c_void),
            None => (SG_DXFER_NONE, 0, std::ptr::null_mut()),
        };
        let mut hdr: SgIoHdr = unsafe { std::mem::zeroed() };
        hdr.interface_id = b'S' as i32;
        hdr.dxfer_direction = dir;
        hdr.cmd_len = 16;
        hdr.mx_sb_len = sense.len() as u8;
        hdr.dxfer_len = len;
        hdr.dxferp = ptr;
        hdr.cmdp = cdb.as_mut_ptr();
        hdr.sbp = sense.as_mut_ptr();
        hdr.timeout = 5000;
        if unsafe { libc::ioctl(self.0, SG_IO as _, &mut hdr as *mut SgIoHdr) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        // DRIVER_SENSE (0x08) is expected: CK_COND returns the registers as sense data.
        if hdr.host_status != 0 || !matches!(hdr.driver_status & 0x0F, 0 | 1 | 8) {
            return Err(Error::Unavailable("device rejected ATA pass-through".into()));
        }
        Ok(sense)
    }
}

/// Reads the ATA LBA mid/high registers returned in sense data (descriptor or fixed format).
fn lba_registers(sense: &[u8; 32]) -> Option<(u8, u8)> {
    match sense[0] & 0x7F {
        0x72 => {
            let mut i = 8;
            while i + 1 < sense.len() {
                let (code, len) = (sense[i], sense[i + 1] as usize);
                if code == 0x09 && i + 11 < sense.len() {
                    return Some((sense[i + 9], sense[i + 11]));
                }
                if len == 0 {
                    break;
                }
                i += 2 + len;
            }
            None
        }
        0x70 => Some((sense[10], sense[9])),
        _ => None,
    }
}

struct Attribute {
    id: u8,
    value: u8,
    raw: u64,
}

fn attributes(data: &[u8; 512]) -> Vec<Attribute> {
    data[2..362]
        .chunks(12)
        .filter(|c| c[0] != 0)
        .map(|c| Attribute {
            id: c[0],
            value: c[3],
            raw: c[5..11].iter().rev().fold(0u64, |a, &b| (a << 8) | b as u64),
        })
        .collect()
}

pub fn report(disk: &Disk, header: &str) -> Result<Output> {
    if !disk.name.starts_with("sd") {
        return Err(Error::Unavailable("SMART is implemented for SATA disks only".into()));
    }
    let dev = Device::open(&disk.name)?;
    let mut data = [0u8; 512];
    dev.smart(SMART_READ_DATA, Some(&mut data))?;
    if data.iter().all(|&b| b == 0) {
        return Err(Error::Unavailable("SMART data is empty (SMART disabled or unsupported bridge)".into()));
    }
    let health = match dev.smart(SMART_RETURN_STATUS, None).ok().as_ref().and_then(lba_registers) {
        Some((0x4F, 0xC2)) => "PASSED",
        Some((0xF4, 0x2C)) => "FAILING",
        _ => "unknown",
    };
    let attrs = attributes(&data);
    let raw = |id: u8| attrs.iter().find(|a| a.id == id).map(|a| a.raw);
    let value = |id: u8| attrs.iter().find(|a| a.id == id).map(|a| a.value);

    let temp = raw(194).or(raw(190)).map(|r| r & 0xFF);
    let hours = raw(9).map(|r| r & 0xFFFF_FFFF);
    let cycles = raw(12);
    let (realloc, pending, uncorrect) = (raw(5), raw(197), raw(198));
    // Normalized wear indicators count down from 100 (Samsung 177, Intel 233, others 231).
    let life = if disk.kind == "HDD" { None } else { value(177).or(value(231)).or(value(233)) };
    let written = raw(241).map(|r| r * 512);

    let show = |v: Option<u64>| v.map(util::thousands).unwrap_or_else(|| "-".into());
    let mut t = Text::new();
    t.title(header)
        .kv("Health", health)
        .kv("Temperature", temp.map(|c| format!("{c}°C")).unwrap_or("-".into()))
        .kv("Power-on", hours.map(|h| format!("{} h   {}", util::thousands(h), dim(&format!("{} days", h / 24)))).unwrap_or("-".into()))
        .kv("Power cycles", show(cycles))
        .kv("Reallocated", show(realloc))
        .kv("Pending", show(pending))
        .kv("Uncorrectable", show(uncorrect));
    if let Some(l) = life {
        t.kv("Life left", format!("{l}%"));
    }
    if let Some(w) = written {
        t.kv("Data written", gb(w));
    }
    Ok(t.finish(json!({
        "health": health, "temperature_celsius": temp, "power_on_hours": hours,
        "power_cycles": cycles, "reallocated_sectors": realloc, "pending_sectors": pending,
        "uncorrectable_sectors": uncorrect, "life_left_percent": life, "data_written": written,
    })))
}
