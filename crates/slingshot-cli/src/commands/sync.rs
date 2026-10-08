//! `slingshot sync [--pull] [--check] [path]`: move eligible source between machines.

use crate::project;
use crate::transfer::{self, Direction};
use slingshot_core::config::Config;
use slingshot_core::presentation;

pub async fn sync(
    agent: Option<String>,
    path: Option<std::path::PathBuf>,
    pull: bool,
    check: bool,
) -> anyhow::Result<i32> {
    let config = Config::load()?;
    let target = config.resolve(agent.as_deref())?;
    let local = project::require(path)?;
    let direction = match pull {
        true => Direction::Pull,
        false => Direction::Push,
    };

    let mut opened = transfer::open(target, &local).await?;
    let result = match check {
        true if !opened.project.initialized => {
            presentation::progress(format!(
                "{} has no copy on {} yet. slingshot sync will copy it",
                local.name, target.name
            ));
            Ok(())
        }
        true => transfer::preview(&mut opened, target, &local, direction)
            .await
            .map(|_| ()),
        false => {
            let step = transfer::syncing(&local);
            let outcome = match direction {
                Direction::Push => transfer::push(&mut opened, target, &local, &step).await,
                Direction::Pull => transfer::pull(&mut opened, target, &local, &step).await,
            };
            outcome.map(|outcome| {
                transfer::finish(step, &outcome, direction, &target.name);
                report_kept(outcome.kept, &target.name, direction);
            })
        }
    };
    opened.control.close().await;
    result.map(|_| 0)
}

/// Paths changed only on the receiving side are left alone, and worth a look.
pub fn report_kept(kept: usize, agent: &str, direction: Direction) {
    if kept == 0 {
        return;
    }
    let (place, hint) = match direction {
        Direction::Push => (agent, "slingshot sync --pull --check"),
        Direction::Pull => ("this machine", "slingshot sync --check"),
    };
    presentation::warning(format!(
        "Kept {} changed only on {place}. Review with {hint}",
        presentation::plural(kept, "path")
    ));
}
