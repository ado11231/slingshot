//! `slingshot internal-watch`: keeps one control connection to the Agent and prints what the
//! menu bar app shows and notifies, one JSON line at a time. The app starts it and closes
//! its input to stop it, so it never outlives the app. A `retry` line on its input skips the
//! wait before the next attempt, for the app's Try again button. When Slingshot is updated,
//! the helper exits cleanly and the app starts the new program in its place.

pub mod event;
pub mod state;

use crate::client::{Control, unexpected};
use crate::route::{self, Route};
use event::{Event, Line, Status, VERSION};
use slingshot_core::config::{Agent, Config};
use slingshot_core::control::{Request, Response};
use state::Watch;
use std::convert::Infallible;
use std::io::{BufRead, Write};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;

/// How often the popover's numbers move.
const HEALTH_EVERY: Duration = Duration::from_secs(2);

/// Jobs are checked on every second health tick, which is soon enough for a notification.
const JOBS_EVERY_TICKS: u32 = 2;

const TICK_LIMIT: Duration = Duration::from_secs(15);

/// About once a minute, look for a better path than the one in use, such as the local
/// network again after a hotspot, and move to it without showing the Agent as offline.
const BETTER_ROUTE_TICKS: u32 = 30;

/// How often the helper checks whether its own program file was replaced. The check has
/// its own thread, so a busy or stalled connection attempt can never delay it.
const UPDATE_EVERY: Duration = Duration::from_secs(5);

/// Waits between attempts while the Agent is unreachable, longest last.
const BACKOFF: [Duration; 4] = [
    Duration::from_secs(2),
    Duration::from_secs(5),
    Duration::from_secs(15),
    Duration::from_secs(30),
];

pub async fn run(agent: Option<String>) -> anyhow::Result<i32> {
    let retry = Arc::new(Notify::new());
    let asked = retry.clone();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            match line {
                Ok(line) if line.trim() == "retry" => asked.notify_one(),
                Ok(_) => {}
                Err(_) => break,
            }
        }
        std::process::exit(0);
    });
    if let Some(program) = Program::current() {
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(UPDATE_EVERY);
                if program.replaced() {
                    std::process::exit(0);
                }
            }
        });
    }

    let mut watch: Option<Watch> = None;
    let mut unconfigured = 0;
    loop {
        let Err(error) = session(agent.as_deref(), &mut watch).await;
        let message = format!("{error:#}");
        let failures = match &mut watch {
            Some(watch) => {
                emit(&Event::Status(Status::offline(
                    watch.name(),
                    message.clone(),
                )));
                for notice in watch.failed(&message) {
                    emit(&Event::Notice(notice));
                }
                watch.failures()
            }
            None => {
                emit(&Event::Status(Status::offline("", message)));
                unconfigured += 1;
                unconfigured
            }
        };
        route::forget();
        let wait = BACKOFF[(failures as usize).saturating_sub(1).min(BACKOFF.len() - 1)];
        tokio::select! {
            () = tokio::time::sleep(wait) => {}
            () = retry.notified() => {}
        }
    }
}

/// One connection's worth of watching. It returns only when the Agent stops answering, and
/// the caller reconnects. When a better path answers, it moves there itself, so the panel
/// never shows the Agent as offline for it. Configuration is read again each time, so
/// linking or unlinking takes effect without restarting the app.
async fn session(agent: Option<&str>, watch: &mut Option<Watch>) -> anyhow::Result<Infallible> {
    let config = Config::load()?;
    let target = config.resolve(agent)?;
    if watch
        .as_ref()
        .is_none_or(|watch| watch.name() != target.name)
    {
        *watch = Some(Watch::new(&target.name));
    }
    let watch = watch.as_mut().expect("watch was just set");
    loop {
        let mut control = Control::connect(target).await?;
        let current = route::resolve(target);
        let path = current.name();
        let mut ticks = 0u32;
        loop {
            if ticks > 0
                && ticks.is_multiple_of(BETTER_ROUTE_TICKS)
                && better_route(target, &current).await
            {
                control.close().await;
                route::forget();
                break;
            }
            let jobs = ticks.is_multiple_of(JOBS_EVERY_TICKS);
            let answer = tokio::time::timeout(TICK_LIMIT, poll(&mut control, jobs)).await;
            let (health, jobs) = match answer {
                Ok(Ok(answer)) => answer,
                Ok(Err(error)) => {
                    control.close().await;
                    return Err(error);
                }
                Err(_) => {
                    control.close().await;
                    anyhow::bail!("{} did not answer in time", target.name);
                }
            };
            let mut notices = watch.reached(path);
            notices.extend(watch.health(&health));
            if let Some(jobs) = jobs {
                notices.extend(watch.jobs(&jobs));
            }
            emit(&Event::Status(Status::online(&target.name, path, &health)));
            for notice in notices {
                emit(&Event::Notice(notice));
            }
            ticks = ticks.wrapping_add(1);
            tokio::time::sleep(HEALTH_EVERY).await;
        }
    }
}

/// Probing waits on the network, so it runs off the async runtime.
async fn better_route(target: &Agent, current: &Route) -> bool {
    let (target, current) = (target.clone(), current.clone());
    tokio::task::spawn_blocking(move || route::better_than(&target, &current))
        .await
        .unwrap_or(false)
}

async fn poll(
    control: &mut Control,
    jobs: bool,
) -> anyhow::Result<(
    slingshot_core::protocol::Health,
    Option<Vec<slingshot_core::control::Job>>,
)> {
    let Response::Health(health) = control.call(Request::Health).await? else {
        return Err(unexpected());
    };
    if !jobs {
        return Ok((health, None));
    }
    let Response::Jobs(jobs) = control.call(Request::Jobs { all: true }).await? else {
        return Err(unexpected());
    };
    Ok((health, Some(jobs)))
}

/// A closed pipe means the app is gone, so there is nobody left to watch for.
fn emit(event: &Event) {
    let line = Line {
        version: VERSION,
        event,
    };
    let mut out = std::io::stdout().lock();
    let written = serde_json::to_writer(&mut out, &line)
        .map_err(std::io::Error::from)
        .and_then(|()| out.write_all(b"\n"))
        .and_then(|()| out.flush());
    if written.is_err() {
        std::process::exit(0);
    }
}

/// This program's file and when it was last written. An update such as `cargo install`
/// writes a new file, so a different time means a newer Slingshot is waiting.
struct Program {
    path: std::path::PathBuf,
    modified: std::time::SystemTime,
}

impl Program {
    fn current() -> Option<Program> {
        Program::at(std::env::current_exe().ok()?)
    }

    fn at(path: std::path::PathBuf) -> Option<Program> {
        let modified = std::fs::metadata(&path).ok()?.modified().ok()?;
        Some(Program { path, modified })
    }

    /// A file that is missing for a moment during an update is not yet a new program.
    fn replaced(&self) -> bool {
        Program::at(self.path.clone()).is_some_and(|now| now.modified != self.modified)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rewritten_program_file_counts_as_replaced() {
        let path = std::env::temp_dir().join(format!(
            "slingshot-watch-{}",
            slingshot_core::storage::new_id()
        ));
        std::fs::write(&path, "old").unwrap();
        let program = Program::at(path.clone()).unwrap();
        assert!(!program.replaced());

        let later = program.modified + Duration::from_secs(60);
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(later)
            .unwrap();
        assert!(program.replaced());

        std::fs::remove_file(&path).unwrap();
        assert!(!program.replaced());
    }
}
