//! The local project a command applies to, and its persistent identity per Agent.

use anyhow::Context;
use serde::{Deserialize, Serialize};
use slingshot_core::{stack, storage};
use std::path::{Path, PathBuf};

/// A project on the Client and where the command was typed inside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Local {
    pub root: PathBuf,
    pub name: String,
    /// The working directory relative to the root, empty at the root itself.
    pub cwd: String,
    pub stacks: Vec<stack::Stack>,
}

/// Find the project containing `start`. The enclosing Git repository wins, so a
/// workspace member still copies the whole workspace it builds with. Without Git, the
/// nearest project marker is used. A home directory or filesystem root never counts,
/// because copying either would be a mistake.
pub fn locate(start: &Path) -> Option<Local> {
    let start = start.canonicalize().ok()?;
    let home = directories::BaseDirs::new().map(|dirs| dirs.home_dir().to_path_buf());
    let root = start
        .ancestors()
        .find(|dir| dir.join(".git").exists())
        .map(Path::to_path_buf)
        .or_else(|| stack::find(&start).map(|project| project.root))?;
    if root.parent().is_none() || home.as_deref().is_some_and(|home| home == root) {
        return None;
    }
    let cwd = start.strip_prefix(&root).ok()?.to_str()?.to_string();
    Some(Local {
        name: root.file_name()?.to_string_lossy().to_string(),
        stacks: stack::stacks_in(&root),
        cwd,
        root,
    })
}

pub fn require(path: Option<PathBuf>) -> anyhow::Result<Local> {
    let start = match path {
        Some(path) => path,
        None => std::env::current_dir()?,
    };
    locate(&start).with_context(|| {
        format!(
            "No project found at {}. Use a folder inside a Git repository or one with Cargo.toml, package.json, pyproject.toml, requirements.txt, or slingshot.toml",
            start.display()
        )
    })
}

/// Client storage, separate from Agent storage on the same machine.
pub fn client_root() -> anyhow::Result<PathBuf> {
    Ok(storage::data_dir()?.join("client"))
}

#[derive(Default, Serialize, Deserialize)]
struct Registry {
    projects: Vec<Registered>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Registered {
    pub id: String,
    pub agent: String,
    pub root: PathBuf,
}

/// The persistent random ID for this project on this Agent, created on first use.
/// Same named folders and different Agents always get different IDs.
pub fn identify(dir: &Path, root: &Path, agent: &str) -> anyhow::Result<String> {
    storage::private_dir(dir)?;
    let _guard = storage::lock(&dir.join("projects.lock"))?;
    let file = dir.join("projects.json");
    let mut registry: Registry = storage::read_json(&file)?;
    if let Some(found) = registry
        .projects
        .iter()
        .find(|p| p.agent == agent && p.root == root)
    {
        return Ok(found.id.clone());
    }
    let id = storage::new_id();
    registry.projects.push(Registered {
        id: id.clone(),
        agent: agent.to_string(),
        root: root.to_path_buf(),
    });
    storage::write_json(&file, &registry)?;
    Ok(id)
}

pub fn for_agent(dir: &Path, agent: &str) -> anyhow::Result<Vec<Registered>> {
    let registry: Registry = storage::read_json(&dir.join("projects.json"))?;
    Ok(registry
        .projects
        .into_iter()
        .filter(|p| p.agent == agent)
        .collect())
}

pub fn forget_agent(dir: &Path, agent: &str) -> anyhow::Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    let _guard = storage::lock(&dir.join("projects.lock"))?;
    let file = dir.join("projects.json");
    let mut registry: Registry = storage::read_json(&file)?;
    registry.projects.retain(|p| p.agent != agent);
    storage::write_json(&file, &registry)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Temp(PathBuf);

    impl Temp {
        fn new() -> Temp {
            let path = std::env::temp_dir().join(format!("slingshot-cli-{}", storage::new_id()));
            std::fs::create_dir_all(&path).unwrap();
            Temp(path.canonicalize().unwrap())
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn identities_are_stable_per_project_and_agent() {
        let temp = Temp::new();
        let state = temp.0.join("client");
        let a = temp.0.join("one/app");
        let b = temp.0.join("two/app");
        let first = identify(&state, &a, "archbox").unwrap();
        assert_eq!(identify(&state, &a, "archbox").unwrap(), first);
        assert_ne!(identify(&state, &b, "archbox").unwrap(), first);
        assert_ne!(identify(&state, &a, "laptop").unwrap(), first);
        assert_eq!(for_agent(&state, "archbox").unwrap().len(), 2);
        forget_agent(&state, "archbox").unwrap();
        assert!(for_agent(&state, "archbox").unwrap().is_empty());
        assert_eq!(for_agent(&state, "laptop").unwrap().len(), 1);
    }

    #[test]
    fn the_git_root_is_the_project_and_the_subfolder_is_kept() {
        let temp = Temp::new();
        let repo = temp.0.join("repo");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::create_dir_all(repo.join("crates/cli/src")).unwrap();
        std::fs::write(repo.join("crates/cli/Cargo.toml"), "").unwrap();
        let found = locate(&repo.join("crates/cli/src")).unwrap();
        assert_eq!(found.root, repo);
        assert_eq!(found.cwd, "crates/cli/src");
        assert_eq!(found.name, "repo");
    }

    #[test]
    fn without_git_the_nearest_marker_is_the_project() {
        let temp = Temp::new();
        let app = temp.0.join("app");
        std::fs::create_dir_all(app.join("src")).unwrap();
        std::fs::write(app.join("package.json"), "{}").unwrap();
        let found = locate(&app.join("src")).unwrap();
        assert_eq!(found.root, app);
        assert_eq!(found.cwd, "src");
        assert_eq!(found.stacks, [stack::Stack::Node]);
        assert!(locate(&temp.0).is_none());
    }
}
