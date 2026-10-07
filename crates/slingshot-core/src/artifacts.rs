//! Where generated build output lives on the Agent.
//! Source copies stay small because build output is redirected to separate storage.

use crate::stack::{Project, Stack};
use std::path::{Path, PathBuf};

/// A project's source copy and the separate folder its build output is sent to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub source: PathBuf,
    pub artifacts: PathBuf,
}

/// One instruction for keeping build output out of the source copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rule {
    /// Set an environment variable and the tool writes elsewhere by itself. The
    /// tidiest kind, because nothing appears in the project at all.
    Env { key: &'static str, value: PathBuf },

    /// Link a required project directory to Agent storage.
    Redirect { name: &'static str, target: PathBuf },
}

/// Combine split rules for every detected stack. A Python `.venv` is the one exception and
/// stays in the source copy: `python3 -m venv` refuses a link, and the scan skips `.venv`,
/// so it is still never hashed, backed up, or copied.
pub fn rules(project: &Project, layout: &Layout) -> Vec<Rule> {
    let mut rules = Vec::new();

    for stack in &project.stacks {
        match stack {
            Stack::Rust => rules.push(Rule::Env {
                key: "CARGO_TARGET_DIR",
                value: layout.artifacts.join("target"),
            }),

            Stack::Node => rules.push(Rule::Redirect {
                name: "node_modules",
                target: layout.artifacts.join("node_modules"),
            }),

            Stack::Python => rules.push(Rule::Env {
                key: "PIP_CACHE_DIR",
                value: layout.artifacts.join("pip-cache"),
            }),
        }
    }

    rules
}

/// The environment variables to put in front of the remote command.
pub fn variables(rules: &[Rule]) -> Vec<(String, String)> {
    rules
        .iter()
        .filter_map(|rule| match rule {
            Rule::Env { key, value } => Some((key.to_string(), value.display().to_string())),
            Rule::Redirect { .. } => None,
        })
        .collect()
}

/// The directories the Agent has to create and link before the command runs.
pub fn redirects(rules: &[Rule]) -> Vec<(&'static str, &Path)> {
    rules
        .iter()
        .filter_map(|rule| match rule {
            Rule::Redirect { name, target } => Some((*name, target.as_path())),
            Rule::Env { .. } => None,
        })
        .collect()
}

/// Describe which generated files will use Agent storage. Takes the stacks rather than
/// the rules, because the answer does not depend on where the storage actually is, and a
/// caller that only wants this line should not have to invent a `Layout` to get it.
pub fn summary(stacks: &[Stack]) -> Option<String> {
    let names: Vec<&str> = stacks
        .iter()
        .flat_map(|stack| match stack {
            Stack::Rust => ["target"].as_slice(),
            Stack::Node => ["node_modules"].as_slice(),
            Stack::Python => ["pip cache", ".venv"].as_slice(),
        })
        .copied()
        .collect();

    match names.is_empty() {
        true => None,
        false => Some(format!("{} → Agent disk", names.join(", "))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(stacks: Vec<Stack>) -> Project {
        Project {
            root: PathBuf::from("/data/projects/1/source"),
            stacks,
        }
    }

    fn layout() -> Layout {
        Layout {
            source: PathBuf::from("/data/projects/1/source"),
            artifacts: PathBuf::from("/data/projects/1/artifacts"),
        }
    }

    #[test]
    fn rust_redirects_the_target_directory() {
        let rules = rules(&project(vec![Stack::Rust]), &layout());

        assert_eq!(
            variables(&rules),
            vec![(
                "CARGO_TARGET_DIR".to_string(),
                "/data/projects/1/artifacts/target".to_string()
            )]
        );
        assert!(redirects(&rules).is_empty());
    }

    /// npm has no environment variable for this, so a link is the only way.
    #[test]
    fn node_needs_a_link_because_npm_has_no_setting() {
        let rules = rules(&project(vec![Stack::Node]), &layout());

        assert!(variables(&rules).is_empty());
        assert_eq!(
            redirects(&rules),
            vec![(
                "node_modules",
                Path::new("/data/projects/1/artifacts/node_modules")
            )]
        );
    }

    /// `python3 -m venv .venv` fails when `.venv` is a link, so only the pip cache moves.
    #[test]
    fn python_moves_the_pip_cache_but_never_links_the_venv() {
        let rules = rules(&project(vec![Stack::Python]), &layout());

        assert_eq!(
            variables(&rules),
            vec![(
                "PIP_CACHE_DIR".to_string(),
                "/data/projects/1/artifacts/pip-cache".to_string()
            )]
        );
        assert!(redirects(&rules).is_empty());
    }

    /// The rule that keeps source copies small. Build output must never land inside
    /// the source folder, where it would be scanned, hashed, and backed up.
    #[test]
    fn no_rule_ever_points_inside_the_source_copy() {
        let layout = layout();
        let rules = rules(
            &project(vec![Stack::Rust, Stack::Node, Stack::Python]),
            &layout,
        );

        for (key, value) in variables(&rules) {
            assert!(
                !Path::new(&value).starts_with(&layout.source),
                "{key} points at {value}, which is inside the source copy"
            );
        }

        for (name, target) in redirects(&rules) {
            assert!(
                !target.starts_with(&layout.source),
                "{name} points at {}, which is inside the source copy",
                target.display()
            );
        }
    }

    #[test]
    fn a_project_with_no_known_stack_gets_no_rules() {
        let rules = rules(&project(Vec::new()), &layout());

        assert!(rules.is_empty());
        assert_eq!(summary(&[]), None);
    }

    #[test]
    fn the_summary_names_what_was_moved() {
        assert_eq!(
            summary(&[Stack::Rust, Stack::Node]),
            Some("target, node_modules → Agent disk".to_string())
        );
    }
}
