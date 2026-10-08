//! `slingshot attach [path]`: open or rejoin a persistent session on the Agent, in the
//! current project's copy, or in the Agent's home folder when there is no project.

use crate::client::{self, Refused, unexpected};
use crate::project::{self, Local};
use crate::route;
use crate::ssh::{self, RemoteCommand};
use crate::transfer::{self, Conflicts, Direction};
use slingshot_core::config::{Agent, Config};
use slingshot_core::control::{Request, Response, SessionInfo};
use slingshot_core::presentation::{self, Style, Tone};

pub async fn attach(
    agent: Option<String>,
    path: Option<std::path::PathBuf>,
) -> anyhow::Result<i32> {
    if !ssh::wants_terminal() {
        anyhow::bail!("slingshot attach needs an interactive terminal");
    }
    let config = Config::load()?;
    let target = config.resolve(agent.as_deref())?;
    let local = match path {
        Some(path) => Some(project::require(Some(path))?),
        None => project::locate(&std::env::current_dir()?),
    };

    let (session, place) = match &local {
        Some(local) => (project_session(target, local).await?, local.name.clone()),
        None => (home_session(target).await?, "home folder".to_string()),
    };

    let verb = match session.created {
        true => "Starting",
        false => "Attaching to",
    };
    let style = Style::stderr();
    eprintln!(
        "{} {verb} a session on {} via {} · {place}",
        style.paint("▶", Tone::Info),
        style.paint(&target.name, Tone::Info),
        style.paint(route::resolve(target).name(), Tone::Info),
    );
    eprintln!(
        "  {}",
        style.dim("Detach with Ctrl B then D, or your tmux prefix then D")
    );

    let mut remote = RemoteCommand::to(
        target,
        session.tmux,
        vec![
            "-u".to_string(),
            "-S".to_string(),
            session.socket,
            "attach-session".to_string(),
            "-t".to_string(),
            format!("={}", session.job.id),
        ],
    );
    remote.tty = true;
    remote.quiet = true;
    let lost = format!(
        "Lost connection to {}. The session keeps running there\n{}",
        target.name,
        presentation::row("Return", "slingshot attach").trim_end()
    );
    let code = super::run::interact(target, &remote, lost).await?;
    if pulls_after(code)
        && let Err(error) = after_leaving(target, local.as_ref(), &session.job.id).await
    {
        presentation::warning(format!("Did not bring back edits: {error:#}"));
    }
    Ok(code)
}

/// A session counts as left, rather than ended, unless the Agent answered and no longer lists
/// it, because leaving with Ctrl B, D is the usual case.
fn still_running(answer: Option<Response>, session: &str) -> bool {
    match answer {
        Some(Response::Jobs(jobs)) => jobs.iter().any(|job| job.id == session),
        _ => true,
    }
}

fn say_left(target: &Agent, running: bool) {
    match running {
        true => {
            presentation::success(format!(
                "Left the session on {}. It keeps running",
                target.name
            ));
            presentation::detail("Return", "slingshot attach");
        }
        false => presentation::success(format!("The session on {} ended", target.name)),
    }
}

/// Only a clean detach, or the session ending, pulls. After a dropped connection the session
/// still runs, and a coding agent there may be halfway through writing a file.
fn pulls_after(code: i32) -> bool {
    code == 0
}

/// Say whether the session was left or ended, then bring back what changed on the Agent
/// during it, such as a coding agent's edits, as `slingshot sync --pull` does. A failure is
/// a warning, because the session itself ended well and the edits wait safely on the Agent.
async fn after_leaving(target: &Agent, local: Option<&Local>, session: &str) -> anyhow::Result<()> {
    let Some(local) = local else {
        let answer = client::request(target, Request::Jobs { all: false }).await;
        say_left(target, still_running(answer.ok(), session));
        return Ok(());
    };
    let mut opened = transfer::open_quietly(target, local).await?;
    let answer = opened.control.call(Request::Jobs { all: false }).await;
    say_left(target, still_running(answer.ok(), session));
    let step = transfer::syncing(local);
    let result = transfer::pull(&mut opened, target, local, &step).await;
    opened.control.close().await;
    match result {
        Ok(outcome) => {
            transfer::finish(step, &outcome, Direction::Pull, &target.name);
            super::sync::report_kept(outcome.kept, &target.name, Direction::Pull);
            Ok(())
        }
        Err(error) => {
            step.clear();
            Err(error)
        }
    }
}

/// Sync first, so the session always starts from the latest edits, even when returning to
/// one that is already running. Once the project has a copy, a sync the Agent refuses or
/// a conflict is reported and the session opens anyway, because being locked out of
/// running work is worse than working on files that are not the newest.
async fn project_session(target: &Agent, local: &Local) -> anyhow::Result<SessionInfo> {
    let mut opened = transfer::open(target, local).await?;
    let step = transfer::syncing(local);
    match transfer::push(&mut opened, target, local, &step).await {
        Ok(outcome) => transfer::finish(step, &outcome, Direction::Push, &target.name),
        Err(error)
            if opened.project.initialized
                && (error.downcast_ref::<Conflicts>().is_some()
                    || error.downcast_ref::<Refused>().is_some()) =>
        {
            step.clear();
            presentation::warning(format!("Did not sync {}: {error:#}", local.name));
        }
        Err(error) => {
            step.clear();
            return Err(error);
        }
    }
    let session = open_session(&mut opened.control, Some(opened.id.clone())).await?;
    opened.control.close().await;
    Ok(session)
}

/// The Agent's home folder needs no copy, so this only connects and opens the session.
async fn home_session(target: &Agent) -> anyhow::Result<SessionInfo> {
    let connecting = client::connecting(target);
    let mut control = client::Control::connect(target).await?;
    let session = open_session(&mut control, None).await;
    match session.is_ok() {
        true => client::connected(connecting, target),
        false => connecting.clear(),
    }
    control.close().await;
    session
}

/// Ask for the session, and show resource warnings when a new one starts.
async fn open_session(
    control: &mut client::Control,
    project: Option<String>,
) -> anyhow::Result<SessionInfo> {
    let Response::Session(session) = control.call(Request::Session { project }).await? else {
        return Err(unexpected());
    };
    if session.created {
        super::run::show_warnings(control.call(Request::Warnings).await);
    }
    Ok(session)
}

#[cfg(test)]
mod tests {
    use super::*;
    use slingshot_core::control::{Job, JobKind, JobState};

    #[test]
    fn a_session_counts_as_ended_only_when_the_agent_no_longer_lists_it() {
        let session = |id: &str| Job {
            id: id.to_string(),
            kind: JobKind::Session,
            project: None,
            project_name: None,
            command: "Shell session".to_string(),
            state: JobState::Running,
            started: 0,
            ended: None,
            exit_code: None,
            pid: None,
            process_start: None,
            boot: 0,
            stop_requested: false,
        };
        assert!(still_running(
            Some(Response::Jobs(vec![session("s1")])),
            "s1"
        ));
        assert!(!still_running(
            Some(Response::Jobs(vec![session("s2")])),
            "s1"
        ));
        assert!(!still_running(Some(Response::Jobs(Vec::new())), "s1"));
        assert!(still_running(None, "s1"));
    }

    #[test]
    fn only_a_clean_detach_pulls() {
        assert!(pulls_after(0));
        assert!(!pulls_after(255));
        assert!(!pulls_after(1));
    }
}
