//! `slingshot env`: environment files kept on the Agent, outside source copies.

use crate::client::{fetch, unexpected};
use crate::project::{self, Local};
use anyhow::Context;
use slingshot_core::config::{Agent, Config};
use slingshot_core::control::{MAX_ENVIRONMENT_FILE, Request, Response};
use slingshot_core::presentation::{self, Style, Tone};
use slingshot_core::source;
use std::path::{Path, PathBuf};

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
    target: Option<String>,
    replace: bool,
) -> anyhow::Result<i32> {
    let config = Config::load()?;
    let (remote, local, id) = resolve(&config, agent)?;
    let target = match target {
        Some(target) => target,
        None => default_target(&file, &local.root)?,
    };
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

/// Where a file lands when no target is given: its own place in the project, so
/// `slingshot env add api/.env` fills `api/.env` on the Agent. A file from outside the
/// project keeps only its name.
fn default_target(file: &Path, root: &Path) -> anyhow::Result<String> {
    let full = std::fs::canonicalize(file)
        .with_context(|| format!("Could not read {}", file.display()))?;
    let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let place = match full.strip_prefix(&root) {
        Ok(inside) => inside.to_path_buf(),
        Err(_) => PathBuf::from(full.file_name().context("The file has no name")?),
    };
    place
        .to_str()
        .map(str::to_string)
        .context("The file's name is not valid text. Give one with --target")
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
            style.paint("slingshot env add .env", Tone::Info)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_without_a_target_keeps_its_place_in_the_project() {
        let project =
            std::env::temp_dir().join(format!("env-target-{}", slingshot_core::storage::new_id()));
        std::fs::create_dir_all(project.join("api")).unwrap();
        std::fs::write(project.join(".env"), "A=1").unwrap();
        std::fs::write(project.join("api/.env.local"), "B=2").unwrap();
        let outside =
            std::env::temp_dir().join(format!("prod-{}.env", slingshot_core::storage::new_id()));
        std::fs::write(&outside, "C=3").unwrap();

        assert_eq!(
            default_target(&project.join(".env"), &project).unwrap(),
            ".env"
        );
        assert_eq!(
            default_target(&project.join("api/.env.local"), &project).unwrap(),
            "api/.env.local"
        );
        assert_eq!(
            default_target(&outside, &project).unwrap(),
            outside.file_name().unwrap().to_str().unwrap()
        );
        std::fs::remove_dir_all(&project).unwrap();
        std::fs::remove_file(&outside).unwrap();
    }
}
