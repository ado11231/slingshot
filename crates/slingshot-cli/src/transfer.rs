//! Copying eligible source between the Client and an Agent.
//!
//! Both sides list their files, the three way plan decides what moves, rsync copies
//! only the changed regular files into staging over SSH, and the receiving side checks
//! every staged file before applying anything.

use crate::client::{self, Control, unexpected};
use crate::project::{self, Local};
use crate::route;
use anyhow::{Context, bail, ensure};
use slingshot_core::config::Agent;
use slingshot_core::control::{ProjectInfo, ProjectRef, Request, Response, Snapshot};
use slingshot_core::presentation::{self, Style, Tone};
use slingshot_core::source::{self, Manifest, Rules};
use slingshot_core::step::Step;
use slingshot_core::storage;
use slingshot_core::sync::{self, Plan, StateDir};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncReadExt;

/// Ping interval while rsync runs, so the Agent keeps the lease for a live Client.
const HEARTBEAT: Duration = Duration::from_secs(20);

pub(crate) struct ProjectSession {
    pub control: Control,
    pub project: ProjectInfo,
    pub id: String,
}

/// Connect to the Agent and make sure the project has storage there.
pub async fn open(agent: &Agent, local: &Local) -> anyhow::Result<ProjectSession> {
    connect(agent, local, true).await
}

/// `open` without the "Connected" line, for a connection right after another one, such as
/// bringing edits back when leaving a session.
pub async fn open_quietly(agent: &Agent, local: &Local) -> anyhow::Result<ProjectSession> {
    connect(agent, local, false).await
}

async fn connect(agent: &Agent, local: &Local, announce: bool) -> anyhow::Result<ProjectSession> {
    let id = project::identify(&project::client_root()?, &local.root, &agent.name)?;
    let connecting = client::connecting(agent);
    let mut control = Control::connect(agent).await?;
    let reference = ProjectRef {
        id: id.clone(),
        name: local.name.clone(),
        client: agent.client_name(),
    };
    let Response::Project(project) = control.call(Request::Open(reference)).await? else {
        return Err(unexpected());
    };
    match announce {
        true => client::connected(connecting, agent),
        false => connecting.clear(),
    }
    Ok(ProjectSession {
        control,
        project,
        id,
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Push,
    Pull,
}

#[derive(Default)]
pub struct SyncResult {
    pub changed: usize,
    pub kept: usize,
    /// The paths a pull brought back, to name them. Empty after a push.
    pub pulled: Vec<String>,
}

/// Finish or undo an interrupted pull on this machine before its source is read again.
async fn recover_local(local: &Local, id: &str) -> anyhow::Result<()> {
    let state = StateDir::new(state_dir(id)?);
    let root = local.root.clone();
    tokio::task::spawn_blocking(move || sync::recover(&root, &state)).await?
}

/// Preview a sync without taking a lease or changing anything.
pub async fn preview(
    opened: &mut ProjectSession,
    agent: &Agent,
    local: &Local,
    direction: Direction,
) -> anyhow::Result<Plan> {
    recover_local(local, &opened.id).await?;
    let excludes = source::client_excludes(&local.root)?;
    let snapshot = match opened
        .control
        .call(Request::Inspect {
            project: opened.id.clone(),
            excludes: excludes.clone(),
        })
        .await?
    {
        Response::Snapshot(snapshot) => snapshot,
        _ => return Err(unexpected()),
    };
    let (local_manifest, _) = scan_local(local, &opened.id, &excludes, &snapshot.baseline).await?;
    let plan = match direction {
        Direction::Push => sync::plan(&snapshot.baseline, &local_manifest, &snapshot.manifest),
        Direction::Pull => sync::plan(&snapshot.baseline, &snapshot.manifest, &local_manifest),
    };
    let (sender, receiver) = match direction {
        Direction::Push => (&local_manifest, &snapshot.manifest),
        Direction::Pull => (&snapshot.manifest, &local_manifest),
    };
    print!(
        "{}",
        preview_text(
            &plan,
            sender,
            receiver,
            &agent.name,
            direction,
            Style::stdout()
        )
    );
    Ok(plan)
}

/// Finish any interrupted sync on this machine, then take the Agent's lease. Push and pull
/// start exactly this way, and both must pair it with `release_on_error`.
async fn take_lease(
    opened: &mut ProjectSession,
    local: &Local,
    pull: bool,
) -> anyhow::Result<(Vec<String>, Snapshot, String)> {
    recover_local(local, &opened.id).await?;
    let excludes = source::client_excludes(&local.root)?;
    let snapshot = begin(opened, &excludes, pull).await?;
    let token = snapshot
        .token
        .clone()
        .context("The Agent did not open a sync")?;
    Ok((excludes, snapshot, token))
}

/// Hand the lease back when the transfer failed, so the project is free again straight
/// away instead of staying busy until the Agent times the connection out.
async fn release_on_error<T>(
    opened: &mut ProjectSession,
    token: String,
    result: &anyhow::Result<T>,
) {
    if result.is_err() {
        let _ = opened.control.call(Request::Release { token }).await;
    }
}

/// Copy Client edits to the Agent, saying on `step` what is being copied.
pub async fn push(
    opened: &mut ProjectSession,
    agent: &Agent,
    local: &Local,
    step: &Step,
) -> anyhow::Result<SyncResult> {
    let (excludes, snapshot, token) = take_lease(opened, local, false).await?;
    let result = async {
        let (local_manifest, _) =
            scan_local(local, &opened.id, &excludes, &snapshot.baseline).await?;
        let plan = sync::plan(&snapshot.baseline, &local_manifest, &snapshot.manifest);
        refuse_conflicts(&plan, &agent.name)?;
        let files = regular_files(&plan, &local_manifest);
        if !files.is_empty() {
            step.set(format!(
                "Copying {} to {}",
                presentation::plural(files.len(), "file"),
                agent.name
            ));
            let transfer = rsync(
                agent,
                &local.root,
                &snapshot.transfer_path,
                &files,
                Direction::Push,
            );
            with_heartbeat(&mut opened.control, transfer).await?;
        }
        let Response::Synced { changed } = opened
            .control
            .call(Request::Finish {
                token: token.clone(),
                manifest: local_manifest,
            })
            .await?
        else {
            return Err(unexpected());
        };
        Ok(SyncResult {
            changed,
            kept: plan.kept.len(),
            pulled: Vec::new(),
        })
    }
    .await;
    release_on_error(opened, token, &result).await;
    result
}

pub fn syncing(local: &Local) -> Step {
    slingshot_core::step::start(format!("Syncing {}", local.name))
}

/// End a sync step with what changed. A pull names what it brought back, since those are
/// edits made somewhere else, such as by a coding agent.
pub fn finish(step: Step, outcome: &SyncResult, direction: Direction, agent: &str) {
    let changes = presentation::plural(outcome.changed, "change");
    match (direction, outcome.changed) {
        (Direction::Push, 0) => step.done("Source up to date"),
        (Direction::Push, _) => step.done(format!("Synced {changes}")),
        (Direction::Pull, 0) => step.done(format!("No edits to bring back from {agent}")),
        (Direction::Pull, count) => {
            step.done(format!(
                "Brought back {} from {agent}",
                presentation::plural(count, "edit")
            ));
            let style = Style::stderr();
            for line in named(&outcome.pulled) {
                eprintln!("  {}", style.dim(line));
            }
        }
    }
}

/// How many paths a pull names before summing up the rest, so a large pull stays readable.
const NAMED: usize = 5;

fn named(paths: &[String]) -> Vec<String> {
    let mut lines: Vec<String> = paths.iter().take(NAMED).cloned().collect();
    if paths.len() > NAMED {
        lines.push(format!("and {} more", paths.len() - NAMED));
    }
    lines
}

/// Copy Agent edits back to the Client, saying on `step` what is being copied.
pub async fn pull(
    opened: &mut ProjectSession,
    agent: &Agent,
    local: &Local,
    step: &Step,
) -> anyhow::Result<SyncResult> {
    let state_dir = state_dir(&opened.id)?;
    let state = StateDir::new(&state_dir);
    let (excludes, snapshot, token) = take_lease(opened, local, true).await?;
    let stage = state_dir.join("staging").join(&token);
    let result = async {
        let (local_manifest, rules) =
            scan_local(local, &opened.id, &excludes, &snapshot.baseline).await?;
        let plan = sync::plan(&snapshot.baseline, &snapshot.manifest, &local_manifest);
        refuse_conflicts(&plan, &agent.name)?;
        for name in &plan.changes {
            storage::relative(name)
                .with_context(|| format!("{} sent an unusable path", agent.name))?;
            ensure!(
                !rules.excluded(name, |f| snapshot.manifest.contains_key(f)),
                "{} sent an excluded path: {name}",
                agent.name
            );
        }
        let applied_manifest = sync::merge(&local_manifest, &plan.changes, &snapshot.manifest);
        source::check_links(&applied_manifest, &rules)?;
        if ignores_case(&local.root) {
            refuse_case_clashes(&sync::case_clashes(&applied_manifest), &agent.name)?;
        }

        let files = regular_files(&plan, &snapshot.manifest);
        storage::private_dir(&stage)?;
        if !files.is_empty() {
            step.set(format!(
                "Copying {} from {}",
                presentation::plural(files.len(), "file"),
                agent.name
            ));
            let transfer = rsync(
                agent,
                &stage,
                &snapshot.transfer_path,
                &files,
                Direction::Pull,
            );
            with_heartbeat(&mut opened.control, transfer).await?;
        }
        if !plan.changes.is_empty() {
            let (root, stage, plan, receiver, sender, token) = (
                local.root.clone(),
                stage.clone(),
                plan.clone(),
                local_manifest.clone(),
                snapshot.manifest.clone(),
                token.clone(),
            );
            let state = StateDir::new(&state.dir);
            tokio::task::spawn_blocking(move || {
                sync::apply(&root, &stage, &state, &plan, &receiver, &sender, &token)
            })
            .await??;
        }
        opened
            .control
            .call(Request::Finish {
                token: token.clone(),
                manifest: applied_manifest,
            })
            .await?;
        Ok(SyncResult {
            changed: plan.changes.len(),
            kept: plan.kept.len(),
            pulled: plan.changes.clone(),
        })
    }
    .await;
    let _ = std::fs::remove_dir_all(state_dir.join("staging"));
    release_on_error(opened, token, &result).await;
    result
}

async fn begin(
    opened: &mut ProjectSession,
    excludes: &[String],
    pull: bool,
) -> anyhow::Result<Snapshot> {
    match opened
        .control
        .call(Request::Begin {
            project: opened.id.clone(),
            excludes: excludes.to_vec(),
            pull,
        })
        .await?
    {
        Response::Snapshot(snapshot) => Ok(snapshot),
        _ => Err(unexpected()),
    }
}

fn state_dir(id: &str) -> anyhow::Result<PathBuf> {
    storage::check_id(id)?;
    let dir = project::client_root()?.join("projects").join(id);
    storage::private_dir(&dir)?;
    Ok(dir)
}

async fn scan_local(
    local: &Local,
    id: &str,
    excludes: &[String],
    baseline: &Manifest,
) -> anyhow::Result<(Manifest, Rules)> {
    let cache = state_dir(id)?.join("hashes.json");
    let (root, excludes, baseline) = (local.root.clone(), excludes.to_vec(), baseline.clone());
    tokio::task::spawn_blocking(move || {
        let rules = Rules::new(&root, &excludes)?;
        let manifest = source::scan(&root, &rules, &baseline, Some(&cache))?;
        Ok((manifest, rules))
    })
    .await?
}

/// Paths changed differently on both machines. Its own type, so a caller such as
/// `attach` can tell a conflict apart from a failed connection.
#[derive(Debug)]
pub struct Conflicts(String);

impl std::fmt::Display for Conflicts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Conflicts {}

fn refuse_conflicts(plan: &Plan, agent: &str) -> anyhow::Result<()> {
    match plan.conflicts.is_empty() {
        true => Ok(()),
        false => Err(Conflicts(conflict_message(&plan.conflicts, agent)).into()),
    }
}

/// What changed on both sides, then the files, then how to look and what to do.
fn conflict_message(paths: &[String], agent: &str) -> String {
    let (headline, fix) = match paths {
        [only] => (only.clone(), "make the file match on both"),
        _ => (
            presentation::plural(paths.len(), "file"),
            "make each file match on both",
        ),
    };
    let mut lines = vec![format!(
        "{headline} changed on both this machine and {agent}"
    )];
    if paths.len() > 1 {
        lines.extend(named(paths).into_iter().map(|name| format!("  {name}")));
    }
    lines.push("  Nothing was changed on either machine.".to_string());
    lines.push(
        presentation::row(
            "Compare",
            "slingshot sync --check, then slingshot sync --pull --check",
        )
        .trim_end()
        .to_string(),
    );
    lines.push(
        presentation::row(
            "Fix",
            format!("{fix}, or undo one side's edit, then sync again"),
        )
        .trim_end()
        .to_string(),
    );
    lines.join("\n")
}

/// Whether this disk treats names that differ only in case as one, found by asking for the
/// project folder under its own name with one letter's case flipped.
fn ignores_case(root: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    let Some(name) = root.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let flipped: String = match name.char_indices().find(|(_, c)| c.is_ascii_alphabetic()) {
        Some((at, c)) => {
            let swapped = match c.is_ascii_lowercase() {
                true => c.to_ascii_uppercase(),
                false => c.to_ascii_lowercase(),
            };
            format!("{}{swapped}{}", &name[..at], &name[at + 1..])
        }
        None => return false,
    };
    match (
        std::fs::metadata(root),
        std::fs::metadata(root.with_file_name(flipped)),
    ) {
        (Ok(real), Ok(other)) => real.ino() == other.ino() && real.dev() == other.dev(),
        _ => false,
    }
}

/// Refused like a conflict, so `attach` still opens the session. Nothing is changed.
fn refuse_case_clashes(clashes: &[(String, String)], agent: &str) -> anyhow::Result<()> {
    let Some((first, second)) = clashes.first() else {
        return Ok(());
    };
    let mut lines = vec![format!("{first} and {second} differ only in case")];
    lines.extend(
        clashes[1..]
            .iter()
            .map(|(one, other)| format!("  Also {one} and {other}")),
    );
    lines.push("  This machine treats them as one file, so nothing was changed.".to_string());
    lines.push(
        presentation::row(
            "Fix",
            format!("rename or remove one of them on {agent}, then sync again"),
        )
        .trim_end()
        .to_string(),
    );
    Err(Conflicts(lines.join("\n")).into())
}

fn regular_files(plan: &Plan, sender: &Manifest) -> Vec<String> {
    plan.changes
        .iter()
        .filter(|name| sender.get(*name).is_some_and(|entry| entry.link.is_none()))
        .cloned()
        .collect()
}

/// Run rsync with an explicit file list. The remote side changes into the transfer
/// folder first, so no Agent path passes through rsync's own remote argument handling.
async fn rsync(
    agent: &Agent,
    local: &Path,
    remote: &str,
    files: &[String],
    direction: Direction,
) -> anyhow::Result<()> {
    ensure!(
        slingshot_core::telemetry::is_installed("rsync"),
        "rsync is not installed on this machine. Fix: {}",
        slingshot_core::preflight::install_hint("rsync")
    );
    let exe = std::env::current_exe()?;
    let exe = exe.to_str().context("Slingshot's own path is not UTF 8")?;
    ensure!(
        !exe.contains('\''),
        "Slingshot's own path contains a quote, which rsync cannot use: {exe}"
    );
    let list = FileList::write(&files.join("\n"))?;
    let local_path = format!("{}/", local.display());
    let (from, to) = match direction {
        Direction::Push => (local_path, "slingshot:.".to_string()),
        Direction::Pull => ("slingshot:./".to_string(), local_path),
    };
    let rsync_path = format!("cd {} && rsync", escape_remote(remote)?);
    let mut child = tokio::process::Command::new("rsync")
        .arg("-e")
        .arg(format!("'{exe}' internal-rsh"))
        .arg(format!("--rsync-path={rsync_path}"))
        .arg(format!("--files-from={}", list.0))
        .arg(from)
        .arg(to)
        .env("SLINGSHOT_RSH_AGENT", &agent.name)
        .env(route::ROUTE_ENV, route::resolve(agent).token())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .context("Could not start rsync")?;
    let stderr = child.stderr.take().context("Missing rsync output")?;
    let mut errors = Vec::new();
    let mut limited = stderr.take(64 * 1024);
    let (status, _) = tokio::join!(child.wait(), limited.read_to_end(&mut errors));
    let status = status?;
    if !status.success() {
        bail!(
            "Copying files with rsync failed. Nothing was applied\n{}",
            String::from_utf8_lossy(&errors).trim()
        );
    }
    Ok(())
}

/// Escape each unusual character with a backslash. Some rsync versions split the remote
/// command on spaces and drop quotes before SSH joins it again, but a backslash before
/// every space survives both that and versions that pass the command unchanged.
fn escape_remote(path: &str) -> anyhow::Result<String> {
    ensure!(
        !path.is_empty()
            && !path
                .chars()
                .any(|c| c == '\'' || c == '"' || c.is_control()),
        "The Agent storage path contains characters rsync cannot use: {path:?}"
    );
    Ok(path
        .chars()
        .map(|c| match c.is_ascii_alphanumeric() || "/._-".contains(c) {
            true => c.to_string(),
            false => format!("\\{c}"),
        })
        .collect())
}

/// The `--files-from` list handed to rsync, removed when it goes out of scope. A `Drop`
/// rather than a cleanup line, so an early return cannot leave it behind in storage.
struct FileList(String);

impl FileList {
    fn write(body: &str) -> anyhow::Result<FileList> {
        let dir = project::client_root()?.join("lists");
        storage::private_dir(&dir)?;
        let file = dir.join(storage::new_id());
        storage::write_bytes(&file, format!("{body}\n").as_bytes())?;
        let path = file
            .to_str()
            .map(str::to_string)
            .context("Slingshot storage path is not UTF 8")?;
        Ok(FileList(path))
    }
}

impl Drop for FileList {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

async fn with_heartbeat(
    control: &mut Control,
    work: impl std::future::Future<Output = anyhow::Result<()>>,
) -> anyhow::Result<()> {
    tokio::pin!(work);
    let mut ticker = tokio::time::interval_at(tokio::time::Instant::now() + HEARTBEAT, HEARTBEAT);
    loop {
        tokio::select! {
            result = &mut work => return result,
            _ = ticker.tick() => {
                control.call(Request::Ping).await?;
            }
        }
    }
}

/// A readable preview of a plan. Words, not symbols, say what would happen.
fn preview_text(
    plan: &Plan,
    sender: &Manifest,
    receiver: &Manifest,
    agent: &str,
    direction: Direction,
    style: Style,
) -> String {
    let (from, to) = match direction {
        Direction::Push => ("this machine".to_string(), agent.to_string()),
        Direction::Pull => (agent.to_string(), "this machine".to_string()),
    };
    let mut output = format!(
        "\n{}\n\n",
        style.heading(format!("Sync preview from {from} to {to}"))
    );
    if plan.changes.is_empty() && plan.conflicts.is_empty() && plan.kept.is_empty() {
        output.push_str(&format!("  {to} already matches {from}\n"));
        return output;
    }
    for name in &plan.changes {
        let action = match (sender.contains_key(name), receiver.contains_key(name)) {
            (true, false) => format!("{:<9}", "Add"),
            (false, _) => style.paint(format!("{:<9}", "Delete"), Tone::Warning),
            (true, true) => format!("{:<9}", "Update"),
        };
        output.push_str(&format!("  {action} {name}\n"));
    }
    for name in &plan.conflicts {
        output.push_str(&format!(
            "  {} {name}\n",
            style.paint(format!("{:<9}", "Conflict"), Tone::Error)
        ));
    }
    for name in &plan.kept {
        output.push_str(&format!(
            "  {:<9} {name} {}\n",
            "Keep",
            style.dim(format!("(changed only on {to})"))
        ));
    }
    output.push_str(&format!(
        "\n  {}\n",
        style.dim(format!(
            "{} would be applied. Nothing was changed",
            presentation::plural(plan.changes.len(), "change")
        ))
    ));
    if !plan.conflicts.is_empty() {
        output.push_str(&format!(
            "  {} must be resolved first\n",
            presentation::plural(plan.conflicts.len(), "conflict")
        ));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use slingshot_core::source::Entry;

    fn file(hash: &str) -> Entry {
        Entry {
            hash: hash.into(),
            executable: false,
            link: None,
        }
    }

    #[test]
    fn a_conflict_says_what_happened_first_then_how_to_fix_it() {
        let one = conflict_message(&["NOTES.md".to_string()], "archbox");
        assert_eq!(
            one.lines().collect::<Vec<_>>(),
            [
                "NOTES.md changed on both this machine and archbox",
                "  Nothing was changed on either machine.",
                "  Compare      slingshot sync --check, then slingshot sync --pull --check",
                "  Fix          make the file match on both, or undo one side's edit, then sync again",
            ]
        );
        let many = conflict_message(&["a.rs".to_string(), "b.rs".to_string()], "archbox");
        assert!(
            many.starts_with("2 files changed on both this machine and archbox\n  a.rs\n  b.rs\n")
        );
        assert!(many.contains("make each file match on both"));
    }

    #[test]
    fn a_case_clash_names_both_files_and_the_fix() {
        let clashes = [("NOTES.md".to_string(), "notes.md".to_string())];
        let message = refuse_case_clashes(&clashes, "archbox")
            .unwrap_err()
            .to_string();
        assert!(message.starts_with("NOTES.md and notes.md differ only in case\n"));
        assert!(message.contains("rename or remove one of them on archbox"));
        assert!(refuse_case_clashes(&[], "archbox").is_ok());
    }

    #[test]
    fn this_disk_reports_whether_it_ignores_case() {
        let dir = std::env::temp_dir().join(format!("CaseProbe{}", storage::new_id()));
        std::fs::create_dir(&dir).unwrap();
        let lower = dir.with_file_name(dir.file_name().unwrap().to_str().unwrap().to_lowercase());
        assert_eq!(ignores_case(&dir), lower.exists());
        std::fs::remove_dir(&dir).unwrap();
    }

    #[test]
    fn a_pull_names_five_paths_then_sums_up_the_rest() {
        let paths: Vec<String> = (1..=7).map(|n| format!("f{n}.rs")).collect();
        assert_eq!(
            named(&paths),
            ["f1.rs", "f2.rs", "f3.rs", "f4.rs", "f5.rs", "and 2 more"]
        );
        assert_eq!(named(&paths[..2]), ["f1.rs", "f2.rs"]);
    }

    #[test]
    fn previews_name_every_action_in_words() {
        let base = Manifest::from([
            ("old".into(), file("a")),
            ("same".into(), file("a")),
            ("both".into(), file("a")),
        ]);
        let sender = Manifest::from([
            ("new".into(), file("n")),
            ("same".into(), file("b")),
            ("both".into(), file("b")),
        ]);
        let receiver = Manifest::from([
            ("old".into(), file("a")),
            ("same".into(), file("a")),
            ("both".into(), file("c")),
            ("mine".into(), file("m")),
        ]);
        let plan = sync::plan(&base, &sender, &receiver);
        let text = preview_text(
            &plan,
            &sender,
            &receiver,
            "archbox",
            Direction::Push,
            Style::new(false),
        );
        assert!(text.contains("Sync preview from this machine to archbox"));
        assert!(text.contains("Add       new"));
        assert!(text.contains("Delete    old"));
        assert!(text.contains("Update    same"));
        assert!(text.contains("Conflict  both"));
        assert!(text.contains("Keep      mine (changed only on archbox)"));
        assert!(text.contains("3 changes would be applied. Nothing was changed"));
        assert!(!text.contains('\x1b'));
    }

    #[test]
    fn links_are_not_sent_through_rsync() {
        let link = Entry {
            link: Some("a".into()),
            ..file("l")
        };
        let sender = Manifest::from([("a".into(), file("x")), ("b".into(), link)]);
        let plan = sync::plan(&Manifest::new(), &sender, &Manifest::new());
        assert_eq!(regular_files(&plan, &sender), ["a"]);
    }

    #[test]
    fn remote_paths_survive_rsync_splitting() {
        assert_eq!(
            escape_remote("/Users/me/Library/Application Support/slingshot").unwrap(),
            "/Users/me/Library/Application\\ Support/slingshot"
        );
        assert_eq!(escape_remote("/a/$(x)&b").unwrap(), "/a/\\$\\(x\\)\\&b");
        assert!(escape_remote("/it's").is_err());
        assert!(escape_remote("/a\nb").is_err());
    }

    #[test]
    fn conflicts_are_listed_with_a_way_forward() {
        let plan = Plan {
            conflicts: vec!["src/main.rs".into()],
            ..Plan::default()
        };
        let error = refuse_conflicts(&plan, "archbox").unwrap_err();
        assert!(error.downcast_ref::<Conflicts>().is_some());
        let error = error.to_string();
        assert!(error.starts_with("src/main.rs changed on both this machine and archbox"));
        assert!(error.contains("Nothing was changed"));
        assert!(error.contains("then sync again"));
    }
}
