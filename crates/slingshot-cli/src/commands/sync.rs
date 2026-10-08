//! `slingshot sync [--pull] [--check] [path]`: move eligible source between machines.

use crate::project::{self, Local};
use crate::transfer::{self, Direction, ProjectSession};
use slingshot_core::config::{Agent, Config};
use slingshot_core::presentation;

pub async fn sync(
    agent: Option<String>,
    path: Option<std::path::PathBuf>,
    pull: bool,
    check: bool,
    overwrite: bool,
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
        false => match chosen(&mut opened, target, &local, direction, overwrite).await {
            Ok(Some(chosen)) => {
                let step = transfer::syncing(&local);
                let outcome = match direction {
                    Direction::Push => {
                        transfer::push(&mut opened, target, &local, &step, &chosen).await
                    }
                    Direction::Pull => {
                        transfer::pull(&mut opened, target, &local, &step, &chosen).await
                    }
                };
                outcome.map(|outcome| {
                    transfer::finish(step, &outcome, direction, &target.name);
                    report_kept(outcome.kept, &target.name, direction);
                })
            }
            Ok(None) => {
                presentation::success("Nothing was changed");
                Ok(())
            }
            Err(error) => Err(error),
        },
    };
    opened.control.close().await;
    result.map(|_| 0)
}

/// The conflicts the owner agreed to settle with the sending side's version, or `None` when
/// they declined. Without `--overwrite` nothing is chosen, so any conflict stops the sync.
async fn chosen(
    opened: &mut ProjectSession,
    target: &Agent,
    local: &Local,
    direction: Direction,
    overwrite: bool,
) -> anyhow::Result<Option<Vec<String>>> {
    if !overwrite {
        return Ok(Some(Vec::new()));
    }
    let conflicts = transfer::conflicts(opened, local, direction).await?;
    if conflicts.is_empty() {
        return Ok(Some(conflicts));
    }
    presentation::warning(overwrite_notice(&conflicts, &target.name, direction));
    anyhow::ensure!(
        crate::ssh::wants_terminal(),
        "Run it in a terminal to confirm, since it replaces files"
    );
    let them = match conflicts.len() {
        1 => "it",
        _ => "them",
    };
    match presentation::confirm(format!("Replace {them}?")).await? {
        true => Ok(Some(conflicts)),
        false => Ok(None),
    }
}

/// Which files get replaced, on which machine, by whose version, and that a backup is kept.
fn overwrite_notice(conflicts: &[String], agent: &str, direction: Direction) -> String {
    let (place, version) = match direction {
        Direction::Push => (agent.to_string(), "this machine's".to_string()),
        Direction::Pull => ("this machine".to_string(), format!("{agent}'s")),
    };
    let files = presentation::plural(conflicts.len(), "file");
    let it = match conflicts.len() {
        1 => "it",
        _ => "them",
    };
    let mut lines = vec![format!(
        "This replaces {files} on {place} with {version} version:"
    )];
    lines.extend(
        transfer::named(conflicts)
            .into_iter()
            .map(|name| format!("  {name}")),
    );
    let keeper = match direction {
        Direction::Push => agent.to_string(),
        Direction::Pull => "This machine".to_string(),
    };
    lines.push(format!("  {keeper} keeps a backup of {it}."));
    lines.join("\n")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overwriting_says_what_is_replaced_where_and_that_a_backup_is_kept() {
        let one = ["NOTES.md".to_string()];
        assert_eq!(
            overwrite_notice(&one, "archbox", Direction::Push),
            "This replaces 1 file on archbox with this machine's version:\n  NOTES.md\n  archbox keeps a backup of it."
        );
        let two = ["a.rs".to_string(), "b.rs".to_string()];
        assert_eq!(
            overwrite_notice(&two, "archbox", Direction::Pull),
            "This replaces 2 files on this machine with archbox's version:\n  a.rs\n  b.rs\n  This machine keeps a backup of them."
        );
    }
}
