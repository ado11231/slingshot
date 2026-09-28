//! `slingshot menubar`: build the Slingshot menu bar app if needed, open it, and tell it
//! where this program is. The app starts `slingshot internal-watch` for its numbers, and an
//! app opened from Finder or at login does not get the shell's PATH, so it cannot find the
//! program alone.

/// The app's bundle identifier, which is also where its settings live.
#[cfg(target_os = "macos")]
const BUNDLE_ID: &str = "dev.slingshot.menubar";

/// The app's source, built into this program so `slingshot menubar` can build the app even
/// after the Slingshot source folder is gone.
#[cfg(target_os = "macos")]
const SOURCES: &[(&str, &str)] = &[
    (
        "Package.swift",
        include_str!("../../../../mac/menubar/Package.swift"),
    ),
    (
        "Info.plist",
        include_str!("../../../../mac/menubar/Info.plist"),
    ),
    (
        "Sources/App.swift",
        include_str!("../../../../mac/menubar/Sources/App.swift"),
    ),
    (
        "Sources/Bars.swift",
        include_str!("../../../../mac/menubar/Sources/Bars.swift"),
    ),
    (
        "Sources/LoginItem.swift",
        include_str!("../../../../mac/menubar/Sources/LoginItem.swift"),
    ),
    (
        "Sources/Model.swift",
        include_str!("../../../../mac/menubar/Sources/Model.swift"),
    ),
    (
        "Sources/Notifier.swift",
        include_str!("../../../../mac/menubar/Sources/Notifier.swift"),
    ),
    (
        "Sources/PopoverView.swift",
        include_str!("../../../../mac/menubar/Sources/PopoverView.swift"),
    ),
    (
        "Sources/Watcher.swift",
        include_str!("../../../../mac/menubar/Sources/Watcher.swift"),
    ),
];

/// Where an installed app records which source it was built from.
#[cfg(target_os = "macos")]
const STAMP: &str = "Contents/Resources/slingshot-source";

/// What `link` does about the menu bar app when it finishes.
#[cfg(target_os = "macos")]
#[derive(Debug, PartialEq)]
enum Offer {
    /// The app is installed, so restart it to show the new Agent.
    Restart,
    /// Explain what the app is and ask once.
    Ask,
    /// Asking cannot work here, so say how to add it later.
    Later(&'static str),
}

#[cfg(target_os = "macos")]
fn offer_for(installed: bool, swift: bool, terminal: bool) -> Offer {
    match (installed, swift, terminal) {
        (true, _, _) => Offer::Restart,
        (false, false, _) => Offer::Later(
            "Install the Xcode command line tools with xcode-select --install, then run slingshot menubar",
        ),
        (false, true, false) => Offer::Later("Run slingshot menubar in a terminal to add it"),
        (false, true, true) => Offer::Ask,
    }
}

/// The end of `link`: keep the menu bar app showing the Agent, installing it after one yes.
#[cfg(not(target_os = "macos"))]
pub async fn offer(_name: &str) -> anyhow::Result<()> {
    Ok(())
}

#[cfg(target_os = "macos")]
pub async fn offer(name: &str) -> anyhow::Result<()> {
    use slingshot_core::presentation::Style;
    use slingshot_core::telemetry;

    let style = Style::stderr();
    match offer_for(
        find_app().is_some(),
        telemetry::is_installed("swift"),
        crate::ssh::wants_terminal(),
    ) {
        Offer::Restart => {
            show(name, None).await?;
        }
        Offer::Later(how) => {
            eprintln!(
                "  {}",
                style.dim(format!("Keep {name} in your menu bar. {how}"))
            );
        }
        Offer::Ask => {
            eprintln!(
                "\n  The menu bar app shows {name}'s CPU, RAM, and GPU, and notifies you when a run finishes."
            );
            eprintln!(
                "  {}",
                style.dim(
                    "It is built with Swift into ~/Applications/Slingshot.app and starts at login."
                )
            );
            if super::tools::confirm("Add it to your menu bar now?".to_string()).await? {
                show(name, None).await?;
            } else {
                eprintln!(
                    "  {}",
                    style.dim("Run slingshot menubar any time to do this later")
                );
            }
        }
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub async fn menubar(_agent: Option<String>, _remove: bool) -> anyhow::Result<i32> {
    anyhow::bail!(
        "The menu bar app is macOS only for now. slingshot health --watch shows the same numbers in a terminal"
    )
}

#[cfg(target_os = "macos")]
pub async fn menubar(agent: Option<String>, remove: bool) -> anyhow::Result<i32> {
    use slingshot_core::config::Config;
    use slingshot_core::presentation::{self, home_path};

    if remove {
        return uninstall().await;
    }
    let config = Config::load()?;
    let target = config.resolve(agent.as_deref())?;
    let app = show(&target.name, agent.as_deref()).await?;
    presentation::detail("App", home_path(&app));
    presentation::detail("Starts", "at login, remove with slingshot menubar --remove");
    Ok(0)
}

/// Build the app if its source changed, tell it where this program is, and start it again,
/// so it always runs the helper from the Slingshot that is installed now.
#[cfg(target_os = "macos")]
async fn show(name: &str, agent: Option<&str>) -> anyhow::Result<std::path::PathBuf> {
    use anyhow::Context;
    use slingshot_core::presentation::{self, home_path};

    let app = match find_app().filter(|app| up_to_date(app)) {
        Some(app) => app,
        None => tokio::task::spawn_blocking(install)
            .await
            .context("Building the menu bar app stopped unexpectedly")??,
    };
    let program = std::env::current_exe()
        .and_then(|path| path.canonicalize())
        .context("Could not find this program's own path")?;

    defaults(&[
        "write",
        BUNDLE_ID,
        "slingshotPath",
        &program.to_string_lossy(),
    ])?;
    match agent {
        Some(agent) => defaults(&["write", BUNDLE_ID, "agent", agent])?,
        None => {
            let _ = defaults(&["delete", BUNDLE_ID, "agent"]);
        }
    }
    tokio::task::spawn_blocking(quit_running_app)
        .await
        .context("Stopping the old menu bar app stopped unexpectedly")?;
    let opened = std::process::Command::new("open")
        .arg(&app)
        .status()
        .context("Could not start open")?;
    anyhow::ensure!(
        opened.success(),
        "Could not open {}. Try opening it from Finder",
        home_path(&app)
    );
    presentation::success(format!("{name} is in your menu bar"));
    Ok(app)
}

/// Take the app out of login items, then delete it, its build folder, and its settings.
/// Only the app itself can leave login items, so it is started once with `--remove`.
#[cfg(target_os = "macos")]
async fn uninstall() -> anyhow::Result<i32> {
    use anyhow::Context;
    use slingshot_core::presentation::{self, home_path};
    use std::fs;

    let Some(app) = find_app() else {
        presentation::success("The menu bar app is not installed");
        return Ok(0);
    };
    let source = crate::project::client_root()?.join("menubar");
    eprintln!("\n  This quits the menu bar app, stops it starting at login, and deletes:");
    presentation::detail("App", home_path(&app));
    if source.exists() {
        presentation::detail("Build", home_path(&source));
    }
    presentation::detail("Settings", BUNDLE_ID);
    anyhow::ensure!(
        crate::ssh::wants_terminal(),
        "Run slingshot menubar --remove in a terminal to confirm"
    );
    if !super::tools::confirm("Remove the menu bar app?".to_string()).await? {
        return Ok(0);
    }

    tokio::task::spawn_blocking(quit_running_app)
        .await
        .context("Stopping the menu bar app stopped unexpectedly")?;
    let left = std::process::Command::new("open")
        .args(["-n", "-W"])
        .arg(&app)
        .args(["--args", "--remove"])
        .status()
        .context("Could not start open")?;
    anyhow::ensure!(
        left.success(),
        "Could not take the app out of login items. Remove Slingshot in System Settings, then General, then Login Items"
    );
    fs::remove_dir_all(&app).with_context(|| {
        format!(
            "Could not delete {}. Move it to the Trash from Finder",
            app.display()
        )
    })?;
    if source.exists() {
        fs::remove_dir_all(&source)
            .with_context(|| format!("Could not delete {}", source.display()))?;
    }
    let _ = defaults(&["delete", BUNDLE_ID]);
    presentation::success("Removed the menu bar app");
    Ok(0)
}

#[cfg(target_os = "macos")]
fn find_app() -> Option<std::path::PathBuf> {
    let home = directories::BaseDirs::new().map(|dirs| dirs.home_dir().join("Applications"));
    home.into_iter()
        .chain(Some(std::path::PathBuf::from("/Applications")))
        .map(|folder| folder.join("Slingshot.app"))
        .find(|app| app.is_dir())
}

/// True when the app was built from the source inside this program.
#[cfg(target_os = "macos")]
fn up_to_date(app: &std::path::Path) -> bool {
    std::fs::read_to_string(app.join(STAMP)).is_ok_and(|stamp| stamp.trim() == stamp_of(SOURCES))
}

#[cfg(target_os = "macos")]
fn stamp_of(sources: &[(&str, &str)]) -> String {
    let mut all = Vec::new();
    for (name, contents) in sources {
        all.extend_from_slice(name.as_bytes());
        all.push(0);
        all.extend_from_slice(contents.as_bytes());
        all.push(0);
    }
    slingshot_core::source::digest(&all)
}

/// Build the app with Swift, sign it for this machine, and install it in ~/Applications in
/// place of an older copy. A stranger needs only the Xcode command line tools for this.
#[cfg(target_os = "macos")]
fn install() -> anyhow::Result<std::path::PathBuf> {
    use anyhow::Context;
    use slingshot_core::{step, telemetry};
    use std::fs;

    anyhow::ensure!(
        telemetry::is_installed("swift"),
        "Swift is needed to build the menu bar app. Install the Xcode command line tools with: xcode-select --install"
    );
    let package = crate::project::client_root()?.join("menubar");
    for (name, contents) in SOURCES {
        let path = package.join(name);
        if let Some(folder) = path.parent() {
            fs::create_dir_all(folder)
                .with_context(|| format!("Could not create {}", folder.display()))?;
        }
        fs::write(&path, contents)
            .with_context(|| format!("Could not write {}", path.display()))?;
    }

    let building = step::start("Building the menu bar app, about a minute the first time");
    let package_path = package.to_string_lossy().to_string();
    let result = swift(&["build", "-c", "release", "--package-path", &package_path])
        .and_then(|_| {
            swift(&[
                "build",
                "-c",
                "release",
                "--package-path",
                &package_path,
                "--show-bin-path",
            ])
        })
        .and_then(|bin| assemble(&package, std::path::Path::new(bin.trim())));
    let built = match result {
        Ok(built) => built,
        Err(error) => {
            building.clear();
            return Err(error);
        }
    };

    quit_running_app();
    let applications = directories::BaseDirs::new()
        .context("Could not find the home folder")?
        .home_dir()
        .join("Applications");
    fs::create_dir_all(&applications)
        .with_context(|| format!("Could not create {}", applications.display()))?;
    let app = applications.join("Slingshot.app");
    if app.exists() {
        fs::remove_dir_all(&app)
            .with_context(|| format!("Could not replace the old {}", app.display()))?;
    }
    fs::rename(&built, &app).with_context(|| format!("Could not install {}", app.display()))?;
    building.done("Built the menu bar app");
    Ok(app)
}

/// Run Swift and return what it printed, or its last lines as the error.
#[cfg(target_os = "macos")]
fn swift(args: &[&str]) -> anyhow::Result<String> {
    use anyhow::Context;

    let output = std::process::Command::new("swift")
        .args(args)
        .output()
        .context("Could not start swift")?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).to_string());
    }
    let printed = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let last: Vec<&str> = printed.lines().rev().take(15).collect();
    let last: Vec<&str> = last.into_iter().rev().collect();
    anyhow::bail!(
        "Could not build the menu bar app. Swift said:\n{}",
        last.join("\n")
    )
}

/// Put the built program, its Info.plist, and the source stamp into an app bundle, then
/// sign it for this machine only, which is all macOS needs to run a locally built app.
#[cfg(target_os = "macos")]
fn assemble(
    package: &std::path::Path,
    bin: &std::path::Path,
) -> anyhow::Result<std::path::PathBuf> {
    use anyhow::Context;
    use std::fs;

    let app = package.join(".build").join("Slingshot.app");
    if app.exists() {
        fs::remove_dir_all(&app).with_context(|| format!("Could not clear {}", app.display()))?;
    }
    let contents = app.join("Contents");
    fs::create_dir_all(contents.join("MacOS"))?;
    fs::create_dir_all(contents.join("Resources"))?;
    fs::copy(
        bin.join("SlingshotMenuBar"),
        contents.join("MacOS").join("Slingshot"),
    )
    .context("Swift finished but the app program was not where it said")?;
    fs::copy(package.join("Info.plist"), contents.join("Info.plist"))?;
    fs::write(app.join(STAMP), stamp_of(SOURCES))?;

    let signed = std::process::Command::new("codesign")
        .args(["--force", "--sign", "-"])
        .arg(&app)
        .stderr(std::process::Stdio::null())
        .status()
        .context("Could not start codesign")?;
    anyhow::ensure!(
        signed.success(),
        "Could not sign the menu bar app. Check that the Xcode command line tools are installed with: xcode-select --install"
    );
    Ok(app)
}

/// Ask a running copy of the app to quit and wait briefly, so the new copy can replace it.
#[cfg(target_os = "macos")]
fn quit_running_app() {
    if !app_running() {
        return;
    }
    let _ = std::process::Command::new("osascript")
        .args(["-e", &format!("quit app id \"{BUNDLE_ID}\"")])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    for _ in 0..10 {
        if !app_running() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
}

#[cfg(target_os = "macos")]
fn app_running() -> bool {
    std::process::Command::new("pgrep")
        .args(["-f", "Slingshot.app/Contents/MacOS/Slingshot"])
        .stdout(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(target_os = "macos")]
fn defaults(args: &[&str]) -> anyhow::Result<()> {
    let status = std::process::Command::new("defaults")
        .args(args)
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|error| anyhow::anyhow!("Could not run defaults: {error}"))?;
    anyhow::ensure!(
        status.success(),
        "Could not save the menu bar app's settings"
    );
    Ok(())
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn every_app_source_file_is_built_into_the_program() {
        let folder =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../mac/menubar/Sources");
        for entry in std::fs::read_dir(folder).unwrap() {
            let name = format!("Sources/{}", entry.unwrap().file_name().to_string_lossy());
            assert!(
                SOURCES.iter().any(|(listed, _)| *listed == name),
                "{name} is missing from SOURCES"
            );
        }
    }

    #[test]
    fn link_restarts_an_installed_app_and_asks_only_when_it_can_build() {
        assert_eq!(offer_for(true, false, false), Offer::Restart);
        assert_eq!(offer_for(false, true, true), Offer::Ask);
        assert!(
            matches!(offer_for(false, false, true), Offer::Later(how) if how.contains("xcode-select --install"))
        );
        assert!(
            matches!(offer_for(false, true, false), Offer::Later(how) if how.contains("slingshot menubar"))
        );
    }

    #[test]
    fn any_change_to_the_source_changes_the_stamp() {
        let before = stamp_of(&[("Sources/App.swift", "a")]);

        assert_eq!(before, stamp_of(&[("Sources/App.swift", "a")]));
        assert_ne!(before, stamp_of(&[("Sources/App.swift", "b")]));
        assert_ne!(before, stamp_of(&[("Sources/Other.swift", "a")]));
    }
}
