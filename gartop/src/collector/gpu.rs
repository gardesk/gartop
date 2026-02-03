//! GPU information collection from sysfs/drm

use crate::error::Result;
use gartop_ipc::{GpuDevice, GpuStats};
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// GPU information collector.
pub struct GpuCollector;

impl GpuCollector {
    /// Create a new GPU collector.
    pub fn new() -> Result<Self> {
        Ok(Self)
    }

    /// Collect current GPU stats from all devices.
    pub fn collect(&mut self) -> Result<GpuStats> {
        let mut devices = Vec::new();

        // Check for DRM devices (AMD, Intel)
        let drm_path = Path::new("/sys/class/drm");
        if drm_path.exists() {
            if let Ok(entries) = fs::read_dir(drm_path) {
                for entry in entries.flatten() {
                    let name = entry.file_name();
                    let name_str = name.to_string_lossy();

                    // Only process card* entries, not renderD*
                    if name_str.starts_with("card") && !name_str.contains('-') {
                        if let Some(device) = Self::read_drm_device(&entry.path(), &name_str) {
                            devices.push(device);
                        }
                    }
                }
            }
        }

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        Ok(GpuStats { devices, timestamp })
    }

    /// Read GPU info from a DRM device directory.
    fn read_drm_device(card_path: &Path, card_name: &str) -> Option<GpuDevice> {
        let device_path = card_path.join("device");
        if !device_path.exists() {
            return None;
        }

        // Read GPU model from various sources
        let model = Self::read_gpu_model(&device_path);

        // GPU utilization (AMD: gpu_busy_percent)
        let usage_percent = Self::read_file_f64(device_path.join("gpu_busy_percent"))
            .unwrap_or(0.0);

        // VRAM stats (AMD)
        let vram_used = Self::read_file_u64(device_path.join("mem_info_vram_used"))
            .unwrap_or(0);
        let vram_total = Self::read_file_u64(device_path.join("mem_info_vram_total"))
            .unwrap_or(0);

        // GPU clock from hwmon (varies by vendor)
        let clock_mhz = Self::find_gpu_clock(&device_path);

        // GPU temperature from hwmon
        let temp_celsius = Self::find_gpu_temp(&device_path);

        // Power usage from hwmon (in microwatts, convert to watts)
        let power_watts = Self::find_gpu_power(&device_path);

        // Skip if we couldn't get any meaningful data
        if usage_percent == 0.0 && vram_total == 0 && clock_mhz.is_none() && temp_celsius.is_none() {
            return None;
        }

        Some(GpuDevice {
            name: card_name.to_string(),
            model,
            usage_percent,
            vram_used,
            vram_total,
            clock_mhz,
            temp_celsius,
            power_watts,
        })
    }

    /// Read GPU model name from device info.
    fn read_gpu_model(device_path: &Path) -> String {
        // Try uevent for DRIVER info
        if let Some(uevent) = Self::read_file(device_path.join("uevent")) {
            for line in uevent.lines() {
                if let Some(driver) = line.strip_prefix("DRIVER=") {
                    return driver.to_string();
                }
            }
        }

        // Try product name from DRM
        if let Some(name) = Self::read_file(device_path.join("product_name")) {
            return name.trim().to_string();
        }

        // Try vendor/device for PCI
        let vendor = Self::read_file(device_path.join("vendor"))
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        let device = Self::read_file(device_path.join("device"))
            .map(|s| s.trim().to_string())
            .unwrap_or_default();

        if !vendor.is_empty() && !device.is_empty() {
            return format!("{} {}", vendor, device);
        }

        "Unknown GPU".to_string()
    }

    /// Find GPU clock frequency from hwmon.
    fn find_gpu_clock(device_path: &Path) -> Option<u32> {
        let hwmon_path = device_path.join("hwmon");
        if !hwmon_path.exists() {
            return None;
        }

        if let Ok(entries) = fs::read_dir(&hwmon_path) {
            for entry in entries.flatten() {
                let hwmon_dir = entry.path();

                // Try freq1_input (GPU clock in Hz)
                if let Some(hz) = Self::read_file_u64(hwmon_dir.join("freq1_input")) {
                    return Some((hz / 1_000_000) as u32); // Hz to MHz
                }

                // Try pp_dpm_sclk for AMD (current clock with * marker)
                if let Some(sclk) = Self::read_file(device_path.join("pp_dpm_sclk")) {
                    for line in sclk.lines() {
                        if line.contains('*') {
                            // Format: "0: 500Mhz *" or similar
                            if let Some(mhz_str) = line.split_whitespace().nth(1) {
                                if let Some(mhz) = mhz_str.strip_suffix("Mhz")
                                    .or_else(|| mhz_str.strip_suffix("MHz"))
                                {
                                    if let Ok(val) = mhz.parse::<u32>() {
                                        return Some(val);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        None
    }

    /// Find GPU temperature from hwmon.
    fn find_gpu_temp(device_path: &Path) -> Option<f64> {
        let hwmon_path = device_path.join("hwmon");
        if !hwmon_path.exists() {
            return None;
        }

        if let Ok(entries) = fs::read_dir(&hwmon_path) {
            for entry in entries.flatten() {
                let hwmon_dir = entry.path();

                // Look for temp1_input (GPU edge temp)
                if let Some(millidegrees) = Self::read_file_i64(hwmon_dir.join("temp1_input")) {
                    return Some(millidegrees as f64 / 1000.0);
                }
            }
        }

        None
    }

    /// Find GPU power usage from hwmon.
    fn find_gpu_power(device_path: &Path) -> Option<f64> {
        let hwmon_path = device_path.join("hwmon");
        if !hwmon_path.exists() {
            return None;
        }

        if let Ok(entries) = fs::read_dir(&hwmon_path) {
            for entry in entries.flatten() {
                let hwmon_dir = entry.path();

                // power1_average (microwatts)
                if let Some(microwatts) = Self::read_file_u64(hwmon_dir.join("power1_average")) {
                    return Some(microwatts as f64 / 1_000_000.0);
                }
            }
        }

        None
    }

    /// Read a file to string.
    fn read_file<P: AsRef<Path>>(path: P) -> Option<String> {
        fs::read_to_string(path).ok()
    }

    /// Read a file as u64.
    fn read_file_u64<P: AsRef<Path>>(path: P) -> Option<u64> {
        Self::read_file(path)?.trim().parse().ok()
    }

    /// Read a file as i64.
    fn read_file_i64<P: AsRef<Path>>(path: P) -> Option<i64> {
        Self::read_file(path)?.trim().parse().ok()
    }

    /// Read a file as f64.
    fn read_file_f64<P: AsRef<Path>>(path: P) -> Option<f64> {
        Self::read_file(path)?.trim().parse().ok()
    }
}

impl Default for GpuCollector {
    fn default() -> Self {
        Self
    }
}
