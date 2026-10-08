//! `slingshot unlink`: clean up this Client's Slingshot data on an Agent, then forget it.

use crate::client::Refused;
use crate::project;
use crate::ssh::RemoteCommand;
use slingshot_core::config::Config;
use slingshot_core::control::{Request, Response};
use slingshot_core::keys::marker;
use slingshot_core::presentation;
use slingshot_core::step;

/// Remote cleanup comes first and stops the unlink when the Agent refuses, for example
/// while this Client's projects still have a run, session, or sync in progress.
/// Cleanup is scoped to projects this Client registered, so other Clients keep theirs.
pub async fn unlink(agent: Option<String>) -> anyhow::Result<i32> {
    let mut config = Config::load()?;
    let target = config.resolve(agent.as_deref())?.clone();
    let client = target.client_name();
    let tag = marker(&client);
    let client_root = project::client_root()?;
    let registered = project::for_agent(&client_root, &target.name)?;
    let ids: Vec<String> = registered.iter().map(|p| p.id.clone()).collect();

    let cleaning = step::start(format!("Cleaning up on {}", target.name));
    let mut complete = true;
    match crate::client::request(
        &target,
        Request::Unlink {
            client,
            projects: ids,
        },
    )
    .await
    {
        Ok(Response::Unlinked { environment_files }) => {
            cleaning.done(removed_line(environment_files, &target.name))
        }
        Ok(_) => return Err(crate::client::unexpected()),
        Err(error) if error.downcast_ref::<Refused>().is_some() => {
            anyhow::bail!("{error:#}. Nothing was unlinked")
        }
        Err(error) => {
            complete = false;
            cleaning.clear();
            presentation::warning(format!(
                "{error:#}. Remote cleanup is incomplete: environment files for this machine's projects remain in Slingshot storage on {}",
                target.name
            ));
        }
    }

    let script = removal_script(&tag);
    let mut remote = RemoteCommand::to(&target, "sh".to_string(), vec!["-c".to_string(), script]);
    remote.tty = false;
    let removing = step::start(format!("Removing this machine's key from {}", target.name));
    match remote.interactive().await {
        Ok(0) => removing.done(format!("Removed this machine's key from {}", target.name)),
        Ok(_) | Err(_) => {
            complete = false;
            removing.clear();
            presentation::warning(format!(
                "Could not reach {}, so the key is still there. Remove the line ending {tag} from its ~/.ssh/authorized_keys by hand",
                target.name
            ))
        }
    }

    slingshot_core::keys::forget_host(&target.host, target.port)?;

    config.remove(&target.name)?;
    let saved = config.save()?;
    if complete {
        project::forget_agent(&client_root, &target.name)?;
    }

    presentation::success(format!("Forgot {}", target.name));
    presentation::detail(
        "Kept",
        format!("source copies and backups on {}", target.name),
    );
    presentation::detail(
        "Config",
        format!(
            "{}, without {}",
            presentation::home_path(&saved),
            target.name
        ),
    );
    if !complete {
        presentation::warning(
            "Remote cleanup did not finish. Pair again and unlink once the Agent is reachable to finish it",
        );
    }

    Ok(0)
}

fn removed_line(environment_files: usize, agent: &str) -> String {
    match environment_files {
        0 => format!("No environment files to remove on {agent}"),
        count => format!(
            "Removed {} on {agent}",
            presentation::plural(count, "environment file")
        ),
    }
}

/// A small shell script that rewrites authorized_keys without our line. It writes
/// through the original file rather than replacing it, so the file keeps the
/// permissions ssh insists on.
fn removal_script(marker: &str) -> String {
    let pattern = shell_words::quote(marker);

    format!(
        "f=$HOME/.ssh/authorized_keys; \
         [ -f \"$f\" ] || exit 0; \
         t=$(mktemp) && grep -F -v -e {pattern} \"$f\" > \"$t\"; \
         cat \"$t\" > \"$f\" && rm -f \"$t\""
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_to_remove_is_said_plainly() {
        assert_eq!(
            removed_line(0, "archbox"),
            "No environment files to remove on archbox"
        );
        assert_eq!(
            removed_line(2, "archbox"),
            "Removed 2 environment files on archbox"
        );
    }
}
