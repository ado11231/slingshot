//! Agent resource use: one snapshot, or a live view with running jobs with `--watch`.

use crate::client::{self, Control, unexpected};
use crate::live;
use slingshot_core::config::Config;
use slingshot_core::control::{Request, Response};
use slingshot_core::presentation::{Style, Tone, capacity, row};
use slingshot_core::protocol::{Health, Specs};
use slingshot_core::storage;

pub async fn health(agent: Option<String>, watch: bool) -> anyhow::Result<i32> {
    let config = Config::load()?;
    let target = config.resolve(agent.as_deref())?;
    let specs = target.specs.as_ref();
    if !watch {
        let Response::Health(health) = client::fetch(target, Request::Health).await? else {
            return Err(unexpected());
        };
        print!("{}", render(&target.name, specs, &health, Style::stdout()));
        return Ok(0);
    }
    live::require_terminal()?;
    let control = tokio::sync::Mutex::new(Control::connect(target).await?);
    let name = target.name.clone();
    live::show(|| async {
        let mut control = control.lock().await;
        let health = fetch(&mut control).await?;
        let Response::Jobs(jobs) = control.call(Request::Jobs { all: false }).await? else {
            return Err(unexpected());
        };
        let style = Style::stdout();
        let mut body = render(&name, specs, &health, style);
        body.push_str(&super::ps::render(
            &name,
            &jobs,
            false,
            storage::now(),
            super::ps::width(),
            style,
        ));
        Ok(body)
    })
    .await
}

pub async fn fetch(control: &mut Control) -> anyhow::Result<Health> {
    match control.call(Request::Health).await? {
        Response::Health(health) => Ok(health),
        _ => Err(unexpected()),
    }
}

/// The heading names the Agent, and what it is from the specs saved at linking, so the
/// hardware needs no extra request.
pub fn render(name: &str, specs: Option<&Specs>, health: &Health, style: Style) -> String {
    let mut output = match specs {
        Some(specs) => format!(
            "\n{}  {}\n\n",
            style.heading(name),
            style.dim(format!(
                "{} · {} ({} cores)",
                specs.os, specs.cpu, specs.cores
            ))
        ),
        None => format!("\n{}\n\n", style.heading(name)),
    };
    output.push_str(&row("CPU", load(health.cpu_percent as f64, style)));
    output.push_str(&row(
        "RAM",
        memory(health.memory_used_mib, health.memory_total_mib, style),
    ));
    output.push_str(&row(
        "Disk",
        format!("{} free", capacity(health.disk_free_mib)),
    ));
    if let Some(free) = health.workspace_free_mib {
        output.push_str(&row("Workspace", workspace(free, style)));
    }

    match (&health.gpu_problem, health.gpus.is_empty()) {
        (Some(problem), _) => output.push_str(&row("GPU", style.paint(problem, Tone::Warning))),
        (None, true) => output.push_str(&row("GPU", "No GPU data available")),
        (None, false) => {}
    }
    for (index, gpu) in health.gpus.iter().enumerate() {
        output.push('\n');
        output.push_str(&row(&format!("GPU {}", index + 1), &gpu.name));
        let usage = gpu
            .utilization_percent
            .map(|value| load(value as f64, style))
            .unwrap_or_else(|| "Unavailable".to_string());
        output.push_str(&row("Usage", usage));
        let vram = match (gpu.vram_free_mib, gpu.vram_total_mib) {
            (Some(free), Some(total)) if free <= total => memory(total - free, total, style),
            _ => "Unavailable".to_string(),
        };
        output.push_str(&row("VRAM", vram));
        let temperature = gpu
            .temperature_c
            .map(|value| {
                let (label, tone) = rating(value as f64, TEMPERATURE, ["Normal", "Warm", "Hot"]);
                mark(format!("{value}°C"), label, tone, style)
            })
            .unwrap_or_else(|| "Unavailable".to_string());
        output.push_str(&row("Temperature", temperature));
    }
    if health.swap_total_mib == 0 {
        output.push('\n');
        output.push_str(&style.status(
            "No swap configured. Jobs may stop if RAM runs out",
            Tone::Warning,
        ));
        output.push('\n');
    }
    output.push('\n');
    output
}

fn workspace(free_mib: u64, style: Style) -> String {
    match free_mib < slingshot_core::telemetry::DISK_WARNING_MIB {
        true => style.paint(
            format!("{} free  Low space", capacity(free_mib)),
            Tone::Error,
        ),
        false => format!("{} free", capacity(free_mib)),
    }
}

/// Color only a value that needs attention. A healthy one stays plain, with its rating
/// dimmed so the number stands out.
fn mark(value: String, label: &str, tone: Tone, style: Style) -> String {
    match tone {
        Tone::Good => format!("{value}  {}", style.dim(label)),
        tone => style.paint(format!("{value}  {label}"), tone),
    }
}

/// Where a measurement becomes worth watching, then worth acting on. The menu bar uses the
/// same limits, so a value it colors or notifies about matches what this command shows.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Limits {
    pub warning: f64,
    pub high: f64,
}

pub(crate) const LOAD: Limits = Limits {
    warning: 70.0,
    high: 90.0,
};

pub(crate) const MEMORY: Limits = Limits {
    warning: 75.0,
    high: 90.0,
};

pub(crate) const TEMPERATURE: Limits = Limits {
    warning: 75.0,
    high: 85.0,
};

impl Limits {
    pub(crate) fn tone(self, value: f64) -> Tone {
        if value >= self.high {
            Tone::Error
        } else if value >= self.warning {
            Tone::Warning
        } else {
            Tone::Good
        }
    }
}

fn rating(value: f64, limits: Limits, labels: [&str; 3]) -> (&str, Tone) {
    let tone = limits.tone(value);
    let label = match tone {
        Tone::Error => labels[2],
        Tone::Warning => labels[1],
        _ => labels[0],
    };
    (label, tone)
}

fn load(percent: f64, style: Style) -> String {
    if !percent.is_finite() || !(0.0..=100.0).contains(&percent) {
        return "Unavailable".to_string();
    }
    let (label, tone) = rating(percent, LOAD, ["Light", "Busy", "High load"]);
    mark(format!("{percent:.1}%"), label, tone, style)
}

fn memory(used: u64, total: u64, style: Style) -> String {
    if total == 0 || used > total {
        return "Unavailable".to_string();
    }
    let percent = used as f64 / total as f64 * 100.0;
    let (label, tone) = rating(percent, MEMORY, ["Available", "Limited", "Low free memory"]);
    mark(
        format!(
            "{} / {} used, {} free",
            capacity(used),
            capacity(total),
            capacity(total - used)
        ),
        label,
        tone,
        style,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use slingshot_core::protocol::GpuHealth;

    fn sample() -> Health {
        Health {
            cpu_percent: 42.0,
            memory_used_mib: 18432,
            memory_total_mib: 65536,
            swap_total_mib: 4096,
            disk_free_mib: 419840,
            workspace_free_mib: Some(1024),
            gpus: vec![GpuHealth {
                name: "Example GPU".to_string(),
                vram_free_mib: Some(14336),
                vram_total_mib: Some(24576),
                utilization_percent: Some(71),
                temperature_c: Some(76),
            }],
            gpu_problem: None,
        }
    }

    #[test]
    fn usage_thresholds_include_the_boundary() {
        for (value, status, color) in [
            (69.0, "Light", ""),
            (70.0, "Busy", "\x1b[33m"),
            (89.0, "Busy", "\x1b[33m"),
            (90.0, "High load", "\x1b[31m"),
        ] {
            let text = load(value, Style::new(true));
            assert!(text.contains(status));
            assert!(text.starts_with(&format!("{color}{value:.1}%")), "{text}");
        }
        for invalid in [f64::NAN, f64::INFINITY, -1.0, 101.0] {
            assert_eq!(load(invalid, Style::new(true)), "Unavailable");
        }
    }

    #[test]
    fn memory_thresholds_and_invalid_totals() {
        for (used, status) in [
            (74, "Available"),
            (75, "Limited"),
            (89, "Limited"),
            (90, "Low free memory"),
        ] {
            assert!(memory(used, 100, Style::new(false)).ends_with(status));
        }
        assert_eq!(memory(0, 0, Style::new(true)), "Unavailable");
        assert_eq!(memory(101, 100, Style::new(true)), "Unavailable");
        assert!(memory(1024, 1536, Style::new(false)).contains("512.0 MiB free"));
    }

    #[test]
    fn temperature_thresholds() {
        let mut health = sample();
        for (temperature, expected) in [
            (74, "  74°C  \x1b[2mNormal"),
            (75, "  \x1b[33m75°C  Warm"),
            (84, "  \x1b[33m84°C  Warm"),
            (85, "  \x1b[31m85°C  Hot"),
        ] {
            health.gpus[0].temperature_c = Some(temperature);
            let output = render("archbox", None, &health, Style::new(true));
            assert!(output.contains(expected), "{output}");
        }
    }

    #[test]
    fn missing_and_multiple_gpu_measurements() {
        let mut health = sample();
        health.gpus.push(GpuHealth {
            name: "Second GPU".to_string(),
            vram_free_mib: None,
            vram_total_mib: None,
            utilization_percent: None,
            temperature_c: None,
        });
        let output = render("archbox", None, &health, Style::new(false));
        assert!(output.contains("GPU 1"));
        assert!(output.contains("GPU 2"));
        assert_eq!(output.matches("Unavailable").count(), 3);
        health.gpus[0].vram_free_mib = Some(999999);
        assert!(
            render("archbox", None, &health, Style::new(false))
                .contains("VRAM         Unavailable")
        );
        health.gpus.clear();
        assert!(
            render("archbox", None, &health, Style::new(false)).contains("No GPU data available")
        );
        health.gpu_problem = Some("Reboot the Agent to fix".to_string());
        let output = render("archbox", None, &health, Style::new(false));
        assert!(
            output.contains("GPU          Reboot the Agent to fix"),
            "{output}"
        );
        assert!(!output.contains("No GPU data available"));
    }

    #[test]
    fn plain_snapshot_has_readable_units_and_no_escapes() {
        let output = render("archbox", None, &sample(), Style::new(false));
        assert!(output.contains("CPU          42.0%  Light"));
        assert!(output.contains("18.0 GiB / 64.0 GiB used, 46.0 GiB free  Available"));
        assert!(output.contains("410.0 GiB free"));
        assert!(output.contains("Workspace    1.0 GiB free  Low space"));
        assert!(output.contains("71.0%  Busy"));
        assert!(output.contains("76°C  Warm"));
        assert!(!output.contains('\x1b'));
        assert!(!output.contains("No swap"));
    }

    #[test]
    fn the_heading_names_the_hardware_from_saved_specs() {
        let specs = Specs {
            name: "archbox".into(),
            os: "Arch Linux".into(),
            kernel: "6.1".into(),
            cpu: "Example CPU".into(),
            cores: 16,
            memory_mib: 65536,
            disk_total_mib: 1048576,
            tools: vec![],
            gpus: vec![],
        };
        let output = render("archbox", Some(&specs), &sample(), Style::new(false));
        assert!(output.starts_with("\narchbox  Arch Linux · Example CPU (16 cores)\n"));
        assert!(render("archbox", None, &sample(), Style::new(false)).starts_with("\narchbox\n"));
    }

    #[test]
    fn swap_notice_is_a_readable_warning() {
        let mut health = sample();
        health.swap_total_mib = 0;
        assert!(
            render("archbox", None, &health, Style::new(false)).contains("! No swap configured.")
        );
    }
}
