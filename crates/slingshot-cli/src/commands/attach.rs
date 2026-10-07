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
    let lost = format!(
        "Lost connection to {}. The session keeps running there. Run slingshot attach again to return to it",
        target.name
    );
    let code = super::run::interact(target, &remote, lost).await?;
    if let Some(local) = local.as_ref().filter(|_| pulls_after(code))
        && let Err(error) = pull_edits(target, local).await
    {
        presentation::warning(format!("Did not pull {}: {error:#}", local.name));
    }
    Ok(code)
}

/// Only a clean detach, or the session ending, pulls. After a dropped connection the session
/// still runs, and a coding agent there may be halfway through writing a file.
fn pulls_after(code: i32) -> bool {
    code == 0
}

/// Bring back what changed on the Agent during the session, such as a coding agent's edits,
/// as `slingshot sync --pull` does. A failure is a warning, because the session itself ended
/// well and the edits wait safely on the Agent.
async fn pull_edits(target: &Agent, local: &Local) -> anyhow::Result<()> {
    let mut opened = transfer::open(target, local).await?;
    let step = transfer::syncing(local);
    let result = transfer::pull(&mut opened, target, local, &step).await;
    opened.control.close().await;
    match result {
        Ok(outcome) => {
            transfer::finish(step, &outcome, Direction::Pull);
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
        Ok(outcome) => transfer::finish(step, &outcome, Direction::Push),
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

    #[test]
    fn only_a_clean_detach_pulls() {
        assert!(pulls_after(0));
        assert!(!pulls_after(255));
        assert!(!pulls_after(1));
    }
}
