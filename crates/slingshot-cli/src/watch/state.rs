//! Deciding when something is worth a notification, and saying it the way a person would.
//! Each observation of the Agent goes in, and the notices it earns come out, so the rules
//! can be tested without an Agent.

use super::event::{self, Machine, Notice, NoticeKind};
use crate::commands::health::{MEMORY, TEMPERATURE};
use slingshot_core::control::{Job, JobKind, JobState};
use slingshot_core::presentation::{capacity, plural};
use slingshot_core::protocol::Health;
use slingshot_core::telemetry::DISK_WARNING_MIB;
use std::collections::HashSet;

/// A run shorter than this finished while you were still looking at it.
const WORTH_NOTICING: u64 = 10;

/// Failed checks in a row before the Agent counts as unreachable, so one slow answer is not
/// reported as an outage.
const FAILURES_BEFORE_UNREACHABLE: u32 = 2;

/// Free workspace space must recover this far past the warning before it can warn again.
const DISK_RECOVERED_MIB: u64 = DISK_WARNING_MIB * 2;

const COMMAND_WIDTH: usize = 40;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Alarm {
    Memory,
    Workspace,
    Temperature(usize),
}

pub struct Watch {
    name: String,
    /// Finished jobs already accounted for. Empty until the first answer sets the baseline.
    known: Option<HashSet<String>>,
    failures: u32,
    unreachable: bool,
    alarms: HashSet<Alarm>,
}

impl Watch {
    pub fn new(name: &str) -> Watch {
        Watch {
            name: name.to_string(),
            known: None,
            failures: 0,
            unreachable: false,
            alarms: HashSet::new(),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// Failed checks since the Agent last answered.
    pub fn failures(&self) -> u32 {
        self.failures
    }

    /// The Agent answered, over the path named `path`.
    pub fn reached(&mut self, path: &str) -> Vec<Notice> {
        self.failures = 0;
        if !std::mem::take(&mut self.unreachable) {
            return Vec::new();
        }
        vec![Notice {
            kind: NoticeKind::Back,
            title: format!("{} is back", self.name),
            subtitle: None,
            body: format!("Connected via {path}."),
        }]
    }

    /// The Agent did not answer, for the reason in `error`. The notice uses the same plain
    /// words as the offline screen rather than the raw error.
    pub fn failed(&mut self, error: &str) -> Vec<Notice> {
        self.failures += 1;
        if self.unreachable || self.failures < FAILURES_BEFORE_UNREACHABLE {
            return Vec::new();
        }
        self.unreachable = true;
        let problem = event::explain(&self.name, error);
        let body = match problem.fix {
            Some(fix) => format!(
                "Run {} on {}.",
                fix.command,
                match fix.machine {
                    Machine::Agent => self.name.as_str(),
                    Machine::Client => "this machine",
                }
            ),
            None => problem.detail,
        };
        vec![Notice {
            kind: NoticeKind::Unreachable,
            title: format!("{} went offline", self.name),
            subtitle: Some(problem.title),
            body,
        }]
    }

    /// Jobs that finished since the last list. The first list only sets the baseline, so
    /// starting the menu bar never replays history.
    pub fn jobs(&mut self, jobs: &[Job]) -> Vec<Notice> {
        let finished: Vec<&Job> = jobs.iter().filter(|job| !job.state.active()).collect();
        let Some(known) = &mut self.known else {
            self.known = Some(finished.iter().map(|job| job.id.clone()).collect());
            return Vec::new();
        };
        let notices = finished
            .iter()
            .filter(|job| known.insert(job.id.clone()))
            .filter_map(|job| notice(&self.name, job))
            .collect();
        let listed: HashSet<&str> = jobs.iter().map(|job| job.id.as_str()).collect();
        known.retain(|id| listed.contains(id.as_str()));
        notices
    }

    /// Resources that just crossed into trouble. Each warns once, then again only after
    /// it has clearly recovered.
    pub fn health(&mut self, health: &Health) -> Vec<Notice> {
        let mut notices = Vec::new();
        let (used, total) = (health.memory_used_mib, health.memory_total_mib);
        if total > 0 && used <= total {
            let percent = used as f64 / total as f64 * 100.0;
            if self.alarm(
                Alarm::Memory,
                percent >= MEMORY.high,
                percent < MEMORY.warning,
            ) {
                notices.push(resource(
                    format!("{} is low on memory", self.name),
                    format!(
                        "{} of {} in use ({percent:.0}%).",
                        capacity(used),
                        capacity(total)
                    ),
                ));
            }
        }
        let free = health.workspace_free_mib.unwrap_or(health.disk_free_mib);
        if self.alarm(
            Alarm::Workspace,
            free < DISK_WARNING_MIB,
            free >= DISK_RECOVERED_MIB,
        ) {
            notices.push(resource(
                format!("{} is low on disk space", self.name),
                format!("{} left for projects and build output.", capacity(free)),
            ));
        }
        for (index, gpu) in health.gpus.iter().enumerate() {
            let Some(celsius) = gpu.temperature_c else {
                continue;
            };
            let value = celsius as f64;
            if self.alarm(
                Alarm::Temperature(index),
                value >= TEMPERATURE.high,
                value < TEMPERATURE.warning,
            ) {
                notices.push(resource(
                    format!("{}'s GPU is running hot", self.name),
                    format!("{} is at {celsius}°C.", gpu.name),
                ));
            }
        }
        notices
    }

    /// True when `alarm` should fire now. It re-arms only once `recovered` holds.
    fn alarm(&mut self, alarm: Alarm, triggered: bool, recovered: bool) -> bool {
        if recovered {
            self.alarms.remove(&alarm);
            return false;
        }
        triggered && self.alarms.insert(alarm)
    }
}

fn resource(title: String, body: String) -> Notice {
    Notice {
        kind: NoticeKind::Resource,
        title,
        subtitle: None,
        body,
    }
}

/// What a finished job is worth saying. The command is the subtitle, so the title can stay
/// short and plain. Stopping is something the person just did, and a session usually ends
/// because someone left it, so only surprises about those are told.
fn notice(name: &str, job: &Job) -> Option<Notice> {
    let took = job.ended?.saturating_sub(job.started);
    let place = job
        .project_name
        .as_ref()
        .map(|project| format!(" in {project}"))
        .unwrap_or_default();
    let what = match job.kind {
        JobKind::Run => short(&job.command),
        JobKind::Session => "Session".to_string(),
    };
    let (kind, title, body) = match (job.kind, job.state) {
        (_, JobState::Interrupted) => (
            NoticeKind::JobInterrupted,
            format!("Interrupted on {name}"),
            format!(
                "Stopped after {}{place}. The connection dropped or {name} restarted.",
                spoken(took)
            ),
        ),
        (JobKind::Run, _) if took < WORTH_NOTICING => return None,
        (JobKind::Run, JobState::Completed) => (
            NoticeKind::JobFinished,
            format!("Finished on {name}"),
            format!("Took {}{place}.", spoken(took)),
        ),
        (JobKind::Run, JobState::Failed) => (
            NoticeKind::JobFailed,
            format!("Failed on {name}"),
            match job.exit_code {
                Some(code) => format!("Exit code {code} after {}{place}.", spoken(took)),
                None => format!("Failed after {}{place}.", spoken(took)),
            },
        ),
        _ => return None,
    };
    Some(Notice {
        kind,
        title,
        subtitle: Some(what),
        body,
    })
}

/// `12 seconds`, `3 minutes`, or `1 hour 5 minutes`, as a sentence would say it.
fn spoken(seconds: u64) -> String {
    let (hours, minutes) = (seconds / 3600, seconds % 3600 / 60);
    match (hours, minutes) {
        (0, 0) => plural(seconds as usize, "second"),
        (0, minutes) => plural(minutes as usize, "minute"),
        (hours, 0) => plural(hours as usize, "hour"),
        (hours, minutes) => format!(
            "{} {}",
            plural(hours as usize, "hour"),
            plural(minutes as usize, "minute")
        ),
    }
}

fn short(command: &str) -> String {
    match command.chars().count() > COMMAND_WIDTH {
        true => format!(
            "{}…",
            command.chars().take(COMMAND_WIDTH - 1).collect::<String>()
        ),
        false => command.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slingshot_core::protocol::GpuHealth;

    fn job(id: &str, kind: JobKind, state: JobState, took: u64) -> Job {
        Job {
            id: id.to_string(),
            kind,
            project: Some("p".to_string()),
            project_name: Some("app".to_string()),
            command: "cargo build --release".to_string(),
            state,
            started: 1_000,
            ended: (!state.active()).then_some(1_000 + took),
            exit_code: match state {
                JobState::Completed => Some(0),
                JobState::Failed => Some(101),
                _ => None,
            },
            pid: None,
            process_start: None,
            boot: 0,
            stop_requested: false,
        }
    }

    fn run(id: &str, state: JobState, took: u64) -> Job {
        job(id, JobKind::Run, state, took)
    }

    fn health(memory_used_mib: u64, workspace_free_mib: u64, celsius: u32) -> Health {
        Health {
            cpu_percent: 10.0,
            memory_used_mib,
            memory_total_mib: 100,
            swap_total_mib: 0,
            disk_free_mib: 0,
            workspace_free_mib: Some(workspace_free_mib),
            gpus: vec![GpuHealth {
                name: "RTX 3090".to_string(),
                vram_free_mib: None,
                vram_total_mib: None,
                utilization_percent: None,
                temperature_c: Some(celsius),
            }],
            gpu_problem: None,
        }
    }

    const PLENTY: u64 = 100_000;

    #[test]
    fn the_first_list_sets_a_baseline() {
        let mut watch = Watch::new("archbox");
        let old = [
            run("a", JobState::Completed, 60),
            run("b", JobState::Failed, 60),
        ];
        assert!(watch.jobs(&old).is_empty());
        assert!(watch.jobs(&old).is_empty());
    }

    #[test]
    fn a_long_run_that_ends_is_told_once() {
        let mut watch = Watch::new("archbox");
        watch.jobs(&[run("a", JobState::Running, 0)]);
        let notices = watch.jobs(&[run("a", JobState::Completed, 192)]);
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].kind, NoticeKind::JobFinished);
        assert_eq!(notices[0].title, "Finished on archbox");
        assert_eq!(
            notices[0].subtitle.as_deref(),
            Some("cargo build --release")
        );
        assert_eq!(notices[0].body, "Took 3 minutes in app.");
        assert!(watch.jobs(&[run("a", JobState::Completed, 192)]).is_empty());
    }

    #[test]
    fn a_failure_names_the_exit_code() {
        let mut watch = Watch::new("archbox");
        watch.jobs(&[]);
        let notices = watch.jobs(&[run("a", JobState::Failed, 12)]);
        assert_eq!(notices[0].kind, NoticeKind::JobFailed);
        assert_eq!(notices[0].title, "Failed on archbox");
        assert_eq!(notices[0].body, "Exit code 101 after 12 seconds in app.");
    }

    #[test]
    fn short_and_stopped_runs_and_ended_sessions_stay_quiet() {
        let mut watch = Watch::new("archbox");
        watch.jobs(&[]);
        assert!(
            watch
                .jobs(&[
                    run("a", JobState::Completed, 9),
                    run("b", JobState::Failed, 2),
                    run("c", JobState::Stopped, 600),
                    job("d", JobKind::Session, JobState::Ended, 600),
                ])
                .is_empty()
        );
    }

    #[test]
    fn interruptions_are_always_told() {
        let mut watch = Watch::new("archbox");
        watch.jobs(&[]);
        let notices = watch.jobs(&[
            job("a", JobKind::Session, JobState::Interrupted, 3),
            run("b", JobState::Interrupted, 1),
        ]);
        assert_eq!(notices.len(), 2);
        assert!(
            notices
                .iter()
                .all(|notice| notice.kind == NoticeKind::JobInterrupted)
        );
        assert_eq!(notices[0].title, "Interrupted on archbox");
        assert_eq!(notices[0].subtitle.as_deref(), Some("Session"));
        assert_eq!(
            notices[1].body,
            "Stopped after 1 second in app. The connection dropped or archbox restarted."
        );
    }

    #[test]
    fn long_commands_are_shortened() {
        let long = "x".repeat(60);
        let shortened = short(&long);
        assert_eq!(shortened.chars().count(), COMMAND_WIDTH);
        assert!(shortened.ends_with('…'));
    }

    #[test]
    fn durations_are_spoken() {
        assert_eq!(spoken(1), "1 second");
        assert_eq!(spoken(48), "48 seconds");
        assert_eq!(spoken(192), "3 minutes");
        assert_eq!(spoken(3600), "1 hour");
        assert_eq!(spoken(3900), "1 hour 5 minutes");
    }

    #[test]
    fn unreachable_needs_two_failures_and_is_told_once_then_back() {
        let refused = "Could not reach archbox: Slingshot is not running on the Agent. Run slingshot start there: Connection refused (os error 111)";
        let mut watch = Watch::new("archbox");
        assert!(watch.reached("local network").is_empty());
        assert!(watch.failed(refused).is_empty());
        let notices = watch.failed(refused);
        assert_eq!(notices[0].kind, NoticeKind::Unreachable);
        assert_eq!(notices[0].title, "archbox went offline");
        assert_eq!(
            notices[0].subtitle.as_deref(),
            Some("Slingshot isn't running on archbox")
        );
        assert_eq!(notices[0].body, "Run slingshot start on archbox.");
        assert!(watch.failed(refused).is_empty());
        let notices = watch.reached("tailnet");
        assert_eq!(notices[0].kind, NoticeKind::Back);
        assert_eq!(notices[0].body, "Connected via tailnet.");
        assert!(watch.reached("tailnet").is_empty());
    }

    #[test]
    fn an_offline_cause_without_a_fix_explains_itself() {
        let mut watch = Watch::new("archbox");
        watch.failed("something new");
        let notices = watch.failed("something new");
        assert_eq!(notices[0].subtitle.as_deref(), Some("Can't reach archbox"));
        assert_eq!(notices[0].body, "something new");
    }

    #[test]
    fn one_failure_between_answers_is_not_an_outage() {
        let mut watch = Watch::new("archbox");
        watch.failed("slow");
        watch.reached("local network");
        assert!(watch.failed("slow").is_empty());
    }

    #[test]
    fn resources_warn_once_and_rearm_after_recovering() {
        let mut watch = Watch::new("archbox");
        assert!(watch.health(&health(50, PLENTY, 60)).is_empty());
        let notices = watch.health(&health(95, PLENTY, 60));
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].title, "archbox is low on memory");
        assert_eq!(notices[0].body, "95.0 MiB of 100.0 MiB in use (95%).");
        assert!(watch.health(&health(95, PLENTY, 60)).is_empty());
        assert!(watch.health(&health(80, PLENTY, 60)).is_empty());
        assert!(watch.health(&health(92, PLENTY, 60)).is_empty());
        assert!(watch.health(&health(70, PLENTY, 60)).is_empty());
        assert_eq!(watch.health(&health(91, PLENTY, 60)).len(), 1);
    }

    #[test]
    fn disk_and_temperature_warn_with_their_own_limits() {
        let mut watch = Watch::new("archbox");
        let notices = watch.health(&health(10, 1024, 90));
        let titles: Vec<_> = notices.iter().map(|notice| notice.title.as_str()).collect();
        assert_eq!(
            titles,
            [
                "archbox is low on disk space",
                "archbox's GPU is running hot"
            ]
        );
        assert_eq!(
            notices[0].body,
            "1.0 GiB left for projects and build output."
        );
        assert_eq!(notices[1].body, "RTX 3090 is at 90°C.");
        assert!(watch.health(&health(10, DISK_WARNING_MIB, 80)).is_empty());
        assert!(watch.health(&health(10, 1024, 90)).is_empty());
        assert!(watch.health(&health(10, DISK_RECOVERED_MIB, 70)).is_empty());
        assert_eq!(watch.health(&health(10, 1024, 90)).len(), 2);
    }
}
