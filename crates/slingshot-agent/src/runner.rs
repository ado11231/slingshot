//! Foreground runs started over SSH by `slingshot run`.
//!
//! The command runs in its own process group so Slingshot can stop all of it. When SSH
//! provides a terminal, that group becomes the terminal's foreground group, so Ctrl C,
//! resizing, and interactive input reach the command exactly as they would locally.

use crate::{jobs, projects, service};
use anyhow::{Context, bail};
use slingshot_core::control::{JobKind, JobState};
use slingshot_core::presentation::{Style, Tone};
use slingshot_core::storage;
use slingshot_core::tools;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::signal::unix::{SignalKind, signal};

pub async fn run(
    project: Option<String>,
    cwd: Option<String>,
    command: Vec<String>,
) -> anyhow::Result<i32> {
    let Some((program, args)) = command.split_first() else {
        bail!("No command given");
    };
    let root = service::root()?;
    storage::private_dir(&root)?;

    let mut lock = None;
    let mut env = Vec::new();
    let mut named = None;
    let directory = match &project {
        Some(id) => {
            let (paths, metadata) = projects::load(&root, id)?;
            lock = Some(projects::acquire(
                &root,
                &paths,
                &metadata,
                projects::Blocking::Runs,
            )?);
            projects::ensure_ready(&paths, &metadata)?;
            env = projects::prepare_artifacts(&paths)?;
            named = Some((metadata.id.clone(), metadata.name.clone()));
            let home = directories::BaseDirs::new().map(|dirs| dirs.home_dir().to_path_buf());
            let directory = start_folder(
                home.as_deref(),
                &metadata.name,
                &metadata.id,
                &paths.source,
                cwd.as_deref(),
            )?;
            if !directory.is_dir() {
                bail!(
                    "{} does not exist in the Agent copy. Run slingshot sync if it was just created",
                    cwd.unwrap_or_default()
                );
            }
            directory
        }
        None => directories::BaseDirs::new()
            .map(|dirs| dirs.home_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from("/")),
    };

    let mut job = jobs::new_job(
        JobKind::Run,
        named
            .as_ref()
            .map(|(id, name)| (id.as_str(), name.as_str())),
        shell_words::join(&command),
    );
    jobs::save(&root, &job)?;

    let terminal = unsafe { libc::isatty(0) } == 1;
    let mut child = tokio::process::Command::new(program);
    child
        .args(args)
        .current_dir(&directory)
        .env("PWD", &directory)
        .envs(env);
    if let Some(dirs) = directories::BaseDirs::new() {
        let path = std::env::var("PATH").unwrap_or_default();
        child.env("PATH", tools::with_user_folders(&path, dirs.home_dir()));
    }
    unsafe {
        child.pre_exec(move || {
            if libc::setpgid(0, 0) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            if terminal {
                libc::signal(libc::SIGTTOU, libc::SIG_IGN);
                libc::tcsetpgrp(0, libc::getpid());
                libc::signal(libc::SIGTTOU, libc::SIG_DFL);
            }
            Ok(())
        });
    }
    let mut child = match child.spawn() {
        Ok(child) => child,
        Err(error) => {
            let (code, message) = not_started(program, &error);
            let _ = jobs::update(&root, &job.id, |job| {
                job.state = JobState::Failed;
                job.ended = Some(storage::now());
                job.exit_code = Some(code);
            });
            eprintln!("{}", Style::stderr().status(message, Tone::Error));
            return Ok(code);
        }
    };

    let pid = child
        .id()
        .context("The command exited before it could be tracked")?;
    let start = jobs::process_start(pid);
    let tracked = jobs::update(&root, &job.id, |record| {
        record.pid = Some(pid);
        record.process_start = start;
    });
    if let Err(error) = tracked {
        unsafe {
            libc::kill(-(pid as i32), libc::SIGKILL);
        }
        let _ = child.wait().await;
        return Err(error.context("Could not record the run, so it was stopped"));
    }
    job.pid = Some(pid);
    job.process_start = start;

    let (status, disconnected) = supervise(&mut child, pid, start, terminal).await?;
    if terminal {
        unsafe {
            libc::signal(libc::SIGTTOU, libc::SIG_IGN);
            libc::tcsetpgrp(0, libc::getpgrp());
            libc::signal(libc::SIGTTOU, libc::SIG_DFL);
        }
    }

    use std::os::unix::process::ExitStatusExt;
    let code = status
        .code()
        .or_else(|| status.signal().map(|signal| 128 + signal))
        .unwrap_or(1);
    let recorded = jobs::update(&root, &job.id, |record| {
        record.state = final_state(record.stop_requested, disconnected, code);
        record.exit_code = Some(code);
        record.ended = Some(storage::now());
    });
    if let Err(error) = recorded {
        tracing::warn!("Could not record how the run ended: {error:#}");
    }
    drop(lock);
    Ok(code)
}

/// A command that could not start exits the way a shell reports it: 127 when it does not
/// exist, 126 when it cannot be run, so scripts see the same codes as locally.
/// Where a run in a project starts: the same `~/Slingshot/<project>` link a session uses,
/// so tools that name things after the folder, such as Docker Compose, see the project
/// rather than the copy's `source` folder. The copy itself is the fallback.
fn start_folder(
    home: Option<&Path>,
    name: &str,
    id: &str,
    source: &Path,
    cwd: Option<&str>,
) -> anyhow::Result<PathBuf> {
    let base = home
        .and_then(|home| jobs::project_link(home, name, id, source).ok())
        .unwrap_or_else(|| source.to_path_buf());
    Ok(match cwd {
        Some(rel) if !rel.is_empty() => base.join(storage::relative(rel)?),
        _ => base,
    })
}

fn not_started(program: &str, error: &std::io::Error) -> (i32, String) {
    match error.kind() {
        std::io::ErrorKind::NotFound => (
            127,
            format!("{program} was not found on the Agent. Check the name, or install it there"),
        ),
        std::io::ErrorKind::PermissionDenied => (
            126,
            format!("{program} is not allowed to run on the Agent. Check that it is executable"),
        ),
        _ => (
            126,
            format!("Could not start {program} on the Agent: {error}"),
        ),
    }
}

/// How a finished run is recorded. Exit code 130 means the command ended on Ctrl C,
/// which is the user's choice rather than a failure of the command.
fn final_state(stop_requested: bool, disconnected: bool, code: i32) -> JobState {
    match (stop_requested, disconnected, code) {
        (true, _, _) => JobState::Stopped,
        (false, true, _) => JobState::Interrupted,
        (false, false, 0) => JobState::Completed,
        (false, false, EXIT_INTERRUPTED) => JobState::Interrupted,
        (false, false, _) => JobState::Failed,
    }
}

/// 128 plus SIGINT, the shell convention for a command ended by Ctrl C.
const EXIT_INTERRUPTED: i32 = 128 + libc::SIGINT;

/// Wait for the command, forwarding hangups and termination. A lost SSH connection is
/// treated as a hangup. It is noticed when the output pipe closes or when the process
/// that started this helper goes away, because not every system reports the pipe.
/// Returns the exit status and whether the connection was lost.
async fn supervise(
    child: &mut tokio::process::Child,
    pid: u32,
    start: Option<u64>,
    terminal: bool,
) -> anyhow::Result<(std::process::ExitStatus, bool)> {
    let mut hangup = signal(SignalKind::hangup())?;
    let mut terminate = signal(SignalKind::terminate())?;
    let mut interrupt = signal(SignalKind::interrupt())?;
    let parent = unsafe { libc::getppid() };
    let mut ticker = tokio::time::interval(Duration::from_secs(1));
    let (signal_to_send, disconnected) = loop {
        tokio::select! {
            status = child.wait() => return Ok((status?, false)),
            _ = hangup.recv() => break (libc::SIGHUP, true),
            _ = terminate.recv() => break (libc::SIGTERM, false),
            _ = interrupt.recv() => break (libc::SIGINT, false),
            _ = ticker.tick() => {
                if unsafe { libc::getppid() } != parent || (!terminal && output_closed()) {
                    break (libc::SIGHUP, true);
                }
            }
        }
    };
    if let Some(start) = start {
        jobs::signal_group(pid, start, signal_to_send);
    }
    match tokio::time::timeout(jobs::GRACE, child.wait()).await {
        Ok(status) => Ok((status?, disconnected)),
        Err(_) => {
            if let Some(start) = start {
                jobs::signal_group(pid, start, libc::SIGKILL);
            }
            Ok((child.wait().await?, disconnected))
        }
    }
}

fn output_closed() -> bool {
    let mut poll = libc::pollfd {
        fd: 1,
        events: 0,
        revents: 0,
    };
    let ready = unsafe { libc::poll(&mut poll, 1, 0) };
    ready > 0 && poll.revents & (libc::POLLERR | libc::POLLHUP) != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ctrl_c_is_an_interruption_and_other_codes_are_failures() {
        assert_eq!(final_state(false, false, 0), JobState::Completed);
        assert_eq!(final_state(false, false, 130), JobState::Interrupted);
        assert_eq!(final_state(false, false, 101), JobState::Failed);
        assert_eq!(final_state(false, true, 0), JobState::Interrupted);
        assert_eq!(final_state(true, false, 130), JobState::Stopped);
    }

    #[test]
    fn runs_start_in_the_project_folder_named_after_the_project() {
        let home = crate::testing::Root::new();
        let source = home.0.join("copy").join("source");
        std::fs::create_dir_all(source.join("api")).unwrap();
        let start = |cwd| start_folder(Some(&home.0), "web app", "abcdef0123456789", &source, cwd);

        let folder = home.0.join("Slingshot").join("web_app");
        assert_eq!(start(None).unwrap(), folder);
        assert_eq!(start(Some("api")).unwrap(), folder.join("api"));
        assert!(start(Some("../outside")).is_err());
        assert_eq!(
            start_folder(None, "web app", "abcdef0123456789", &source, None).unwrap(),
            source
        );
    }

    #[test]
    fn commands_that_cannot_start_exit_like_a_shell() {
        let missing = std::io::Error::from(std::io::ErrorKind::NotFound);
        let (code, message) = not_started("carg", &missing);
        assert_eq!(code, 127);
        assert!(message.starts_with("carg was not found on the Agent"));
        let denied = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        assert_eq!(not_started("./build.sh", &denied).0, 126);
    }
}
