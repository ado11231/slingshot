//! Starting the Agent at boot. Wraps the system's own service manager: a systemd user
//! service on Linux, with linger so it starts before anyone logs in, and a launchd agent on
//! macOS, which starts at login. Either one runs `slingshot internal-daemon`.

use anyhow::{Context, bail};
use slingshot_core::presentation::{self, home_path};
use slingshot_core::storage;
use std::path::{Path, PathBuf};
use std::process::Command;

const UNIT: &str = "slingshot.service";
const LABEL: &str = "dev.slingshot.agent";

/// Saved when the person at `slingshot start` says no, so the question is asked only once.
const DECLINED: &str = "boot-declined";

/// What starting at boot writes and runs on this system.
pub struct Plan {
    pub file: PathBuf,
    contents: String,
    commands: Vec<Vec<String>>,
    /// Lets a Linux user service start before anyone logs in. `None` when already on.
    linger: Option<Vec<String>>,
    /// "at boot" or "at login", for the question and the result.
    pub when: &'static str,
    logs: String,
}

/// The service file this system uses, whether or not it exists.
pub fn file() -> anyhow::Result<PathBuf> {
    let dirs = directories::BaseDirs::new().context("Could not find your home directory")?;
    Ok(match cfg!(target_os = "macos") {
        true => dirs
            .home_dir()
            .join("Library/LaunchAgents")
            .join(format!("{LABEL}.plist")),
        false => dirs.config_dir().join("systemd/user").join(UNIT),
    })
}

pub fn installed() -> anyhow::Result<bool> {
    Ok(file()?.exists())
}

pub fn declined(root: &Path) -> bool {
    root.join(DECLINED).exists()
}

pub fn decline(root: &Path) -> anyhow::Result<()> {
    storage::write_bytes(&root.join(DECLINED), b"")
}

/// Work out the file and commands for the Agent `name`. The service keeps this shell's PATH,
/// because tools such as Homebrew's tmux are not on the short PATH a service starts with.
pub fn plan(root: &Path, name: &str, user: &str) -> anyhow::Result<Plan> {
    let program = std::env::current_exe().context("Could not find the slingshot program")?;
    let program = program
        .to_str()
        .context("The slingshot program's path is not valid text. Move it to a plain path")?;
    let path = std::env::var("PATH").unwrap_or_default();
    let file = file()?;
    if cfg!(target_os = "macos") {
        let log = root.join("daemon.log");
        return Ok(Plan {
            contents: launchd(program, name, &path, &log.to_string_lossy()),
            commands: vec![words(&[
                "launchctl",
                "bootstrap",
                &format!("gui/{}", user_id()),
                &file.to_string_lossy(),
            ])],
            linger: None,
            when: "at login",
            logs: home_path(&log),
            file,
        });
    }
    if !cfg!(target_os = "linux") {
        bail!("Starting at boot works on Linux and macOS. Run slingshot start by hand here");
    }
    Ok(Plan {
        contents: systemd(program, name, &path)?,
        commands: vec![
            words(&["systemctl", "--user", "daemon-reload"]),
            words(&["systemctl", "--user", "enable", "--now", UNIT]),
        ],
        linger: (!lingering(user)).then(|| words(&["loginctl", "enable-linger", user])),
        when: "at boot",
        logs: format!("journalctl --user -u {UNIT}"),
        file,
    })
}

/// The commands that stop the service and take it out of the service manager.
fn removal() -> Vec<String> {
    match cfg!(target_os = "macos") {
        true => words(&[
            "launchctl",
            "bootout",
            &format!("gui/{}/{LABEL}", user_id()),
        ]),
        false => words(&["systemctl", "--user", "disable", "--now", UNIT]),
    }
}

/// List what installing writes and runs, before asking.
pub fn show(plan: &Plan) {
    presentation::detail("Writes", home_path(&plan.file));
    for command in plan.commands.iter().chain(&plan.linger) {
        presentation::detail("Runs", shell_words::join(command));
    }
}

/// Write the service file and start the service. A refused linger only means the service
/// waits for a login, so it is a warning with the fix rather than a failure.
pub fn install(plan: &Plan) -> anyhow::Result<()> {
    let folder = plan
        .file
        .parent()
        .context("The service file has no folder")?;
    std::fs::create_dir_all(folder)
        .with_context(|| format!("Could not create {}", folder.display()))?;
    storage::write_bytes(&plan.file, plan.contents.as_bytes())?;
    for command in &plan.commands {
        run(command)?;
    }
    presentation::success(format!("Slingshot starts {}", plan.when));
    presentation::detail("Logs", &plan.logs);
    if let Some(linger) = &plan.linger
        && run(linger).is_err()
    {
        presentation::warning(format!(
            "Slingshot starts when you log in, not at boot. To start it at boot, run sudo {}",
            shell_words::join(linger)
        ));
    }
    Ok(())
}

/// `slingshot start --remove`: show what will happen, ask, then stop the service and delete
/// its file. Linger stays on, because other services of this account may rely on it.
pub async fn remove() -> anyhow::Result<i32> {
    let file = file()?;
    if !file.exists() {
        presentation::success("Slingshot does not start by itself");
        return Ok(0);
    }
    let command = removal();
    eprintln!("\n  This stops Slingshot and stops it starting by itself:");
    presentation::detail("Runs", shell_words::join(&command));
    presentation::detail("Deletes", home_path(&file));
    anyhow::ensure!(
        crate::interactive(),
        "Run slingshot start --remove in a terminal to confirm"
    );
    if !presentation::confirm("Remove the boot service?".to_string()).await? {
        return Ok(0);
    }
    remove_service()?;
    presentation::success("Slingshot no longer starts by itself");
    Ok(0)
}

/// Stop the service and delete its file, without asking. Callers ask first.
pub fn remove_service() -> anyhow::Result<()> {
    let file = file()?;
    if let Err(error) = run(&removal()) {
        presentation::warning(format!("{error:#}"));
    }
    std::fs::remove_file(&file)
        .with_context(|| format!("Could not delete {}. Delete it by hand", file.display()))
}

fn run(command: &[String]) -> anyhow::Result<()> {
    let shown = shell_words::join(command);
    let output = Command::new(&command[0])
        .args(&command[1..])
        .output()
        .with_context(|| format!("Could not start {}", command[0]))?;
    if !output.status.success() {
        bail!(
            "{shown} failed: {}. Fix that, then run slingshot start --boot",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

fn lingering(user: &str) -> bool {
    Command::new("loginctl")
        .args(["show-user", user, "--property=Linger", "--value"])
        .output()
        .is_ok_and(|output| String::from_utf8_lossy(&output.stdout).trim() == "yes")
}

/// The same call `service` makes to check the account on its socket.
fn user_id() -> u32 {
    unsafe { libc::getuid() }
}

fn words(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|part| part.to_string()).collect()
}

fn systemd(program: &str, name: &str, path: &str) -> anyhow::Result<String> {
    Ok(format!(
        "[Unit]\n\
         Description=Slingshot Agent\n\
         \n\
         [Service]\n\
         ExecStart={} internal-daemon --name {}\n\
         Environment={}\n\
         Restart=on-failure\n\
         RestartSec=5\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n",
        systemd_word(program, true)?,
        systemd_word(name, true)?,
        systemd_word(&format!("PATH={path}"), false)?,
    ))
}

/// Quote one value for a unit file. `%` starts a systemd specifier everywhere, and `$` a
/// variable only in command lines.
fn systemd_word(value: &str, command: bool) -> anyhow::Result<String> {
    if value.contains(['\n', '\r']) {
        bail!("A line break cannot go in a systemd unit. Rename the Agent or move slingshot");
    }
    let mut quoted = value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('%', "%%");
    if command {
        quoted = quoted.replace('$', "$$");
    }
    Ok(format!("\"{quoted}\""))
}

fn launchd(program: &str, name: &str, path: &str, log: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{}</string>
        <string>internal-daemon</string>
        <string>--name</string>
        <string>{}</string>
    </array>
    <key>EnvironmentVariables</key>
    <dict>
        <key>PATH</key>
        <string>{}</string>
    </dict>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <dict>
        <key>SuccessfulExit</key>
        <false/>
    </dict>
    <key>StandardErrorPath</key>
    <string>{}</string>
</dict>
</plist>
"#,
        xml(program),
        xml(name),
        xml(path),
        xml(log),
    )
}

fn xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_values_are_quoted_so_systemd_reads_them_back_unchanged() {
        assert_eq!(
            systemd_word(r#"/opt/my "tools"\slingshot"#, true).unwrap(),
            r#""/opt/my \"tools\"\\slingshot""#
        );
        assert_eq!(
            systemd_word("box%h$HOME", true).unwrap(),
            r#""box%%h$$HOME""#
        );
        assert_eq!(systemd_word("PATH=/a$b", false).unwrap(), r#""PATH=/a$b""#);
        assert!(systemd_word("two\nlines", true).is_err());
    }

    #[test]
    fn the_unit_runs_the_daemon_under_the_chosen_name_and_restarts_it() {
        let unit = systemd("/usr/bin/slingshot", "archbox", "/usr/bin:/bin").unwrap();
        assert!(
            unit.contains(r#"ExecStart="/usr/bin/slingshot" internal-daemon --name "archbox""#)
        );
        assert!(unit.contains(r#"Environment="PATH=/usr/bin:/bin""#));
        assert!(unit.contains("Restart=on-failure"));
        assert!(unit.contains("WantedBy=default.target"));
    }

    #[test]
    fn the_launch_agent_escapes_its_values_and_restarts_only_after_a_failure() {
        let plist = launchd("/opt/a&b/slingshot", "<box>", "/usr/bin", "/tmp/daemon.log");
        assert!(plist.contains("<string>/opt/a&amp;b/slingshot</string>"));
        assert!(plist.contains("<string>&lt;box&gt;</string>"));
        assert!(plist.contains("<key>SuccessfulExit</key>\n        <false/>"));
        assert!(plist.contains("<string>/tmp/daemon.log</string>"));
    }

    #[test]
    fn a_no_is_remembered() {
        let root = crate::testing::Root::new();
        assert!(!declined(&root.0));
        decline(&root.0).unwrap();
        assert!(declined(&root.0));
    }
}
