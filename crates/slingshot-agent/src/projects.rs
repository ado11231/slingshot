//! Project storage on the Agent: source copies, sync leases, build output, and
//! environment files. Each project ID gets separate folders for each kind of data.

use crate::jobs;
use anyhow::{Context, bail, ensure};
use serde::{Deserialize, Serialize};
use slingshot_core::artifacts::{self, Layout};
use slingshot_core::control::{
    Job, JobKind, MAX_ENVIRONMENT_FILE, ProjectInfo, ProjectRef, Snapshot,
};
use slingshot_core::source::{self, Manifest, Rules};
use slingshot_core::sync::{self, StateDir};
use slingshot_core::{stack, storage};
use std::fs::{self, File};
use std::path::{Path, PathBuf};

pub struct Paths {
    pub dir: PathBuf,
    pub source: PathBuf,
    pub artifacts: PathBuf,
    pub environment: PathBuf,
    pub state: PathBuf,
}

impl Paths {
    pub fn new(root: &Path, id: &str) -> anyhow::Result<Paths> {
        storage::check_id(id)?;
        let dir = root.join("projects").join(id);
        Ok(Paths {
            source: dir.join("source"),
            artifacts: dir.join("artifacts"),
            environment: dir.join("environment"),
            state: dir.join("state"),
            dir,
        })
    }

    fn metadata(&self) -> PathBuf {
        self.dir.join("project.json")
    }

    fn lock(&self) -> PathBuf {
        self.state.join("work.lock")
    }

    fn staging(&self) -> PathBuf {
        self.state.join("staging")
    }

    fn hashes(&self) -> PathBuf {
        self.state.join("hashes.json")
    }

    fn names(&self) -> PathBuf {
        self.environment.join("names.json")
    }

    fn sync_state(&self) -> StateDir {
        StateDir::new(&self.state)
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Metadata {
    pub id: String,
    pub name: String,
    pub client: String,
    pub created: u64,
}

/// Create a project's folders if needed. Opening never copies or changes source.
pub fn open(root: &Path, project: &ProjectRef) -> anyhow::Result<ProjectInfo> {
    let paths = Paths::new(root, &project.id)?;
    for text in [&project.name, &project.client] {
        ensure!(
            !text.is_empty() && text.len() <= 255 && !text.chars().any(char::is_control),
            "Invalid project or Client name"
        );
    }
    storage::private_dir(&root.join("projects"))?;
    for dir in [
        &paths.dir,
        &paths.source,
        &paths.artifacts,
        &paths.environment,
        &paths.state,
    ] {
        storage::private_dir(dir)?;
    }
    let existing: Option<Metadata> = storage::read_json(&paths.metadata())?;
    let metadata = Metadata {
        id: project.id.clone(),
        name: project.name.clone(),
        client: project.client.clone(),
        created: existing
            .as_ref()
            .map(|m| m.created)
            .unwrap_or_else(storage::now),
    };
    if existing
        .as_ref()
        .is_none_or(|m| m.name != metadata.name || m.client != metadata.client)
    {
        storage::write_json(&paths.metadata(), &metadata)?;
    }
    Ok(ProjectInfo {
        id: metadata.id,
        name: metadata.name,
        initialized: paths.sync_state().baseline_file().exists(),
    })
}

pub fn load(root: &Path, id: &str) -> anyhow::Result<(Paths, Metadata)> {
    let paths = Paths::new(root, id)?;
    let metadata: Option<Metadata> = storage::read_json(&paths.metadata())?;
    let metadata =
        metadata.context("This project has no copy on the Agent yet. Run slingshot sync first")?;
    Ok((paths, metadata))
}

/// Which active jobs stop other work on a project. A run owns the project copy while it
/// works, so nothing else may change it. A session is a place to work, like a terminal on
/// the Client, so syncing and running beside it are allowed. Unlinking waits for both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Blocking {
    Runs,
    AllJobs,
}

fn blocker(active: &[Job], blocking: Blocking) -> Option<&Job> {
    active
        .iter()
        .find(|job| blocking == Blocking::AllJobs || job.kind == JobKind::Run)
}

/// Hold the project lock and confirm no job that `blocking` names is active.
pub fn acquire(
    root: &Path,
    paths: &Paths,
    metadata: &Metadata,
    blocking: Blocking,
) -> anyhow::Result<File> {
    let active = jobs::active_for(root, &metadata.id)?;
    let lock = storage::try_lock(&paths.lock())?;
    let describe = |kind: JobKind| match kind {
        JobKind::Run => "a run",
        JobKind::Session => "a session",
    };
    if let Some(job) = blocker(&active, blocking) {
        bail!(
            "{} has {} in progress (job {}). Stop it with slingshot stop {} or wait for it to finish",
            metadata.name,
            describe(job.kind),
            storage::short_id(&job.id),
            storage::short_id(&job.id)
        );
    }
    let Some(lock) = lock else {
        bail!(
            "{} is busy with a sync or another run. Try again when it finishes",
            metadata.name
        );
    };
    ensure!(
        blocker(&jobs::active_for(root, &metadata.id)?, blocking).is_none(),
        "{} started other work just now. Try again",
        metadata.name
    );
    Ok(lock)
}

/// Refuse work while an interrupted sync is waiting to be recovered.
pub fn ensure_ready(paths: &Paths, metadata: &Metadata) -> anyhow::Result<()> {
    let state = paths.sync_state();
    ensure!(
        !state.interrupted(),
        "An interrupted sync of {} needs recovery. Run slingshot sync to recover it",
        metadata.name
    );
    ensure!(
        state.baseline_file().exists(),
        "{} has not been copied to the Agent yet. Run slingshot sync first",
        metadata.name
    );
    Ok(())
}

/// A source transfer in progress. Dropping it releases the project lock and removes
/// the staging folder, which happens when the control connection ends for any reason.
pub struct Lease {
    pub token: String,
    project: String,
    pull: bool,
    excludes: Vec<String>,
    before: Manifest,
    stage: PathBuf,
    _lock: File,
}

impl Drop for Lease {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.stage);
    }
}

pub fn begin(
    root: &Path,
    id: &str,
    excludes: Vec<String>,
    pull: bool,
) -> anyhow::Result<(Lease, Snapshot)> {
    let (paths, metadata) = load(root, id)?;
    let lock = acquire(root, &paths, &metadata, Blocking::Runs)?;
    let state = paths.sync_state();
    sync::recover(&paths.source, &state)?;
    let rules = Rules::new(&paths.source, &excludes)?;
    let baseline = state.baseline()?;
    let before = source::scan(&paths.source, &rules, &baseline, Some(&paths.hashes()))?;

    let _ = fs::remove_dir_all(paths.staging());
    storage::private_dir(&paths.staging())?;
    let token = storage::new_id();
    let stage = paths.staging().join(&token);
    storage::private_dir(&stage)?;

    let transfer_path = match pull {
        true => &paths.source,
        false => &stage,
    };
    let snapshot = Snapshot {
        token: Some(token.clone()),
        manifest: before.clone(),
        baseline,
        transfer_path: transfer_path
            .to_str()
            .context("Agent storage path is not UTF 8")?
            .to_string(),
    };
    let lease = Lease {
        token,
        project: id.to_string(),
        pull,
        excludes,
        before,
        stage,
        _lock: lock,
    };
    Ok((lease, snapshot))
}

pub fn inspect(root: &Path, id: &str, excludes: &[String]) -> anyhow::Result<Snapshot> {
    let (paths, metadata) = load(root, id)?;
    let state = paths.sync_state();
    ensure!(
        !state.interrupted(),
        "An interrupted sync of {} needs recovery. Run slingshot sync to recover it",
        metadata.name
    );
    let rules = Rules::new(&paths.source, excludes)?;
    let baseline = state.baseline()?;
    let manifest = source::scan(&paths.source, &rules, &baseline, Some(&paths.hashes()))?;
    Ok(Snapshot {
        token: None,
        manifest,
        baseline,
        transfer_path: String::new(),
    })
}

/// Complete a transfer. A push applies staged files after checking them against the
/// Client's manifest. A pull records only the paths where both copies now agree.
/// The Client's claims are never trusted without checking.
/// Apply a finished transfer. `overwrite` lists the conflicts the owner chose to settle in
/// the Client's favor; any other conflict refuses the whole sync.
pub fn finish(
    root: &Path,
    lease: Lease,
    token: &str,
    manifest: Manifest,
    overwrite: &[String],
) -> anyhow::Result<usize> {
    ensure!(
        lease.token == token,
        "Sync token does not match this transfer"
    );
    let (paths, _) = load(root, &lease.project)?;
    let state = paths.sync_state();
    let rules = Rules::new(&paths.source, &lease.excludes)?;
    let baseline = state.baseline()?;
    let current = source::scan(&paths.source, &rules, &baseline, Some(&paths.hashes()))?;
    ensure!(
        current == lease.before,
        "Agent source changed during the transfer. Run the sync again"
    );

    for name in manifest.keys() {
        storage::relative(name)?;
        ensure!(
            !rules.excluded(name, |f| manifest.contains_key(f)),
            "The Client sent an excluded path: {name}"
        );
    }

    if lease.pull {
        let agreed = sync::agreed(&baseline, &lease.before, &manifest);
        let changed = agreed
            .iter()
            .filter(|(k, v)| baseline.get(*k) != Some(v))
            .count()
            + baseline.keys().filter(|k| !agreed.contains_key(*k)).count();
        storage::write_json(&state.baseline_file(), &agreed)?;
        return Ok(changed);
    }

    let plan = sync::overwrite(
        sync::plan(&baseline, &manifest, &lease.before),
        &manifest,
        overwrite,
    );
    if !plan.conflicts.is_empty() {
        bail!(
            "Both machines changed these paths:\n{}",
            plan.conflicts.join("\n")
        );
    }
    let applied_manifest = sync::merge(&lease.before, &plan.changes, &manifest);
    source::check_links(&applied_manifest, &rules)?;
    sync::apply(
        &paths.source,
        &lease.stage,
        &state,
        &plan,
        &lease.before,
        &manifest,
        token,
    )
}

/// Prepare build output folders and return the environment for work in this project.
pub fn prepare_artifacts(paths: &Paths) -> anyhow::Result<Vec<(String, String)>> {
    let project = stack::Project {
        root: paths.source.clone(),
        stacks: stack::stacks_in(&paths.source),
    };
    let layout = Layout {
        source: paths.source.clone(),
        artifacts: paths.artifacts.clone(),
    };
    let rules = artifacts::rules(&project, &layout);
    storage::private_dir(&paths.artifacts)?;
    for (key, value) in artifacts::variables(&rules) {
        fs::create_dir_all(&value).with_context(|| format!("Could not prepare {key}"))?;
    }
    for (name, target) in artifacts::redirects(&rules) {
        fs::create_dir_all(target)?;
        let link = paths.source.join(name);
        match fs::symlink_metadata(&link) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                std::os::unix::fs::symlink(target, &link)?
            }
            Err(e) => return Err(e.into()),
            Ok(_) => {}
        }
    }
    Ok(artifacts::variables(&rules))
}

/// The file name a stored environment file gets: its target in hex, so a nested target such
/// as `api/.env` becomes one flat name that cannot leave the environment folder.
fn stored_name(target: &str) -> String {
    target.bytes().map(|b| format!("{b:02x}")).collect()
}

pub fn env_add(
    root: &Path,
    id: &str,
    target: &str,
    contents: &[u8],
    replace: bool,
) -> anyhow::Result<()> {
    ensure!(
        contents.len() <= MAX_ENVIRONMENT_FILE,
        "Environment files are limited to 1 MiB"
    );
    source::environment_target(target)?;
    let (paths, metadata) = load(root, id)?;
    let _lock = acquire(root, &paths, &metadata, Blocking::Runs)?;
    ensure_ready(&paths, &metadata)?;
    storage::private_dir(&paths.environment)?;

    let stored = paths.environment.join(stored_name(target));
    let mut names: Vec<String> = storage::read_json(&paths.names())?;
    if names.iter().any(|n| n == target) && !replace {
        bail!("{target} is already set on the Agent. Pass --replace to replace it");
    }
    storage::create_parents(&paths.source, target)?;
    let link = storage::safe_path(&paths.source, target)?;
    let linked = match fs::symlink_metadata(&link) {
        Ok(meta) => {
            ensure!(
                meta.file_type().is_symlink() && fs::read_link(&link)? == stored,
                "A file Slingshot does not manage already exists at {target} on the Agent"
            );
            true
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(e) => return Err(e.into()),
    };
    storage::write_bytes(&stored, contents)?;
    if !linked {
        std::os::unix::fs::symlink(&stored, &link)?;
    }
    if !names.iter().any(|n| n == target) {
        names.push(target.to_string());
        names.sort();
        storage::write_json(&paths.names(), &names)?;
    }
    Ok(())
}

pub fn env_list(root: &Path, id: &str) -> anyhow::Result<Vec<String>> {
    let (paths, _) = load(root, id)?;
    storage::read_json(&paths.names())
}

pub fn env_remove(root: &Path, id: &str, target: &str) -> anyhow::Result<()> {
    source::environment_target(target)?;
    let (paths, metadata) = load(root, id)?;
    let _lock = acquire(root, &paths, &metadata, Blocking::Runs)?;
    let names: Vec<String> = storage::read_json(&paths.names())?;
    ensure!(
        names.iter().any(|n| n == target),
        "No environment file is set at {target}"
    );
    remove_environment(&paths, target)
}

/// Remove the link only when it is still Slingshot's own, then the stored copy.
fn remove_environment(paths: &Paths, target: &str) -> anyhow::Result<()> {
    let stored = paths.environment.join(stored_name(target));
    if let Ok(link) = storage::safe_path(&paths.source, target)
        && let Ok(meta) = fs::symlink_metadata(&link)
        && meta.file_type().is_symlink()
        && fs::read_link(&link).is_ok_and(|t| t == stored)
    {
        fs::remove_file(&link)?;
    }
    match fs::remove_file(&stored) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
        _ => {}
    }
    let mut names: Vec<String> = storage::read_json(&paths.names())?;
    names.retain(|n| n != target);
    storage::write_json(&paths.names(), &names)
}

/// Remove environment files for the projects one Client registered with this Agent.
/// Every project must be idle first. Source copies and backups are preserved.
pub fn unlink(root: &Path, ids: &[String]) -> anyhow::Result<usize> {
    let mut held = Vec::new();
    for id in ids {
        let paths = Paths::new(root, id)?;
        let metadata: Option<Metadata> = storage::read_json(&paths.metadata())?;
        let Some(metadata) = metadata else {
            continue;
        };
        let lock = acquire(root, &paths, &metadata, Blocking::AllJobs)?;
        held.push((paths, lock));
    }
    let mut removed = 0;
    for (paths, _lock) in &held {
        let names: Vec<String> = storage::read_json(&paths.names())?;
        for name in names {
            remove_environment(paths, &name)?;
            removed += 1;
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::Root;
    use std::os::unix::fs::PermissionsExt;

    fn active(kind: JobKind) -> Job {
        crate::jobs::new_job(kind, Some(("p", "app")), "work".into())
    }

    #[test]
    fn a_session_blocks_only_unlinking_while_a_run_blocks_everything() {
        let session = [active(JobKind::Session)];
        assert!(blocker(&session, Blocking::Runs).is_none());
        assert!(blocker(&session, Blocking::AllJobs).is_some());
        let both = [active(JobKind::Session), active(JobKind::Run)];
        assert_eq!(
            blocker(&both, Blocking::Runs).map(|job| job.kind),
            Some(JobKind::Run)
        );
        assert!(blocker(&[], Blocking::AllJobs).is_none());
    }

    fn project(root: &Path) -> String {
        let id = storage::new_id();
        open(
            root,
            &ProjectRef {
                id: id.clone(),
                name: "app".into(),
                client: "laptop".into(),
            },
        )
        .unwrap();
        id
    }

    fn initialize(root: &Path, id: &str) -> Paths {
        let paths = Paths::new(root, id).unwrap();
        storage::write_json(&paths.sync_state().baseline_file(), &Manifest::new()).unwrap();
        paths
    }

    #[test]
    fn projects_keep_data_in_separate_private_folders() {
        let root = Root::new();
        let a = project(&root.0);
        let b = project(&root.0);
        assert_ne!(
            Paths::new(&root.0, &a).unwrap().source,
            Paths::new(&root.0, &b).unwrap().source
        );
        let paths = Paths::new(&root.0, &a).unwrap();
        for dir in [
            &paths.source,
            &paths.artifacts,
            &paths.environment,
            &paths.state,
        ] {
            assert_eq!(
                fs::metadata(dir).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
        assert!(Paths::new(&root.0, "../../etc").is_err());
    }

    #[test]
    fn a_lease_blocks_other_work_until_dropped() {
        let root = Root::new();
        let id = project(&root.0);
        let (lease, snapshot) = begin(&root.0, &id, Vec::new(), false).unwrap();
        assert!(Path::new(&snapshot.transfer_path).is_dir());
        assert!(begin(&root.0, &id, Vec::new(), false).is_err());
        let stage = PathBuf::from(&snapshot.transfer_path);
        drop(lease);
        assert!(!stage.exists());
        assert!(begin(&root.0, &id, Vec::new(), false).is_ok());
    }

    #[test]
    fn a_push_installs_verified_files_and_rejects_excluded_paths() {
        let root = Root::new();
        let id = project(&root.0);
        let (lease, snapshot) = begin(&root.0, &id, Vec::new(), false).unwrap();
        let stage = PathBuf::from(&snapshot.transfer_path);
        fs::create_dir_all(stage.join("src")).unwrap();
        fs::write(stage.join("src/main.rs"), "fn main() {}").unwrap();
        let client = TempDirLike::with(&[("src/main.rs", "fn main() {}")]);
        let manifest = source::scan(
            &client.0,
            &Rules::new(&client.0, &[]).unwrap(),
            &Manifest::new(),
            None,
        )
        .unwrap();
        let token = lease.token.clone();
        assert_eq!(
            finish(&root.0, lease, &token, manifest.clone(), &[]).unwrap(),
            1
        );
        let paths = Paths::new(&root.0, &id).unwrap();
        assert_eq!(
            fs::read_to_string(paths.source.join("src/main.rs")).unwrap(),
            "fn main() {}"
        );
        assert_eq!(paths.sync_state().baseline().unwrap(), manifest);

        let (lease, _) = begin(&root.0, &id, Vec::new(), false).unwrap();
        let mut sneaky = manifest.clone();
        sneaky.insert(".env".into(), manifest["src/main.rs"].clone());
        let token = lease.token.clone();
        let error = finish(&root.0, lease, &token, sneaky, &[])
            .unwrap_err()
            .to_string();
        assert!(error.contains("excluded"), "{error}");
        assert!(!paths.source.join(".env").exists());
    }

    #[test]
    fn a_push_refuses_links_that_leave_the_project() {
        let root = Root::new();
        let id = project(&root.0);
        let (lease, _) = begin(&root.0, &id, Vec::new(), false).unwrap();
        let mut manifest = Manifest::new();
        manifest.insert(
            "escape".into(),
            source::Entry {
                hash: "x".into(),
                executable: false,
                link: Some("../../../../etc/passwd".into()),
            },
        );
        let token = lease.token.clone();
        assert!(finish(&root.0, lease, &token, manifest, &[]).is_err());
        assert!(
            fs::symlink_metadata(Paths::new(&root.0, &id).unwrap().source.join("escape")).is_err()
        );
    }

    #[test]
    fn a_pull_records_only_agreed_paths() {
        let root = Root::new();
        let id = project(&root.0);
        let paths = initialize(&root.0, &id);
        fs::write(paths.source.join("agent.txt"), "from agent").unwrap();
        let (lease, snapshot) = begin(&root.0, &id, Vec::new(), true).unwrap();
        assert_eq!(snapshot.transfer_path, paths.source.to_str().unwrap());
        let token = lease.token.clone();
        finish(&root.0, lease, &token, Manifest::new(), &[]).unwrap();
        assert!(paths.sync_state().baseline().unwrap().is_empty());
        let (lease, snapshot) = begin(&root.0, &id, Vec::new(), true).unwrap();
        let token = lease.token.clone();
        finish(&root.0, lease, &token, snapshot.manifest.clone(), &[]).unwrap();
        assert_eq!(paths.sync_state().baseline().unwrap(), snapshot.manifest);
    }

    #[test]
    fn environment_files_are_private_managed_and_never_in_manifests() {
        let root = Root::new();
        let id = project(&root.0);
        assert!(env_add(&root.0, &id, ".env", b"SECRET=1", false).is_err());
        let paths = initialize(&root.0, &id);
        env_add(&root.0, &id, "api/.env.local", b"SECRET=1", false).unwrap();
        let link = paths.source.join("api/.env.local");
        assert_eq!(fs::read_to_string(&link).unwrap(), "SECRET=1");
        let stored = fs::read_link(&link).unwrap();
        assert!(stored.starts_with(&paths.environment));
        assert_eq!(
            fs::metadata(&stored).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(env_list(&root.0, &id).unwrap(), ["api/.env.local"]);
        let manifest = source::scan(
            &paths.source,
            &Rules::new(&paths.source, &[]).unwrap(),
            &Manifest::new(),
            None,
        )
        .unwrap();
        assert!(manifest.is_empty(), "{manifest:?}");

        assert!(env_add(&root.0, &id, "api/.env.local", b"SECRET=2", false).is_err());
        env_add(&root.0, &id, "api/.env.local", b"SECRET=2", true).unwrap();
        assert_eq!(fs::read_to_string(&link).unwrap(), "SECRET=2");
        assert_eq!(fs::read_dir(&paths.environment).unwrap().count(), 2);

        env_remove(&root.0, &id, "api/.env.local").unwrap();
        assert!(fs::symlink_metadata(&link).is_err());
        assert!(!stored.exists());
        assert!(env_list(&root.0, &id).unwrap().is_empty());
    }

    #[test]
    fn environment_add_refuses_unmanaged_files_and_symlinked_parents() {
        let root = Root::new();
        let id = project(&root.0);
        let paths = initialize(&root.0, &id);
        fs::write(paths.source.join(".env"), "not ours").unwrap();
        assert!(env_add(&root.0, &id, ".env", b"x", false).is_err());
        assert_eq!(
            fs::read_to_string(paths.source.join(".env")).unwrap(),
            "not ours"
        );
        let outside = Root::new();
        std::os::unix::fs::symlink(&outside.0, paths.source.join("linked")).unwrap();
        assert!(env_add(&root.0, &id, "linked/.env", b"x", false).is_err());
        assert!(!outside.0.join(".env").exists());
    }

    #[test]
    fn unlink_removes_environment_files_but_keeps_source() {
        let root = Root::new();
        let id = project(&root.0);
        let other = project(&root.0);
        let paths = initialize(&root.0, &id);
        initialize(&root.0, &other);
        fs::write(paths.source.join("main.rs"), "code").unwrap();
        env_add(&root.0, &id, ".env", b"x", false).unwrap();
        env_add(&root.0, &other, ".env", b"y", false).unwrap();
        assert_eq!(unlink(&root.0, std::slice::from_ref(&id)).unwrap(), 1);
        assert!(paths.source.join("main.rs").exists());
        assert!(env_list(&root.0, &id).unwrap().is_empty());
        assert_eq!(env_list(&root.0, &other).unwrap(), [".env"]);
    }

    struct TempDirLike(PathBuf);

    impl TempDirLike {
        fn with(files: &[(&str, &str)]) -> TempDirLike {
            let path = std::env::temp_dir().join(format!("slingshot-client-{}", storage::new_id()));
            for (name, body) in files {
                let file = path.join(name);
                fs::create_dir_all(file.parent().unwrap()).unwrap();
                fs::write(file, body).unwrap();
            }
            TempDirLike(path)
        }
    }

    impl Drop for TempDirLike {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}
