//! Three way source synchronization with staged, recoverable application.
//!
//! The sender is the side whose edits are being copied and the receiver is the side
//! being updated. Both are compared with the last shared baseline, so edits made only
//! on the receiver are kept and paths changed differently on both sides stop the sync.

use crate::source::{self, Entry, Manifest};
use crate::storage::{self, PARTIAL_PREFIX};
use anyhow::{Context, bail, ensure};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

/// Source backup sets kept per project. Older sets are removed after a successful sync.
const BACKUPS_KEPT: usize = 20;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Plan {
    /// Paths the receiver must add, replace, or delete.
    pub changes: Vec<String>,
    /// Paths changed differently on both sides since the baseline.
    pub conflicts: Vec<String>,
    /// Paths changed only on the receiver, which are left alone.
    pub kept: Vec<String>,
    /// The baseline to record once the changes are applied.
    pub baseline: Manifest,
}

pub fn plan(base: &Manifest, sender: &Manifest, receiver: &Manifest) -> Plan {
    let mut result = Plan {
        baseline: base.clone(),
        ..Plan::default()
    };
    let names: BTreeSet<&String> = base
        .keys()
        .chain(sender.keys())
        .chain(receiver.keys())
        .collect();
    for name in names {
        let (before, from, to) = (base.get(name), sender.get(name), receiver.get(name));
        if from == to {
            record(&mut result.baseline, name, from);
        } else if from == before {
            result.kept.push(name.clone());
        } else if to == before {
            result.changes.push(name.clone());
            record(&mut result.baseline, name, from);
        } else {
            result.conflicts.push(name.clone());
        }
    }
    result
}

/// Settle the conflicts in `allowed` in the sender's favor, as the owner chose with
/// `--overwrite`. The receiver keeps a backup of each file it replaces, like any change.
pub fn overwrite(mut plan: Plan, sender: &Manifest, allowed: &[String]) -> Plan {
    let (replaced, left): (Vec<String>, Vec<String>) = std::mem::take(&mut plan.conflicts)
        .into_iter()
        .partition(|name| allowed.contains(name));
    for name in replaced {
        record(&mut plan.baseline, &name, sender.get(&name));
        plan.changes.push(name);
    }
    plan.changes.sort();
    plan.conflicts = left;
    plan
}

/// Pairs of paths that differ only in case, which a disk that ignores case, such as a Mac's
/// by default, cannot hold side by side.
pub fn case_clashes(manifest: &Manifest) -> Vec<(String, String)> {
    let mut seen: BTreeMap<String, &String> = BTreeMap::new();
    let mut clashes = Vec::new();
    for name in manifest.keys() {
        match seen.get(&name.to_lowercase()) {
            Some(first) => clashes.push(((*first).clone(), name.clone())),
            None => {
                seen.insert(name.to_lowercase(), name);
            }
        }
    }
    clashes
}

/// The baseline after a transfer: paths where both sides now agree take the shared
/// value, and every other path keeps its previous baseline entry.
pub fn agreed(base: &Manifest, one: &Manifest, other: &Manifest) -> Manifest {
    let mut result = base.clone();
    let names: BTreeSet<&String> = one.keys().chain(other.keys()).chain(base.keys()).collect();
    for name in names {
        if one.get(name) == other.get(name) {
            record(&mut result, name, one.get(name));
        }
    }
    result
}

/// The receiver's manifest once a plan has been applied to it. Both machines project the
/// result this way before reporting it, so their baselines cannot drift apart.
pub fn merge(receiver: &Manifest, changes: &[String], sender: &Manifest) -> Manifest {
    let mut result = receiver.clone();
    for name in changes {
        record(&mut result, name, sender.get(name));
    }
    result
}

fn record(baseline: &mut Manifest, name: &str, value: Option<&Entry>) {
    match value {
        Some(value) => baseline.insert(name.to_string(), value.clone()),
        None => baseline.remove(name),
    };
}

/// Where one side keeps synchronization state for a project.
pub struct StateDir {
    pub dir: PathBuf,
}

impl StateDir {
    pub fn new(dir: impl Into<PathBuf>) -> StateDir {
        StateDir { dir: dir.into() }
    }

    pub fn baseline_file(&self) -> PathBuf {
        self.dir.join("baseline.json")
    }

    fn journal_file(&self) -> PathBuf {
        self.dir.join("journal.json")
    }

    pub fn backups(&self) -> PathBuf {
        self.dir.join("backups")
    }

    pub fn baseline(&self) -> anyhow::Result<Manifest> {
        storage::read_json(&self.baseline_file())
    }

    pub fn interrupted(&self) -> bool {
        self.journal_file().exists()
    }
}

#[derive(Serialize, Deserialize)]
struct Journal {
    token: String,
    backup: PathBuf,
    steps: Vec<Step>,
    baseline: Manifest,
}

#[derive(Serialize, Deserialize)]
struct Step {
    name: String,
    before: Option<Entry>,
    after: Option<Entry>,
}

/// Apply a plan to `root` using regular files staged under `stage`, and report how many
/// paths changed.
///
/// Every destination is compared with what the plan expected, and every staged file is
/// hashed, before anything is touched. A journal and backups are written first, and the
/// new baseline is the commit point. Any failure rolls the changes back.
pub fn apply(
    root: &Path,
    stage: &Path,
    state: &StateDir,
    plan: &Plan,
    receiver: &Manifest,
    sender: &Manifest,
    token: &str,
) -> anyhow::Result<usize> {
    ensure!(
        plan.conflicts.is_empty(),
        "Resolve source conflicts before syncing"
    );
    ensure!(
        !state.interrupted(),
        "An interrupted sync must be recovered first"
    );
    storage::private_dir(&state.dir)?;
    if plan.changes.is_empty() {
        storage::write_json(&state.baseline_file(), &plan.baseline)?;
        return Ok(0);
    }

    let deleted: BTreeSet<&str> = plan
        .changes
        .iter()
        .filter(|name| !sender.contains_key(*name))
        .map(String::as_str)
        .collect();
    for name in &plan.changes {
        storage::relative(name)?;
        let prefix = format!("{name}/");
        let emptied = fs::symlink_metadata(root.join(name)).is_ok_and(|m| m.is_dir())
            && deleted.iter().any(|d| d.starts_with(&prefix));
        let current = match emptied {
            true => None,
            false => source::entry(root, name).with_context(|| format!("Cannot update {name}"))?,
        };
        ensure!(
            current.as_ref() == receiver.get(name),
            "{name} changed during the sync. Run the sync again"
        );
        check_parents(root, name, &deleted)?;
        if let Some(expected) = sender.get(name)
            && expected.link.is_none()
        {
            let staged = storage::safe_path(stage, name)?;
            let meta = fs::symlink_metadata(&staged)
                .with_context(|| format!("Transfer is missing {name}"))?;
            ensure!(
                meta.is_file(),
                "Transferred item is not a regular file: {name}"
            );
            ensure!(
                source::hash_file(&staged)? == expected.hash,
                "{name} changed during the transfer. Run the sync again"
            );
        }
    }

    let backup = state.backups().join(format!("{}-{token}", storage::now()));
    let replaces = plan.changes.iter().any(|name| receiver.contains_key(name));
    if replaces {
        storage::private_dir(&backup)?;
    }
    let mut steps: Vec<Step> = plan
        .changes
        .iter()
        .map(|name| Step {
            name: name.clone(),
            before: receiver.get(name).cloned(),
            after: sender.get(name).cloned(),
        })
        .collect();
    steps.sort_by(|a, b| match (a.after.is_none(), b.after.is_none()) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        (true, true) => b.name.cmp(&a.name),
        (false, false) => a.name.cmp(&b.name),
    });
    let journal = Journal {
        token: token.to_string(),
        backup: backup.clone(),
        steps,
        baseline: plan.baseline.clone(),
    };
    storage::write_json(&state.journal_file(), &journal)?;

    let outcome = (|| {
        for step in &journal.steps {
            apply_step(root, stage, &backup, step, token)
                .with_context(|| format!("Could not update {}", step.name))?;
        }
        storage::write_json(&state.baseline_file(), &plan.baseline)
    })();

    if let Err(error) = outcome {
        return match recover(root, state) {
            Ok(()) => Err(error.context("The sync was rolled back")),
            Err(recovery) => Err(error.context(format!("Rollback also failed: {recovery:#}"))),
        };
    }
    fs::remove_file(state.journal_file())?;
    prune_backups(state);
    Ok(plan.changes.len())
}

/// Every existing parent must be a real directory, unless it is a file this plan deletes.
fn check_parents(root: &Path, name: &str, deleted: &BTreeSet<&str>) -> anyhow::Result<()> {
    let mut current = root.to_path_buf();
    let mut rel = String::new();
    let parts: Vec<&str> = name.split('/').collect();
    for part in &parts[..parts.len() - 1] {
        current.push(part);
        if !rel.is_empty() {
            rel.push('/');
        }
        rel.push_str(part);
        match fs::symlink_metadata(&current) {
            Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => {}
            Ok(_) if deleted.contains(rel.as_str()) => return Ok(()),
            Ok(_) => bail!("{rel} is in the way of {name}"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

/// Put one path into its final state. `entry` of `None` deletes it. Content is staged
/// beside the destination, given its mode, and renamed over the top, so a crash leaves
/// either the old file or the new one and never a half written mix.
///
/// `from` is the directory holding the replacement content. `verify` re-hashes it after
/// the copy, which is worth doing for content that crossed the network and not for a
/// backup this machine wrote itself.
fn install(
    root: &Path,
    dest: &Path,
    name: &str,
    entry: Option<&Entry>,
    from: &Path,
    verify: bool,
    token: &str,
) -> anyhow::Result<()> {
    let Some(entry) = entry else {
        fs::remove_file(dest)?;
        remove_empty_parents(root, name);
        return Ok(());
    };

    storage::create_parents(root, name)?;
    let temp = partial_path(dest, token);
    let _ = fs::remove_file(&temp);
    match &entry.link {
        Some(target) => std::os::unix::fs::symlink(target, &temp)?,
        None => {
            copy_file(&storage::safe_path(from, name)?, &temp)?;
            if verify {
                ensure!(
                    source::hash_file(&temp)? == entry.hash,
                    "Staged content changed while it was being installed"
                );
            }
            let old = fs::symlink_metadata(dest)
                .ok()
                .filter(|m| m.is_file())
                .map(|m| m.permissions().mode());
            fs::set_permissions(
                &temp,
                fs::Permissions::from_mode(mode(old, entry.executable)),
            )?;
        }
    }
    fs::rename(&temp, dest)?;
    Ok(())
}

fn apply_step(
    root: &Path,
    stage: &Path,
    backup: &Path,
    step: &Step,
    token: &str,
) -> anyhow::Result<()> {
    let dest = storage::safe_path(root, &step.name)?;
    if step.before.is_some() {
        copy_entry(&dest, backup, &step.name)?;
    }
    install(
        root,
        &dest,
        &step.name,
        step.after.as_ref(),
        stage,
        true,
        token,
    )
}

/// Keep the receiver's existing read and write bits and change only execute bits.
fn mode(old: Option<u32>, executable: bool) -> u32 {
    let base = old.map(|m| m & 0o666).unwrap_or(0o644);
    match executable {
        true => base | ((base & 0o444) >> 2) | 0o100,
        false => base,
    }
}

fn partial_path(dest: &Path, token: &str) -> PathBuf {
    let parent = dest.parent().unwrap_or(Path::new("."));
    let name = dest
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let short: String = name.chars().take(64).collect();
    parent.join(format!("{PARTIAL_PREFIX}{token}-{short}"))
}

/// Copy a file or link into the backup tree, flushed to disk before the original changes.
fn copy_entry(from: &Path, backup: &Path, name: &str) -> anyhow::Result<()> {
    storage::create_parents(backup, name)?;
    let saved = storage::safe_path(backup, name)?;
    let _ = fs::remove_file(&saved);
    let meta = fs::symlink_metadata(from)?;
    if meta.file_type().is_symlink() {
        std::os::unix::fs::symlink(fs::read_link(from)?, &saved)?;
    } else {
        copy_file(from, &saved)?;
        fs::set_permissions(
            &saved,
            fs::Permissions::from_mode(meta.permissions().mode() & 0o700),
        )?;
    }
    Ok(())
}

fn copy_file(from: &Path, to: &Path) -> anyhow::Result<()> {
    let mut input = File::open(from)?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(to)?;
    std::io::copy(&mut input, &mut output)?;
    output.flush()?;
    output.sync_all()?;
    Ok(())
}

fn remove_empty_parents(root: &Path, name: &str) {
    let mut path = root.join(name);
    while let Some(parent) = path.parent() {
        if parent == root || fs::remove_dir(parent).is_err() {
            break;
        }
        path = parent.to_path_buf();
    }
}

/// Finish or undo an interrupted sync.
///
/// If the new baseline was already written, the sync completed and only the journal
/// remains. Otherwise each path is returned to its previous state from the backup.
/// A path that matches neither the old nor the new state was edited afterwards, so it
/// is left untouched and recovery stops with instructions.
pub fn recover(root: &Path, state: &StateDir) -> anyhow::Result<()> {
    if !state.interrupted() {
        return Ok(());
    }
    let journal: Journal = serde_json::from_slice(&fs::read(state.journal_file())?)
        .context("The sync recovery journal is unreadable")?;
    let committed = storage::read_json::<Option<Manifest>>(&state.baseline_file())
        .ok()
        .flatten()
        .is_some_and(|baseline| baseline == journal.baseline);

    let mut unresolved = Vec::new();
    if !committed {
        for step in journal.steps.iter().rev() {
            if let Err(error) = undo_step(root, &journal, step) {
                unresolved.push(format!("{}: {error:#}", step.name));
            }
        }
    }
    for step in &journal.steps {
        if let Ok(dest) = storage::safe_path(root, &step.name) {
            let _ = fs::remove_file(partial_path(&dest, &journal.token));
        }
    }
    if !unresolved.is_empty() {
        bail!(
            "Could not recover an interrupted sync. Backups are in {}\n{}\nMake each path match its backup or delete it, then run the sync again",
            journal.backup.display(),
            unresolved.join("\n")
        );
    }
    fs::remove_file(state.journal_file())?;
    Ok(())
}

fn undo_step(root: &Path, journal: &Journal, step: &Step) -> anyhow::Result<()> {
    let current = source::entry(root, &step.name)?;
    if current == step.before {
        return Ok(());
    }
    ensure!(
        current == step.after,
        "It changed after the sync was interrupted"
    );
    let dest = storage::safe_path(root, &step.name)?;
    if let Some(before) = &step.before {
        ensure!(
            source::entry(&journal.backup, &step.name)?.is_some_and(|e| e.hash == before.hash),
            "Its backup is missing or incomplete"
        );
    }
    install(
        root,
        &dest,
        &step.name,
        step.before.as_ref(),
        &journal.backup,
        false,
        &journal.token,
    )
}

fn prune_backups(state: &StateDir) {
    let Ok(items) = fs::read_dir(state.backups()) else {
        return;
    };
    let mut sets: Vec<(u64, PathBuf)> = items
        .flatten()
        .filter_map(|item| {
            let name = item.file_name().to_string_lossy().to_string();
            let stamp = name.split('-').next()?.parse().ok()?;
            Some((stamp, item.path()))
        })
        .collect();
    sets.sort();
    let excess = sets.len().saturating_sub(BACKUPS_KEPT);
    for (_, path) in sets.into_iter().take(excess) {
        let _ = fs::remove_dir_all(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::{Rules, scan};
    use crate::storage::testing::TempDir;

    fn file(hash: &str) -> Entry {
        Entry {
            hash: hash.into(),
            executable: false,
            link: None,
        }
    }

    fn manifest(items: &[(&str, &str)]) -> Manifest {
        items
            .iter()
            .map(|(name, hash)| (name.to_string(), file(hash)))
            .collect()
    }

    /// An interrupted step can leave a symlink where a regular file used to be. Reading
    /// the destination's permissions must not follow that link, or the restored file
    /// inherits the mode of whatever the link pointed at, possibly outside the project.
    #[test]
    fn overwriting_settles_only_the_chosen_conflicts_for_the_sender() {
        let base = manifest(&[("a.rs", "0"), ("b.rs", "0")]);
        let sender = manifest(&[("a.rs", "mine"), ("b.rs", "mine")]);
        let receiver = manifest(&[("a.rs", "theirs"), ("b.rs", "theirs")]);
        let settled = overwrite(
            plan(&base, &sender, &receiver),
            &sender,
            &["a.rs".to_string()],
        );
        assert_eq!(settled.changes, ["a.rs"]);
        assert_eq!(settled.conflicts, ["b.rs"]);
        assert_eq!(settled.baseline.get("a.rs"), sender.get("a.rs"));
    }

    #[test]
    fn names_that_differ_only_in_case_are_found() {
        let names = manifest(&[("NOTES.md", "a"), ("notes.md", "b"), ("src/main.rs", "c")]);
        assert_eq!(
            case_clashes(&names),
            [("NOTES.md".to_string(), "notes.md".to_string())]
        );
        assert!(case_clashes(&manifest(&[("a.rs", "a"), ("b.rs", "b")])).is_empty());
    }

    #[test]
    fn undoing_a_symlink_does_not_take_its_targets_permissions() {
        let root = TempDir::new("undo");
        let backup = TempDir::new("undo-backup");
        let outside = TempDir::new("undo-outside");

        let target = outside.write("wide-open", "elsewhere");
        fs::set_permissions(&target, fs::Permissions::from_mode(0o666)).unwrap();
        backup.write("a.txt", "original");
        std::os::unix::fs::symlink(&target, root.path().join("a.txt")).unwrap();

        let before = source::entry(backup.path(), "a.txt").unwrap();
        let after = source::entry(root.path(), "a.txt").unwrap();
        assert!(after.as_ref().is_some_and(|e| e.link.is_some()));

        let journal = Journal {
            token: "t".into(),
            backup: backup.path().to_path_buf(),
            steps: Vec::new(),
            baseline: Manifest::new(),
        };
        let step = Step {
            name: "a.txt".into(),
            before,
            after,
        };

        undo_step(root.path(), &journal, &step).unwrap();

        let restored = root.path().join("a.txt");
        assert_eq!(fs::read_to_string(&restored).unwrap(), "original");
        assert_eq!(
            fs::symlink_metadata(&restored)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o644
        );
    }

    #[test]
    fn planning_covers_every_three_way_case() {
        let base = manifest(&[
            ("same", "a"),
            ("sent", "a"),
            ("kept", "a"),
            ("both", "a"),
            ("gone", "a"),
            ("agreed", "a"),
        ]);
        let sender = manifest(&[
            ("same", "a"),
            ("sent", "b"),
            ("kept", "a"),
            ("both", "b"),
            ("new", "n"),
            ("agreed", "z"),
        ]);
        let receiver = manifest(&[
            ("same", "a"),
            ("sent", "a"),
            ("kept", "c"),
            ("both", "c"),
            ("gone", "a"),
            ("agreed", "z"),
        ]);
        let plan = plan(&base, &sender, &receiver);
        assert_eq!(plan.changes, ["gone", "new", "sent"]);
        assert_eq!(plan.conflicts, ["both"]);
        assert_eq!(plan.kept, ["kept"]);
        assert_eq!(plan.baseline.get("sent"), Some(&file("b")));
        assert_eq!(plan.baseline.get("agreed"), Some(&file("z")));
        assert!(!plan.baseline.contains_key("gone"));
        assert_eq!(plan.baseline.get("kept"), Some(&file("a")));
    }

    #[test]
    fn the_recorded_baseline_only_advances_where_both_sides_agree() {
        let base = manifest(&[("a", "1"), ("b", "1"), ("c", "1")]);
        let agent = manifest(&[("a", "2"), ("b", "2"), ("d", "4")]);
        let client = manifest(&[("a", "2"), ("b", "1"), ("c", "1"), ("d", "4")]);
        let result = agreed(&base, &agent, &client);
        assert_eq!(
            result,
            manifest(&[("a", "2"), ("b", "1"), ("c", "1"), ("d", "4")])
        );
    }

    #[test]
    fn a_deletion_on_both_sides_is_agreement() {
        let base = manifest(&[("x", "a")]);
        let plan = plan(&base, &Manifest::new(), &Manifest::new());
        assert!(plan.changes.is_empty() && plan.conflicts.is_empty());
        assert!(plan.baseline.is_empty());
    }

    #[test]
    fn a_path_never_synchronized_is_not_deleted() {
        let plan = plan(
            &Manifest::new(),
            &Manifest::new(),
            &manifest(&[("local", "a")]),
        );
        assert!(plan.changes.is_empty());
        assert_eq!(plan.kept, ["local"]);
    }

    struct Sides {
        sender: TempDir,
        receiver: TempDir,
        state: TempDir,
    }

    impl Sides {
        fn new() -> Sides {
            Sides {
                sender: TempDir::new("sender"),
                receiver: TempDir::new("receiver"),
                state: TempDir::new("state"),
            }
        }

        fn scan(&self, dir: &TempDir, baseline: &Manifest) -> Manifest {
            scan(
                dir.path(),
                &Rules::new(dir.path(), &[]).unwrap(),
                baseline,
                None,
            )
            .unwrap()
        }

        fn sync(&self) -> anyhow::Result<Plan> {
            let state = StateDir::new(self.state.path());
            let base = state.baseline()?;
            let sender = self.scan(&self.sender, &base);
            let receiver = self.scan(&self.receiver, &base);
            let plan = plan(&base, &sender, &receiver);
            apply(
                self.receiver.path(),
                self.sender.path(),
                &state,
                &plan,
                &receiver,
                &sender,
                "t1",
            )?;
            Ok(plan)
        }
    }

    /// A Python `.venv` lives in the Agent copy, so no sync may ever touch it.
    #[test]
    fn a_venv_on_the_receiver_survives_every_sync() {
        let sides = Sides::new();
        sides.sender.write("main.py", "print(1)");
        sides.sender.write("old.py", "old");
        sides.sync().unwrap();
        sides.receiver.write(".venv/pyvenv.cfg", "home = /usr/bin");
        sides.receiver.write(".venv/bin/python", "python");

        fs::remove_file(sides.sender.path().join("old.py")).unwrap();
        sides.sender.write("main.py", "print(2)");
        let plan = sides.sync().unwrap();

        assert_eq!(plan.changes, ["main.py", "old.py"]);
        let receiver = sides.receiver.path();
        assert_eq!(
            fs::read_to_string(receiver.join(".venv/pyvenv.cfg")).unwrap(),
            "home = /usr/bin"
        );
        assert!(receiver.join(".venv/bin/python").exists());
    }

    #[test]
    fn apply_copies_changes_and_advances_the_baseline() {
        let sides = Sides::new();
        sides.sender.write("src/main.rs", "fn main() {}");
        sides.sender.write("old.txt", "old");
        let script = sides.sender.write("run.sh", "#!/bin/sh");
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        std::os::unix::fs::symlink("src/main.rs", sides.sender.path().join("alias")).unwrap();
        sides.sync().unwrap();
        let receiver = sides.receiver.path();
        assert_eq!(
            fs::read_to_string(receiver.join("src/main.rs")).unwrap(),
            "fn main() {}"
        );
        assert_eq!(
            fs::read_link(receiver.join("alias")).unwrap(),
            Path::new("src/main.rs")
        );
        assert!(
            fs::metadata(receiver.join("run.sh"))
                .unwrap()
                .permissions()
                .mode()
                & 0o100
                != 0
        );

        fs::remove_file(sides.sender.path().join("old.txt")).unwrap();
        sides.sender.write("src/main.rs", "fn main() { work() }");
        sides.receiver.write("receiver_only.txt", "mine");
        let plan = sides.sync().unwrap();
        assert_eq!(plan.changes, ["old.txt", "src/main.rs"]);
        assert!(!receiver.join("old.txt").exists());
        assert!(receiver.join("receiver_only.txt").exists());
        let backups: Vec<_> = fs::read_dir(StateDir::new(sides.state.path()).backups())
            .unwrap()
            .collect();
        assert_eq!(backups.len(), 1);
        assert!(!StateDir::new(sides.state.path()).interrupted());
    }

    #[test]
    fn conflicting_edits_stop_before_anything_changes() {
        let sides = Sides::new();
        sides.sender.write("a.txt", "base");
        sides.sync().unwrap();
        sides.sender.write("a.txt", "sender");
        sides.sender.write("b.txt", "new");
        sides.receiver.write("a.txt", "receiver");
        assert!(sides.sync().is_err());
        assert_eq!(
            fs::read_to_string(sides.receiver.path().join("a.txt")).unwrap(),
            "receiver"
        );
        assert!(!sides.receiver.path().join("b.txt").exists());
    }

    #[test]
    fn a_destination_edited_after_planning_is_not_overwritten() {
        let sides = Sides::new();
        sides.sender.write("a.txt", "one");
        let state = StateDir::new(sides.state.path());
        let sender = sides.scan(&sides.sender, &Manifest::new());
        let receiver = sides.scan(&sides.receiver, &Manifest::new());
        let plan = plan(&Manifest::new(), &sender, &receiver);
        sides.receiver.write("a.txt", "surprise");
        assert!(
            apply(
                sides.receiver.path(),
                sides.sender.path(),
                &state,
                &plan,
                &receiver,
                &sender,
                "t"
            )
            .is_err()
        );
        assert_eq!(
            fs::read_to_string(sides.receiver.path().join("a.txt")).unwrap(),
            "surprise"
        );
        assert!(!state.interrupted());
    }

    #[test]
    fn a_staged_file_that_does_not_match_is_rejected() {
        let sides = Sides::new();
        sides.sender.write("a.txt", "one");
        let state = StateDir::new(sides.state.path());
        let sender = sides.scan(&sides.sender, &Manifest::new());
        let plan = plan(&Manifest::new(), &sender, &Manifest::new());
        sides.sender.write("a.txt", "tampered");
        assert!(
            apply(
                sides.receiver.path(),
                sides.sender.path(),
                &state,
                &plan,
                &Manifest::new(),
                &sender,
                "t"
            )
            .is_err()
        );
        assert!(!sides.receiver.path().join("a.txt").exists());
    }

    #[test]
    fn a_failure_midway_rolls_every_path_back() {
        let sides = Sides::new();
        sides.sender.write("a.txt", "one");
        sides.sender.write("locked/b.txt", "one");
        sides.sync().unwrap();
        sides.sender.write("a.txt", "two");
        sides.sender.write("locked/b.txt", "two");
        let locked = sides.receiver.path().join("locked");
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o555)).unwrap();
        let result = sides.sync();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
        let error = format!("{:#}", result.unwrap_err());
        assert!(error.contains("rolled back"), "{error}");
        assert_eq!(
            fs::read_to_string(sides.receiver.path().join("a.txt")).unwrap(),
            "one"
        );
        assert_eq!(fs::read_to_string(locked.join("b.txt")).unwrap(), "one");
        assert!(!StateDir::new(sides.state.path()).interrupted());
    }

    #[test]
    fn recovery_rolls_back_an_interrupted_sync() {
        let sides = Sides::new();
        sides.sender.write("a.txt", "one");
        sides.sender.write("gone.txt", "bye");
        sides.sync().unwrap();
        let state = StateDir::new(sides.state.path());
        let before = state.baseline().unwrap();
        let receiver = sides.receiver.path();

        sides.sender.write("a.txt", "two");
        sides.sender.write("added.txt", "new");
        fs::remove_file(sides.sender.path().join("gone.txt")).unwrap();
        let sender = sides.scan(&sides.sender, &before);
        let current = sides.scan(&sides.receiver, &before);
        let plan = plan(&before, &sender, &current);
        let backup = state.backups().join("9-crash");
        let mut steps = Vec::new();
        for name in &plan.changes {
            steps.push(Step {
                name: name.clone(),
                before: current.get(name).cloned(),
                after: sender.get(name).cloned(),
            });
        }
        storage::private_dir(&backup).unwrap();
        let journal = Journal {
            token: "crash".into(),
            backup: backup.clone(),
            steps,
            baseline: plan.baseline.clone(),
        };
        storage::write_json(&state.journal_file(), &journal).unwrap();
        for step in &journal.steps[..2] {
            apply_step(receiver, sides.sender.path(), &backup, step, "crash").unwrap();
        }
        fs::write(
            receiver.join(format!("{PARTIAL_PREFIX}crash-a.txt")),
            "partial",
        )
        .unwrap();

        recover(receiver, &state).unwrap();
        assert!(!state.interrupted());
        assert_eq!(fs::read_to_string(receiver.join("a.txt")).unwrap(), "one");
        assert_eq!(
            fs::read_to_string(receiver.join("gone.txt")).unwrap(),
            "bye"
        );
        assert!(!receiver.join("added.txt").exists());
        assert!(
            !receiver
                .join(format!("{PARTIAL_PREFIX}crash-a.txt"))
                .exists()
        );
        assert_eq!(state.baseline().unwrap(), before);
    }

    #[test]
    fn recovery_after_the_commit_point_keeps_the_new_state() {
        let sides = Sides::new();
        sides.sender.write("a.txt", "one");
        let state = StateDir::new(sides.state.path());
        sides.sync().unwrap();
        let baseline = state.baseline().unwrap();
        let journal = Journal {
            token: "t".into(),
            backup: state.backups().join("x"),
            steps: vec![Step {
                name: "a.txt".into(),
                before: None,
                after: baseline.get("a.txt").cloned(),
            }],
            baseline: baseline.clone(),
        };
        storage::write_json(&state.journal_file(), &journal).unwrap();
        recover(sides.receiver.path(), &state).unwrap();
        assert!(sides.receiver.path().join("a.txt").exists());
        assert!(!state.interrupted());
    }

    #[test]
    fn recovery_refuses_to_touch_a_path_edited_after_the_crash() {
        let sides = Sides::new();
        sides.sender.write("a.txt", "one");
        sides.sync().unwrap();
        let state = StateDir::new(sides.state.path());
        let base = state.baseline().unwrap();
        let journal = Journal {
            token: "t".into(),
            backup: state.backups().join("x"),
            steps: vec![Step {
                name: "a.txt".into(),
                before: base.get("a.txt").cloned(),
                after: Some(file("ffff")),
            }],
            baseline: Manifest::new(),
        };
        storage::write_json(&state.journal_file(), &journal).unwrap();
        sides.receiver.write("a.txt", "edited by hand");
        let error = recover(sides.receiver.path(), &state)
            .unwrap_err()
            .to_string();
        assert!(error.contains("a.txt"), "{error}");
        assert!(state.interrupted());
        assert_eq!(
            fs::read_to_string(sides.receiver.path().join("a.txt")).unwrap(),
            "edited by hand"
        );
        assert!(
            apply(
                sides.receiver.path(),
                sides.sender.path(),
                &state,
                &Plan::default(),
                &base,
                &base,
                "u"
            )
            .is_err()
        );
    }

    #[test]
    fn a_file_can_replace_a_folder_it_used_to_be() {
        let sides = Sides::new();
        sides.sender.write("thing/inner.txt", "x");
        sides.sync().unwrap();
        fs::remove_dir_all(sides.sender.path().join("thing")).unwrap();
        sides.sender.write("thing", "now a file");
        sides.sync().unwrap();
        assert_eq!(
            fs::read_to_string(sides.receiver.path().join("thing")).unwrap(),
            "now a file"
        );
    }

    #[test]
    fn unknown_files_in_the_way_stop_the_sync() {
        let sides = Sides::new();
        sides.sender.write("thing/inner.txt", "x");
        sides.receiver.write("thing", "unrelated file");
        assert!(sides.sync().is_err());
        assert_eq!(
            fs::read_to_string(sides.receiver.path().join("thing")).unwrap(),
            "unrelated file"
        );
    }

    #[test]
    fn only_execute_bits_change_on_existing_files() {
        assert_eq!(mode(Some(0o100640), true), 0o750);
        assert_eq!(mode(Some(0o104755), true), 0o755);
        assert_eq!(mode(None, false), 0o644);
        assert_eq!(mode(None, true), 0o755);
        assert_eq!(mode(Some(0o755), false), 0o644);
    }
}
