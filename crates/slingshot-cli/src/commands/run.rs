//! `slingshot run <cmd>`: sync the project, then run a command in the Agent copy.

use crate::client::{self, Refused};
use crate::project::{self, Local};
use crate::route;
use crate::ssh::{Disconnected, RemoteCommand};
use crate::transfer::{self, Direction};
use slingshot_core::artifacts;
use slingshot_core::config::{Agent, Config};
use slingshot_core::control::{Request, Response};
use slingshot_core::presentation::{self, Style, Tone};
use slingshot_core::step;
use std::time::{Duration, Instant};

/// Run `cmd` on the Agent and return its exit code. A non zero code is not an
/// error: slingshot did its job, and the command it ran happened to fail.
pub async fn run(agent: Option<String>, cmd: Vec<String>) -> anyhow::Result<i32> {
    if cmd.is_empty() {
        anyhow::bail!("No command given. Try slingshot run echo hello");
    }

    let started = Instant::now();
    let config = Config::load()?;
    let target = config.resolve(agent.as_deref())?;
    let local = project::locate(&std::env::current_dir()?);

    let mut args = vec!["internal-run".to_string()];
    let warnings = match &local {
        Some(local) => {
            let mut opened = transfer::open(target, local).await?;
            let step = transfer::syncing(local);
            let outcome = transfer::push(&mut opened, target, local, &step).await?;
            transfer::finish(step, &outcome, Direction::Push);
            let warnings = opened.control.call(Request::Warnings).await;
            opened.control.close().await;
            args.extend(["--project".to_string(), opened.id]);
            if !local.cwd.is_empty() {
                args.extend(["--cwd".to_string(), local.cwd.clone()]);
            }
            warnings
        }
        None => {
            let connecting = client::connecting(target);
            let warnings = client::request(target, Request::Warnings).await;
            match warnings.is_ok() {
                true => client::connected(connecting, target),
                false => connecting.clear(),
            }
            warnings
        }
    };
    show_warnings(warnings);

    args.push("--".to_string());
    args.extend(cmd);

    let style = Style::stderr();
    let path = route::resolve(target).name();
    eprintln!(
        "{}\n",
        announcement(&target.name, path, local.as_ref(), style)
    );

    let remote = RemoteCommand::to(target, target.program().to_string(), args);
    let code = interact(target, &remote, lost_connection(&target.name)).await?;
    eprintln!("\n{}", finished(code, started.elapsed(), style));
    Ok(code)
}

/// The last line of a run: how long it took and how the command exited. Exit 130 means
/// the command ended on Ctrl C or `slingshot stop`, which someone chose, so it is not a failure.
fn finished(code: i32, took: Duration, style: Style) -> String {
    let took = step::elapsed(took);
    match code {
        0 => style.status(format!("Done in {took} · exit 0"), Tone::Good),
        STOPPED => style.status(format!("Stopped in {took} · exit {code}"), Tone::Warning),
        code => style.status(format!("Failed in {took} · exit {code}"), Tone::Error),
    }
}

/// 128 plus SIGINT, the exit code of a command ended by Ctrl C or `slingshot stop`.
const STOPPED: i32 = 130;

/// Run an interactive remote command, replacing SSH's messages about a dropped connection
/// with `lost`. When SSH exits 255 silently, a quick check tells a keepalive timeout
/// apart from a command that really exited with 255.
pub async fn interact(target: &Agent, remote: &RemoteCommand, lost: String) -> anyhow::Result<i32> {
    let error = match remote.interactive().await {
        Ok(code) => return Ok(code),
        Err(error) => error,
    };
    match error.downcast_ref::<Disconnected>() {
        Some(Disconnected::Certain) => anyhow::bail!(lost),
        Some(Disconnected::Possible) if RemoteCommand::reachable(target).await => Ok(255),
        Some(_) => anyhow::bail!(lost),
        None => Err(error),
    }
}

/// Said instead of SSH's own messages. A run belongs to its connection, so the Agent
/// stops it once it notices, and sessions are the way to outlive a disconnect.
fn lost_connection(name: &str) -> String {
    format!(
        "Lost connection to {name}. The Agent stops the run once it notices, unless it finishes first. See how it ended with slingshot ps --all, and use slingshot attach for work that must survive a disconnect"
    )
}

/// Show resource warnings. A failed check is ignored, because it must never block work,
/// but an Agent that refuses the request is worth mentioning.
pub fn show_warnings(result: anyhow::Result<Response>) {
    match result {
        Ok(Response::Warnings(warnings)) => {
            for warning in warnings {
                presentation::warning(warning);
            }
        }
        Err(error) if error.downcast_ref::<Refused>().is_some() => {
            presentation::warning(format!("Could not check Agent resources: {error:#}"))
        }
        _ => {}
    }
}

/// The line printed before anything runs. Saying where the work happens is a hard
/// requirement, including the path taken, which project folder, and what build output moved.
fn announcement(name: &str, path: &str, local: Option<&Local>, style: Style) -> String {
    let mut line = format!(
        "{} Running on {} via {}",
        style.paint("▶", Tone::Info),
        style.paint(name, Tone::Info),
        style.paint(path, Tone::Info)
    );

    let Some(local) = local else {
        return line;
    };
    line.push_str(&format!(" · {}", location(local)));
    if let Some(summary) = split_summary(local) {
        line.push_str(&format!(" · {summary}"));
    }
    line
}

pub fn location(local: &Local) -> String {
    match local.cwd.is_empty() {
        true => local.name.clone(),
        false => format!("{}/{}", local.name, local.cwd),
    }
}

fn split_summary(local: &Local) -> Option<String> {
    artifacts::summary(&local.stacks)
}

#[cfg(test)]
mod tests {
    use super::*;
    use slingshot_core::stack::Stack;
    use std::path::PathBuf;

    fn local(cwd: &str, stacks: Vec<Stack>) -> Local {
        Local {
            root: PathBuf::from("/work/app"),
            name: "app".to_string(),
            cwd: cwd.to_string(),
            stacks,
        }
    }

    #[test]
    fn outside_a_project_it_only_says_where() {
        assert_eq!(
            announcement("archbox", "local network", None, Style::new(false)),
            "▶ Running on archbox via local network"
        );
    }

    #[test]
    fn inside_a_project_it_names_the_folder_and_what_moved() {
        assert_eq!(
            announcement(
                "archbox",
                "tailnet",
                Some(&local("crates/cli", vec![Stack::Rust])),
                Style::new(false)
            ),
            "▶ Running on archbox via tailnet · app/crates/cli · target → Agent disk"
        );
    }

    #[test]
    fn the_last_line_gives_the_time_and_exit_code() {
        let plain = Style::new(false);
        assert_eq!(
            finished(0, Duration::from_millis(3_500), plain),
            "✓ Done in 3.5s · exit 0"
        );
        assert_eq!(
            finished(101, Duration::from_millis(1_200), plain),
            "✗ Failed in 1.2s · exit 101"
        );
        assert_eq!(
            finished(130, Duration::from_millis(4_500), plain),
            "! Stopped in 4.5s · exit 130"
        );
    }

    #[test]
    fn a_lost_connection_names_the_agent_and_where_to_look() {
        let message = lost_connection("archbox");
        assert!(
            message.starts_with("Lost connection to archbox."),
            "{message}"
        );
        assert!(message.contains("slingshot ps --all"), "{message}");
    }

    #[test]
    fn a_project_with_no_known_stack_still_reports_its_folder() {
        assert_eq!(
            announcement(
                "archbox",
                "local network",
                Some(&local("", Vec::new())),
                Style::new(false)
            ),
            "▶ Running on archbox via local network · app"
        );
    }
}
