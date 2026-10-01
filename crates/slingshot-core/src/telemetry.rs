//! Agent hardware and resource measurements. GPU data is optional.

use crate::presentation::capacity;
use crate::protocol::{Gpu, GpuHealth, Health, Specs};
use std::path::Path;
use std::process::Command;
use sysinfo::{Disks, MINIMUM_CPU_UPDATE_INTERVAL, System};

/// Programs worth knowing about when deciding what the Agent can do for you.
const INTERESTING_TOOLS: &[&str] = &[
    "docker",
    "podman",
    "rsync",
    "git",
    "tmux",
    "ollama",
    "nvidia-smi",
];

const BYTES_PER_MIB: u64 = 1024 * 1024;

/// Static facts about the Agent. `name` is passed in rather than read from the machine,
/// because it is chosen at pairing and should not drift if the hostname changes.
pub fn specs(name: &str) -> Specs {
    let mut sys = System::new_all();
    sys.refresh_all();

    let cpu = sys
        .cpus()
        .first()
        .map(|c| c.brand().trim().to_string())
        .filter(|b| !b.is_empty())
        .unwrap_or_else(|| "unknown cpu".to_string());

    Specs {
        name: name.to_string(),
        os: System::long_os_version().unwrap_or_else(|| "unknown".to_string()),
        kernel: System::kernel_version().unwrap_or_else(|| "unknown".to_string()),
        cpu,
        cores: sys.cpus().len(),
        memory_mib: sys.total_memory() / BYTES_PER_MIB,
        disk_total_mib: root_disk().map(|(total, _)| total).unwrap_or(0) / BYTES_PER_MIB,
        gpus: gpu_specs(),
        tools: INTERESTING_TOOLS
            .iter()
            .filter(|tool| is_installed(tool))
            .map(|tool| tool.to_string())
            .collect(),
    }
}

/// RAM use at or above this percentage produces a warning before new work starts.
const MEMORY_WARNING_PERCENT: u64 = 90;

/// Less free workspace space than this produces a warning before new work starts.
pub const DISK_WARNING_MIB: u64 = 2 * 1024;

/// A live snapshot. CPU usage needs two samples with a gap between them, because a
/// percentage is a change over time and a single reading has nothing to compare to.
/// `workspace` is where Slingshot keeps project copies, measured separately from `/`.
pub fn health(workspace: Option<&Path>) -> Health {
    let mut sys = System::new_all();
    sys.refresh_cpu_usage();
    std::thread::sleep(MINIMUM_CPU_UPDATE_INTERVAL);
    sys.refresh_cpu_usage();
    sys.refresh_memory();
    let (gpus, gpu_problem) = gpu_health();

    Health {
        cpu_percent: sys.global_cpu_usage(),
        memory_used_mib: sys.used_memory() / BYTES_PER_MIB,
        memory_total_mib: sys.total_memory() / BYTES_PER_MIB,
        swap_total_mib: sys.total_swap() / BYTES_PER_MIB,
        disk_free_mib: root_disk().map(|(_, free)| free).unwrap_or(0) / BYTES_PER_MIB,
        workspace_free_mib: workspace
            .and_then(free_bytes)
            .map(|free| free / BYTES_PER_MIB),
        gpus,
        gpu_problem,
    }
}

/// Resource warnings shown before a run or a new session. Measurement failures give no
/// warning, because a missing reading must never block valid work.
pub fn warnings(workspace: &Path) -> Vec<String> {
    let mut sys = System::new();
    sys.refresh_memory();
    resource_warnings(
        sys.used_memory() / BYTES_PER_MIB,
        sys.total_memory() / BYTES_PER_MIB,
        free_bytes(workspace).map(|free| free / BYTES_PER_MIB),
    )
}

fn resource_warnings(
    used_mib: u64,
    total_mib: u64,
    workspace_free_mib: Option<u64>,
) -> Vec<String> {
    let mut warnings = Vec::new();
    if total_mib > 0
        && used_mib <= total_mib
        && used_mib * 100 >= total_mib * MEMORY_WARNING_PERCENT
    {
        warnings.push(format!(
            "RAM is {}% used ({} free). The job may run slowly or be stopped",
            used_mib * 100 / total_mib,
            capacity(total_mib - used_mib)
        ));
    }
    if let Some(free) = workspace_free_mib
        && free < DISK_WARNING_MIB
    {
        warnings.push(format!(
            "Only {} free on the Agent workspace disk. Builds may fail",
            capacity(free)
        ));
    }
    warnings
}

/// Available bytes on the file system holding `path`, chosen by the longest mount point.
fn free_bytes(path: &Path) -> Option<u64> {
    let path = path.canonicalize().ok()?;
    let disks = Disks::new_with_refreshed_list();
    disks
        .list()
        .iter()
        .filter(|disk| path.starts_with(disk.mount_point()))
        .max_by_key(|disk| disk.mount_point().as_os_str().len())
        .map(|disk| disk.available_space())
}

/// Total and available bytes for the filesystem holding the root of the tree.
/// That is the one that fills up and kills a build, so it is the one worth showing.
fn root_disk() -> Option<(u64, u64)> {
    let disks = Disks::new_with_refreshed_list();

    let root = disks
        .list()
        .iter()
        .find(|d| d.mount_point() == std::path::Path::new("/"))
        .or_else(|| disks.list().iter().max_by_key(|d| d.total_space()))?;

    Some((root.total_space(), root.available_space()))
}

/// True when a program can be found on PATH.
pub fn is_installed(program: &str) -> bool {
    Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {program}"))
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}

/// Ask nvidia-smi for comma separated values. No Nvidia GPU gives an empty list, which
/// is the ordinary case rather than a failure. `Err` explains an nvidia-smi that is
/// installed but cannot reach the driver.
fn nvidia_smi(fields: &str) -> Result<Vec<Vec<String>>, String> {
    let Ok(out) = Command::new("nvidia-smi")
        .arg(format!("--query-gpu={fields}"))
        .arg("--format=csv,noheader,nounits")
        .output()
    else {
        return Ok(Vec::new());
    };

    if !out.status.success() {
        let mut text = String::from_utf8_lossy(&out.stdout).to_string();
        text.push_str(&String::from_utf8_lossy(&out.stderr));
        return match gpu_problem(&text) {
            Some(problem) => Err(problem),
            None => Ok(Vec::new()),
        };
    }

    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.split(',').map(|f| f.trim().to_string()).collect())
        .collect())
}

/// A readable reason from a failed nvidia-smi. A version mismatch means a driver update
/// is waiting for a reboot, which is common on rolling distributions.
fn gpu_problem(output: &str) -> Option<String> {
    if output.contains("No devices were found") {
        return None;
    }
    if output.contains("version mismatch") {
        return Some(
            "NVIDIA driver and library versions differ, usually after a driver update. Reboot the Agent to fix".to_string(),
        );
    }
    output
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| format!("nvidia-smi failed: {line}"))
}

fn gpu_specs() -> Vec<Gpu> {
    nvidia_smi("name,memory.total")
        .unwrap_or_default()
        .into_iter()
        .map(|row| Gpu {
            name: row.first().cloned().unwrap_or_else(|| "gpu".to_string()),
            vram_mib: row.get(1).and_then(|v| v.parse().ok()),
        })
        .collect()
}

fn gpu_health() -> (Vec<GpuHealth>, Option<String>) {
    let rows = match nvidia_smi("name,memory.free,memory.total,utilization.gpu,temperature.gpu") {
        Ok(rows) => rows,
        Err(problem) => return (Vec::new(), Some(problem)),
    };
    let gpus = rows
        .into_iter()
        .map(|row| GpuHealth {
            name: row.first().cloned().unwrap_or_else(|| "gpu".to_string()),
            vram_free_mib: row.get(1).and_then(|v| v.parse().ok()),
            vram_total_mib: row.get(2).and_then(|v| v.parse().ok()),
            utilization_percent: row.get(3).and_then(|v| v.parse().ok()),
            temperature_c: row.get(4).and_then(|v| v.parse().ok()),
        })
        .collect();
    (gpus, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warnings_start_at_ninety_percent_ram_and_two_gib_disk() {
        assert!(resource_warnings(89, 100, Some(4096)).is_empty());
        let memory = resource_warnings(90, 100, Some(4096));
        assert_eq!(memory.len(), 1);
        assert!(memory[0].contains("RAM is 90% used"), "{memory:?}");
        assert!(resource_warnings(10, 100, Some(2048)).is_empty());
        let disk = resource_warnings(10, 100, Some(2047));
        assert!(disk[0].contains("workspace disk"), "{disk:?}");
        assert!(resource_warnings(0, 0, None).is_empty());
        assert!(resource_warnings(200, 100, None).is_empty());
    }

    #[test]
    fn a_driver_mismatch_is_named_and_other_failures_keep_their_reason() {
        let mismatch = gpu_problem(
            "Failed to initialize NVML: Driver/library version mismatch\nNVML library version: 615.71\n",
        )
        .unwrap();
        assert!(mismatch.contains("Reboot the Agent"), "{mismatch}");
        assert_eq!(
            gpu_problem(
                "\nNVIDIA-SMI has failed because it couldn't communicate with the NVIDIA driver.\n"
            )
            .as_deref(),
            Some(
                "nvidia-smi failed: NVIDIA-SMI has failed because it couldn't communicate with the NVIDIA driver."
            )
        );
        assert!(gpu_problem("No devices were found\n").is_none());
        assert!(gpu_problem("").is_none());
    }

    #[test]
    fn free_bytes_is_measured_for_an_existing_path() {
        assert!(free_bytes(&std::env::temp_dir()).is_some());
        assert!(free_bytes(Path::new("/definitely/not/here")).is_none());
    }
}
