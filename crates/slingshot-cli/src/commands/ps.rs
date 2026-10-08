//! `slingshot ps` and `slingshot stop`: Slingshot runs and sessions on the Agent.

use crate::client::{self, unexpected};
use slingshot_core::config::Config;
use slingshot_core::control::{Job, JobKind, JobState, Request, Response};
use slingshot_core::presentation::{self, Style, Tone};
use slingshot_core::step;
use slingshot_core::storage;

pub async fn ps(agent: Option<String>, all: bool) -> anyhow::Result<i32> {
    let config = Config::load()?;
    let target = config.resolve(agent.as_deref())?;
    let Response::Jobs(jobs) = client::fetch(target, Request::Jobs { all }).await? else {
        return Err(unexpected());
    };
    print!(
        "\n{}",
        render(
            &target.name,
            &jobs,
            all,
            storage::now(),
            width(),
            Style::stdout()
        )
    );
    Ok(0)
}

/// The terminal's width, so long commands are cut to fit. Output sent to a file or a
/// pipe keeps every command whole.
pub fn width() -> Option<usize> {
    use std::io::IsTerminal;

    std::io::stdout()
        .is_terminal()
        .then(|| crossterm::terminal::size().ok())
        .flatten()
        .map(|(columns, _)| usize::from(columns))
}

/// Everything before the command column: the indent and five columns with their gaps.
const BEFORE_COMMAND: usize = 2 + 8 + 2 + 7 + 2 + 12 + 2 + 16 + 2 + 9 + 2;

pub async fn stop(agent: Option<String>, id: String) -> anyhow::Result<i32> {
    let config = Config::load()?;
    let target = config.resolve(agent.as_deref())?;
    let stopping = step::start(format!("Stopping {id} on {}", target.name));
    let Response::Job(job) = client::request(target, Request::Stop { job: id }).await? else {
        return Err(unexpected());
    };
    stopping.done(format!(
        "Stopped {} ({})",
        job.command,
        storage::short_id(&job.id)
    ));
    presentation::detail("Kept", "source and build output");
    Ok(0)
}

/// What the state column says. An interruption other than Ctrl C means the connection was
/// lost, and only a failure shows its exit code, since it is the one worth looking up.
fn state_label(job: &Job) -> String {
    match (job.state, job.exit_code) {
        (JobState::Interrupted, Some(130)) => job.state.label().to_string(),
        (JobState::Interrupted, _) if job.kind == JobKind::Run => "Lost connection".to_string(),
        (JobState::Failed, Some(code)) => format!("{} {code}", job.state.label()),
        _ => job.state.label().to_string(),
    }
}

pub fn render(
    agent: &str,
    jobs: &[Job],
    all: bool,
    now: u64,
    width: Option<usize>,
    style: Style,
) -> String {
    let heading = match all {
        true => format!("Jobs on {agent}"),
        false => format!("Active jobs on {agent}"),
    };
    let mut output = format!("{}\n\n", style.heading(heading));
    if jobs.is_empty() {
        match all {
            true => output.push_str("  No jobs recorded\n"),
            false => output.push_str(&format!(
                "  Nothing running. See finished jobs with {}\n",
                style.paint("slingshot ps --all", Tone::Info)
            )),
        }
        return output;
    }
    let header = format!(
        "{:<8}  {:<7}  {:<15}  {:<16}  {:<9}  {}",
        "ID", "KIND", "STATE", "PROJECT", "STARTED", "COMMAND"
    );
    output.push_str(&format!("  {}\n", style.dim(header)));
    for job in jobs {
        let kind = match job.kind {
            JobKind::Run => "Run",
            JobKind::Session => "Session",
        };
        let tone = match job.state {
            JobState::Running => Some(Tone::Good),
            JobState::Completed | JobState::Ended => None,
            JobState::Stopped | JobState::Interrupted => Some(Tone::Warning),
            JobState::Failed => Some(Tone::Error),
        };
        let state = state_label(job);
        let project = match &job.project_name {
            Some(name) => format!("{:<16}", name.chars().take(16).collect::<String>()),
            None => style.dim(format!("{:<16}", "home")),
        };
        output.push_str(&format!(
            "  {:<8}  {:<7}  {}  {}  {:<9}  {}\n",
            storage::short_id(&job.id),
            kind,
            match tone {
                Some(tone) => style.paint(format!("{state:<15}"), tone),
                None => format!("{state:<15}"),
            },
            project,
            ago(now.saturating_sub(job.started)),
            fit(
                &job.command,
                width.map(|width| width.saturating_sub(BEFORE_COMMAND))
            )
        ));
    }
    output
}

/// A command on one line, cut with an ellipsis when it is wider than `room`, so a long
/// or multi line command never breaks the table.
fn fit(command: &str, room: Option<usize>) -> String {
    let mut lines = command.lines();
    let mut text = lines.next().unwrap_or_default().trim_end().to_string();
    if lines.next().is_some() {
        text.push_str(" …");
    }
    match room.map(|room| room.max(10)) {
        Some(room) if text.chars().count() > room => {
            let kept: String = text.chars().take(room - 1).collect();
            format!("{kept}…")
        }
        _ => text,
    }
}

fn ago(seconds: u64) -> String {
    match seconds {
        0..60 => format!("{seconds}s ago"),
        60..3600 => format!("{}m ago", seconds / 60),
        3600..86400 => format!("{}h ago", seconds / 3600),
        _ => format!("{}d ago", seconds / 86400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(kind: JobKind, state: JobState, started: u64) -> Job {
        Job {
            id: "1a2b3c4d5e6f77889900aabbccddeeff".into(),
            kind,
            project: Some("p".into()),
            project_name: Some("app".into()),
            command: "cargo build --release".into(),
            state,
            started,
            ended: None,
            exit_code: None,
            pid: None,
            process_start: None,
            boot: 0,
            stop_requested: false,
        }
    }

    #[test]
    fn jobs_are_listed_with_short_ids_and_readable_ages() {
        let mut failed = job(JobKind::Run, JobState::Failed, 1000);
        failed.exit_code = Some(101);
        let jobs = vec![job(JobKind::Session, JobState::Running, 9_950), failed];
        let text = render("archbox", &jobs, true, 10_000, None, Style::new(false));
        assert!(text.starts_with("Jobs on archbox\n\n"));
        assert!(text.contains(
            "1a2b3c4d  Session  Running          app               50s ago    cargo build --release"
        ));
        assert!(text.contains("Failed 101"));
        assert!(text.contains("2h ago"));
        assert!(!text.contains('\x1b'));
    }

    #[test]
    fn a_run_cut_off_by_the_network_reads_as_a_lost_connection() {
        let mut lost = job(JobKind::Run, JobState::Interrupted, 1000);
        lost.exit_code = Some(141);
        assert_eq!(state_label(&lost), "Lost connection");
        lost.exit_code = Some(130);
        assert_eq!(state_label(&lost), "Interrupted");
        let mut failed = job(JobKind::Run, JobState::Failed, 1000);
        failed.exit_code = Some(101);
        assert_eq!(state_label(&failed), "Failed 101");
    }

    #[test]
    fn only_failures_show_their_exit_code() {
        let mut interrupted = job(JobKind::Run, JobState::Interrupted, 1000);
        interrupted.exit_code = Some(130);
        let mut stopped = job(JobKind::Run, JobState::Stopped, 1000);
        stopped.exit_code = Some(130);
        let text = render(
            "archbox",
            &[interrupted, stopped],
            true,
            1010,
            None,
            Style::new(false),
        );
        assert!(text.contains("Interrupted      app"), "{text}");
        assert!(text.contains("Stopped          app"), "{text}");
        assert!(!text.contains("130"), "{text}");
    }

    #[test]
    fn an_empty_list_explains_itself() {
        let text = render("archbox", &[], false, 0, None, Style::new(false));
        assert!(text.contains("Nothing running. See finished jobs with slingshot ps --all"));
    }

    #[test]
    fn long_and_multi_line_commands_fit_on_one_line() {
        assert_eq!(fit("cargo build", Some(40)), "cargo build");
        assert_eq!(fit("sh -c 'set -e\necho hi'", None), "sh -c 'set -e …");
        assert_eq!(fit("abcdefghijklmnop", Some(12)), "abcdefghijk…");
        assert_eq!(fit("abcdefghijklmnop", Some(2)), "abcdefghi…");
        assert_eq!(fit("abcdefghijklmnop", None), "abcdefghijklmnop");
    }

    #[test]
    fn a_job_outside_a_project_runs_in_home() {
        let mut outside = job(JobKind::Run, JobState::Completed, 1000);
        outside.project_name = None;
        let text = render("archbox", &[outside], true, 1010, None, Style::new(false));
        assert!(text.contains("Completed        home"), "{text}");
    }
}
