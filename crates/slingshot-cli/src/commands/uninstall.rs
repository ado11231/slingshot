//! `slingshot uninstall`: list everything Slingshot keeps on this machine, ask once, and
//! remove it. The program itself is removed afterwards with `cargo uninstall`, because a
//! running program cannot safely delete itself.

use crate::keys;
use slingshot_core::config::{self, Config};
use slingshot_core::presentation::{self, home_path};
use slingshot_core::storage;
use std::fs;
use std::path::{Path, PathBuf};

const REMOVE_PROGRAM: &str = "cargo uninstall slingshot-cli";

/// What this machine keeps as a Client.
struct Found {
    agents: Vec<String>,
    menubar: Option<PathBuf>,
    /// The Client's own settings and data. Only these, never the whole folder, because on a
    /// Mac the same folder also holds an Agent's data when the Mac is one too.
    files: Vec<PathBuf>,
    keys: Vec<PathBuf>,
}

impl Found {
    fn look() -> anyhow::Result<Found> {
        let data = storage::data_dir()?;
        Ok(Found {
            agents: Config::load_or_empty()?.names(),
            menubar: super::menubar::installed(),
            files: existing([
                config::dir()?.join("config.toml"),
                config::known_hosts_path()?,
                data.join("client"),
            ]),
            keys: existing(keys::files()?),
        })
    }

    fn is_empty(&self) -> bool {
        self.agents.is_empty()
            && self.menubar.is_none()
            && self.files.is_empty()
            && self.keys.is_empty()
    }
}

fn existing(paths: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = Vec::new();
    for path in paths {
        if path.exists() && !found.contains(&path) {
            found.push(path);
        }
    }
    found
}

pub async fn uninstall() -> anyhow::Result<i32> {
    let found = Found::look()?;
    if found.is_empty() {
        presentation::success("Slingshot keeps nothing on this machine");
        presentation::detail("Program", format!("remove it with {REMOVE_PROGRAM}"));
        return Ok(0);
    }
    eprintln!("\n  This removes Slingshot from this machine:\n");
    for line in listing(&found) {
        eprintln!("{line}");
    }
    anyhow::ensure!(
        crate::ssh::wants_terminal(),
        "Run slingshot uninstall in a terminal to confirm, since it deletes files"
    );
    if !presentation::confirm_deleting("Remove all of this?".to_string()).await? {
        presentation::success("Nothing was removed");
        return Ok(0);
    }

    for agent in &found.agents {
        super::unlink::unlink(Some(agent.clone()))
            .await
            .map_err(|error| anyhow::anyhow!("{error:#}\n  Nothing else was removed."))?;
    }
    if let Some(app) = &found.menubar {
        super::menubar::remove_app(app).await?;
        presentation::success("Removed the menu bar app");
    }
    for path in found.files.iter().chain(&found.keys) {
        remove(path)?;
        presentation::success(format!("Removed {}", home_path(path)));
    }
    for folder in [config::dir()?, storage::data_dir()?] {
        let _ = fs::remove_dir(folder);
    }
    presentation::success(format!(
        "Slingshot's files are gone. Remove the program with: {REMOVE_PROGRAM}"
    ));
    Ok(0)
}

/// One dim line per kind of thing, so the owner sees all of it before agreeing.
fn listing(found: &Found) -> Vec<String> {
    let mut lines = Vec::new();
    for (index, agent) in found.agents.iter().enumerate() {
        let label = if index == 0 { "Links" } else { "" };
        lines.push(row(
            label,
            format!("{agent}: this machine's key and environment files there"),
        ));
    }
    if let Some(app) = &found.menubar {
        lines.push(row(
            "Menu bar",
            format!("{}, its login item and settings", home_path(app)),
        ));
    }
    for (index, file) in found.files.iter().enumerate() {
        let label = if index == 0 { "Settings" } else { "" };
        lines.push(row(label, home_path(file)));
    }
    for (index, key) in found.keys.iter().enumerate() {
        let label = if index == 0 { "Key" } else { "" };
        lines.push(row(label, home_path(key)));
    }
    lines
}

fn row(label: &str, value: String) -> String {
    presentation::Style::stderr().dim(presentation::row(label, value).trim_end())
}

fn remove(path: &Path) -> anyhow::Result<()> {
    let removed = match path.is_dir() {
        true => fs::remove_dir_all(path),
        false => fs::remove_file(path),
    };
    removed.map_err(|error| {
        anyhow::anyhow!(
            "Could not remove {}: {error}. Delete it by hand",
            home_path(path)
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn everything_found_is_listed_with_a_label_once_per_kind() {
        let found = Found {
            agents: vec!["archbox".into(), "devbox".into()],
            menubar: None,
            files: vec![PathBuf::from("/x/slingshot/config.toml")],
            keys: vec![PathBuf::from("/x/k"), PathBuf::from("/x/k.pub")],
        };
        let lines: Vec<String> = listing(&found)
            .into_iter()
            .map(|line| line.trim_end().to_string())
            .collect();
        assert_eq!(
            lines,
            [
                "  Links        archbox: this machine's key and environment files there",
                "               devbox: this machine's key and environment files there",
                "  Settings     /x/slingshot/config.toml",
                "  Key          /x/k",
                "               /x/k.pub",
            ]
        );
    }

    #[test]
    fn only_paths_that_exist_are_kept_once() {
        let here = std::env::temp_dir();
        let missing = here.join("slingshot-no-such-file");
        assert_eq!(existing([here.clone(), missing, here.clone()]), [here]);
    }
}
