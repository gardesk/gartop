//! Temperature sensor collection from /sys/class/hwmon

use crate::error::Result;
use gartop_ipc::{TempSensor, TempStats};
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// Temperature sensor collector.
pub struct TempCollector;

impl TempCollector {
    /// Create a new temperature collector.
    pub fn new() -> Result<Self> {
        Ok(Self)
    }

    /// Collect current temperature readings from all sensors.
    pub fn collect(&mut self) -> Result<TempStats> {
        let mut sensors = Vec::new();
        let hwmon_path = Path::new("/sys/class/hwmon");

        if hwmon_path.exists() {
            if let Ok(entries) = fs::read_dir(hwmon_path) {
                for entry in entries.flatten() {
                    let hwmon_dir = entry.path();
                    if let Some(device_sensors) = Self::read_hwmon_device(&hwmon_dir) {
                        sensors.extend(device_sensors);
                    }
                }
            }
        }

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        Ok(TempStats { sensors, timestamp })
    }

    /// Read all temperature sensors from a hwmon device directory.
    fn read_hwmon_device(hwmon_dir: &Path) -> Option<Vec<TempSensor>> {
        let device_name = Self::read_file(hwmon_dir.join("name")).unwrap_or_default();
        let mut sensors = Vec::new();

        // Find all temp*_input files
        if let Ok(entries) = fs::read_dir(hwmon_dir) {
            for entry in entries.flatten() {
                let filename = entry.file_name();
                let filename_str = filename.to_string_lossy();

                if filename_str.starts_with("temp") && filename_str.ends_with("_input") {
                    // Extract sensor index (e.g., "temp1_input" -> "1")
                    let index = filename_str
                        .strip_prefix("temp")
                        .and_then(|s| s.strip_suffix("_input"))
                        .unwrap_or("");

                    if let Some(sensor) = Self::read_sensor(hwmon_dir, index, &device_name) {
                        sensors.push(sensor);
                    }
                }
            }
        }

        if sensors.is_empty() {
            None
        } else {
            Some(sensors)
        }
    }

    /// Read a single temperature sensor.
    fn read_sensor(hwmon_dir: &Path, index: &str, device_name: &str) -> Option<TempSensor> {
        // Read temperature (millidegrees Celsius)
        let temp_path = hwmon_dir.join(format!("temp{}_input", index));
        let temp_millidegrees: i64 = Self::read_file(&temp_path)?.trim().parse().ok()?;
        let temp_celsius = temp_millidegrees as f64 / 1000.0;

        // Read label (optional)
        let label_path = hwmon_dir.join(format!("temp{}_label", index));
        let label = Self::read_file(label_path)
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| format!("temp{}", index));

        // Read critical threshold (optional, millidegrees)
        let crit_path = hwmon_dir.join(format!("temp{}_crit", index));
        let critical = Self::read_file(crit_path)
            .and_then(|s| s.trim().parse::<i64>().ok())
            .map(|m| m as f64 / 1000.0);

        // Read high threshold (optional, millidegrees)
        let high_path = hwmon_dir.join(format!("temp{}_max", index));
        let high = Self::read_file(high_path)
            .and_then(|s| s.trim().parse::<i64>().ok())
            .map(|m| m as f64 / 1000.0);

        Some(TempSensor {
            label,
            device: device_name.to_string(),
            temp_celsius,
            critical,
            high,
        })
    }

    /// Read a file to string, returning None on error.
    fn read_file<P: AsRef<Path>>(path: P) -> Option<String> {
        fs::read_to_string(path).ok()
    }
}

impl Default for TempCollector {
    fn default() -> Self {
        Self
    }
}
