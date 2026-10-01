//! The lines `slingshot internal-watch` prints for the menu bar app. Each line is one JSON
//! object. The app only draws and posts what arrives here, so every threshold and every
//! decision to notify stays in Rust.

use crate::commands::health::{LOAD, Limits, MEMORY, TEMPERATURE};
use serde::Serialize;
use slingshot_core::presentation::Tone;
use slingshot_core::protocol::Health;
use slingshot_core::telemetry::DISK_WARNING_MIB;

/// Raised whenever a field changes meaning or disappears, so the app can ask for an update
/// rather than draw something wrong.
pub const VERSION: u32 = 1;

#[derive(Debug, Serialize)]
pub struct Line<'a> {
    pub version: u32,
    #[serde(flatten)]
    pub event: &'a Event,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    Status(Status),
    Notice(Notice),
}

/// Everything the popover shows. Measurements are absent while the Agent is offline.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Status {
    pub agent: String,
    pub online: bool,
    /// The path in use, as in "via tailnet".
    pub path: Option<String>,
    /// The full error while offline, kept for anyone who wants the exact words.
    pub error: Option<String>,
    /// The same failure in plain words, with the command that fixes it when there is one.
    pub problem: Option<Problem>,
    pub cpu: Option<Percent>,
    pub memory: Option<Usage>,
    pub workspace: Option<Space>,
    pub gpus: Vec<Gpu>,
    pub gpu_problem: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Problem {
    pub title: String,
    pub detail: String,
    pub fix: Option<Fix>,
}

/// A command to run, and which machine to run it on.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Fix {
    pub machine: Machine,
    pub command: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Machine {
    Agent,
    Client,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Percent {
    pub percent: f32,
    pub level: Level,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Usage {
    pub used_mib: u64,
    pub total_mib: u64,
    pub level: Level,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Space {
    pub free_mib: u64,
    pub level: Level,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Gpu {
    pub name: String,
    pub utilization: Option<Percent>,
    pub temperature_c: Option<u32>,
    pub temperature_level: Option<Level>,
    pub vram: Option<Usage>,
}

/// How much attention a value needs. Healthy values are drawn plain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Good,
    Warning,
    High,
}

impl Level {
    fn of(value: f64, limits: Limits) -> Level {
        match limits.tone(value) {
            Tone::Error => Level::High,
            Tone::Warning => Level::Warning,
            _ => Level::Good,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Notice {
    pub kind: NoticeKind,
    pub title: String,
    /// A second line under the title, such as the command that finished.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    pub body: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NoticeKind {
    JobFinished,
    JobFailed,
    JobInterrupted,
    Unreachable,
    Back,
    Resource,
}

impl Status {
    pub fn online(agent: &str, path: &str, health: &Health) -> Status {
        Status {
            agent: agent.to_string(),
            online: true,
            path: Some(path.to_string()),
            error: None,
            problem: None,
            cpu: percent(health.cpu_percent as f64, LOAD),
            memory: usage(health.memory_used_mib, health.memory_total_mib),
            workspace: Some(space(
                health.workspace_free_mib.unwrap_or(health.disk_free_mib),
            )),
            gpus: health.gpus.iter().map(gpu).collect(),
            gpu_problem: health.gpu_problem.clone(),
        }
    }

    pub fn offline(agent: &str, error: String) -> Status {
        Status {
            agent: agent.to_string(),
            online: false,
            path: None,
            problem: Some(explain(agent, &error)),
            error: Some(error),
            cpu: None,
            memory: None,
            workspace: None,
            gpus: Vec::new(),
            gpu_problem: None,
        }
    }
}

/// Turn a connection failure into what it means and what to do. The messages matched here
/// are Slingshot's own, from the config, the control helper, and the iroh diagnosis.
pub fn explain(name: &str, error: &str) -> Problem {
    let text = error.to_lowercase();
    let has = |needles: &[&str]| needles.iter().any(|needle| text.contains(needle));
    let problem = |title: String, detail: String, fix: Option<Fix>| Problem { title, detail, fix };
    let run = |machine, command: &str| {
        Some(Fix {
            machine,
            command: command.to_string(),
        })
    };
    if has(&["no agent configured"]) {
        problem(
            "No box linked yet".to_string(),
            "Run slingshot start on the box, then link this machine with the code it prints."
                .to_string(),
            run(Machine::Client, "slingshot link <code>"),
        )
    } else if has(&["no longer accepts"]) {
        problem(
            format!("{name} no longer accepts this machine"),
            format!("Link again from the same network as {name}."),
            run(Machine::Client, "slingshot link <code>"),
        )
    } else if has(&["not running on the agent", "connection refused"]) {
        problem(
            format!("Slingshot isn't running on {name}"),
            format!("{name} is on, but slingshot start has stopped."),
            run(Machine::Agent, "slingshot start"),
        )
    } else if has(&["cannot reach iroh's relays"]) {
        problem(
            "This machine is offline".to_string(),
            "Check the internet connection, then try again.".to_string(),
            None,
        )
    } else if has(&["versions differ", "update slingshot"]) {
        problem(
            "Slingshot versions differ".to_string(),
            format!("Update Slingshot on this machine and on {name}."),
            None,
        )
    } else if has(&[
        "not reachable",
        "did not answer",
        "timed out",
        "no route to host",
    ]) {
        problem(
            format!("{name} is off or asleep"),
            "Nothing answered on the local network, tailnet, or iroh. Wake it and check that slingshot start is running.".to_string(),
            run(Machine::Agent, "slingshot start"),
        )
    } else {
        problem(format!("Can't reach {name}"), error.to_string(), None)
    }
}

fn percent(value: f64, limits: Limits) -> Option<Percent> {
    (value.is_finite() && (0.0..=100.0).contains(&value)).then(|| Percent {
        percent: value as f32,
        level: Level::of(value, limits),
    })
}

fn usage(used_mib: u64, total_mib: u64) -> Option<Usage> {
    (total_mib > 0 && used_mib <= total_mib).then(|| Usage {
        used_mib,
        total_mib,
        level: Level::of(used_mib as f64 / total_mib as f64 * 100.0, MEMORY),
    })
}

fn space(free_mib: u64) -> Space {
    Space {
        free_mib,
        level: match free_mib < DISK_WARNING_MIB {
            true => Level::High,
            false => Level::Good,
        },
    }
}

fn gpu(gpu: &slingshot_core::protocol::GpuHealth) -> Gpu {
    Gpu {
        name: gpu.name.clone(),
        utilization: gpu
            .utilization_percent
            .and_then(|value| percent(value as f64, LOAD)),
        temperature_c: gpu.temperature_c,
        temperature_level: gpu
            .temperature_c
            .map(|value| Level::of(value as f64, TEMPERATURE)),
        vram: match (gpu.vram_free_mib, gpu.vram_total_mib) {
            (Some(free), Some(total)) if free <= total => usage(total - free, total),
            _ => None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slingshot_core::protocol::GpuHealth;

    fn health() -> Health {
        Health {
            cpu_percent: 40.0,
            memory_used_mib: 30000,
            memory_total_mib: 32768,
            swap_total_mib: 4096,
            disk_free_mib: 500_000,
            workspace_free_mib: Some(1024),
            gpus: vec![GpuHealth {
                name: "RTX 3090".to_string(),
                vram_free_mib: Some(15000),
                vram_total_mib: Some(24576),
                utilization_percent: Some(75),
                temperature_c: Some(60),
            }],
            gpu_problem: None,
        }
    }

    #[test]
    fn a_status_line_carries_values_and_levels() {
        let event = Event::Status(Status::online("archbox", "tailnet", &health()));
        let line = serde_json::to_value(Line {
            version: VERSION,
            event: &event,
        })
        .unwrap();
        assert_eq!(line["version"], 1);
        assert_eq!(line["event"], "status");
        assert_eq!(line["online"], true);
        assert_eq!(line["path"], "tailnet");
        assert_eq!(line["cpu"]["level"], "good");
        assert_eq!(line["memory"]["level"], "high");
        assert_eq!(line["workspace"]["free_mib"], 1024);
        assert_eq!(line["workspace"]["level"], "high");
        assert_eq!(line["gpus"][0]["utilization"]["level"], "warning");
        assert_eq!(line["gpus"][0]["temperature_level"], "good");
        assert_eq!(line["gpus"][0]["vram"]["used_mib"], 24576 - 15000);
    }

    #[test]
    fn invalid_measurements_are_left_out() {
        let mut health = health();
        health.cpu_percent = f32::NAN;
        health.memory_total_mib = 0;
        health.workspace_free_mib = None;
        health.gpus[0].vram_free_mib = Some(99_999);
        let status = Status::online("archbox", "iroh", &health);
        assert!(status.cpu.is_none());
        assert!(status.memory.is_none());
        assert_eq!(status.workspace.unwrap().free_mib, 500_000);
        assert!(status.gpus[0].vram.is_none());
    }

    #[test]
    fn failures_are_explained_with_a_fix_on_the_right_machine() {
        let cases = [
            (
                "Could not reach archbox: Slingshot is not running on the Agent. Run slingshot start there: Connection refused (os error 111)",
                "Slingshot isn't running on archbox",
                Some((Machine::Agent, "slingshot start")),
            ),
            (
                "archbox is not reachable. It may be off or asleep, or slingshot start is not running there",
                "archbox is off or asleep",
                Some((Machine::Agent, "slingshot start")),
            ),
            (
                "archbox did not answer in time",
                "archbox is off or asleep",
                Some((Machine::Agent, "slingshot start")),
            ),
            (
                "archbox no longer accepts this machine. Run slingshot link again from the same network as archbox",
                "archbox no longer accepts this machine",
                Some((Machine::Client, "slingshot link <code>")),
            ),
            (
                "No Agent configured yet\n\nOn the Agent:   slingshot start",
                "No box linked yet",
                Some((Machine::Client, "slingshot link <code>")),
            ),
            (
                "Could not reach archbox, because this machine cannot reach iroh's relays. Check its internet connection",
                "This machine is offline",
                None,
            ),
            (
                "Slingshot versions differ between the machines (protocol 4 and 5). Update Slingshot on both machines",
                "Slingshot versions differ",
                None,
            ),
            ("something new", "Can't reach archbox", None),
        ];
        for (error, title, fix) in cases {
            let problem = explain("archbox", error);
            assert_eq!(problem.title, title, "{error}");
            assert_eq!(
                problem.fix.map(|fix| (fix.machine, fix.command)),
                fix.map(|(machine, command)| (machine, command.to_string())),
                "{error}"
            );
        }
        assert_eq!(explain("archbox", "something new").detail, "something new");
    }

    #[test]
    fn an_offline_line_carries_the_problem() {
        let event = Event::Status(Status::offline(
            "archbox",
            "archbox did not answer in time".to_string(),
        ));
        let line = serde_json::to_value(Line {
            version: VERSION,
            event: &event,
        })
        .unwrap();
        assert_eq!(line["online"], false);
        assert_eq!(line["problem"]["title"], "archbox is off or asleep");
        assert_eq!(line["problem"]["fix"]["machine"], "agent");
        assert_eq!(line["problem"]["fix"]["command"], "slingshot start");
        assert_eq!(line["error"], "archbox did not answer in time");
    }

    #[test]
    fn a_notice_line_is_flat() {
        let event = Event::Notice(Notice {
            kind: NoticeKind::JobFinished,
            title: "t".to_string(),
            subtitle: None,
            body: "b".to_string(),
        });
        let text = serde_json::to_string(&Line {
            version: VERSION,
            event: &event,
        })
        .unwrap();
        assert_eq!(
            text,
            r#"{"version":1,"event":"notice","kind":"job_finished","title":"t","body":"b"}"#
        );
    }
}
