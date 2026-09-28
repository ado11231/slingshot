//! `slingshot env`: environment files kept on the Agent, outside source copies.

use crate::client::{fetch, unexpected};
use crate::project::{self, Local};
use anyhow::Context;
use slingshot_core::config::{Agent, Config};
use slingshot_core::control::{MAX_ENVIRONMENT_FILE, Request, Response};
use slingshot_core::presentation::{self, Style, Tone};
use slingshot_core::source;
use std::path::PathBuf;

/// The Agent, the project folder, and the project's ID there. Every env subcommand needs
/// all three before it can say anything useful.
fn resolve(config: &Config, agent: Option<String>) -> anyhow::Result<(&Agent, Local, String)> {
    let target = config.resolve(agent.as_deref())?;
    let local = project::require(None)?;
    let id = project::identify(&project::client_root()?, &local.root, &target.name)?;
    Ok((target, local, id))
}

/// Contents travel inside the control message over SSH input, never as command
/// arguments, and are never printed.
pub async fn add(
    agent: Option<String>,
    file: PathBuf,
    target: String,
    replace: bool,
) -> anyhow::Result<i32> {
    source::environment_target(&target)?;
    let meta =
        std::fs::metadata(&file).with_context(|| format!("Could not read {}", file.display()))?;
    anyhow::ensure!(meta.is_file(), "{} is not a file", file.display());
    anyhow::ensure!(
        meta.len() <= MAX_ENVIRONMENT_FILE as u64,
        "Environment files are limited to 1 MiB"
    );
    let contents =
        std::fs::read(&file).with_context(|| format!("Could not read {}", file.display()))?;

    let config = Config::load()?;
    let (remote, local, id) = resolve(&config, agent)?;
    fetch(
        remote,
        Request::EnvAdd {
            project: id,
            target: target.clone(),
            contents,
            replace,
        },
    )
    .await?;
    presentation::success(format!(
        "Set {target} for {} on {}",
        local.name, remote.name
    ));
    Ok(0)
}

pub async fn list(agent: Option<String>) -> anyhow::Result<i32> {
    let config = Config::load()?;
    let (remote, local, id) = resolve(&config, agent)?;
    let Response::EnvironmentFiles(names) = fetch(remote, Request::EnvList { project: id }).await?
    else {
        return Err(unexpected());
    };
    let style = Style::stdout();
    println!(
        "\n{}\n",
        style.heading(format!(
            "Environment files for {} on {}",
            local.name, remote.name
        ))
    );
    if names.is_empty() {
        println!(
            "  None set. Add one with {}",
            style.paint("slingshot env add --file <file> --target .env", Tone::Info)
        );
    }
    for name in names {
        println!("  {name}");
    }
    Ok(0)
}

pub async fn remove(agent: Option<String>, target: String) -> anyhow::Result<i32> {
    source::environment_target(&target)?;
    let config = Config::load()?;
    let (remote, local, id) = resolve(&config, agent)?;
    fetch(
        remote,
        Request::EnvRemove {
            project: id,
            target: target.clone(),
        },
    )
    .await?;
    presentation::success(format!(
        "Removed {target} for {} on {}",
        local.name, remote.name
    ));
    Ok(0)
}
