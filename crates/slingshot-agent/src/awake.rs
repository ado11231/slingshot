//! Keeps the Agent from sleeping on its own while `slingshot start` runs, because a sleeping
//! Agent cannot be reached from anywhere. Wraps the system's own tool, and ties its lock to
//! this process so the lock ends with `start`, even when `start` is killed outright.

use std::process::{Child, ChildStdin, Command, Stdio};
use std::time::Duration;

/// Holds the lock until dropped.
pub struct Awake {
    child: Child,
    _lifeline: Option<ChildStdin>,
}

impl Drop for Awake {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A tool that keeps the system awake. `lifeline` means it runs a command that must read
/// from this process to stay alive, which is how it learns that this process has ended.
struct Tool {
    command: Command,
    lifeline: bool,
}

/// Take the lock with whichever tool this system has. `None` means nothing could hold it,
/// and the Agent may sleep.
pub fn hold() -> Option<Awake> {
    [systemd_inhibit(), caffeinate()]
        .into_iter()
        .find_map(start)
}

/// logind's inhibitor lives as long as the command it runs, here a `cat` reading from us.
/// It never asks for a password: polkit wants one outside a session at the machine, such
/// as over ssh, and a prompt there is a surprise nobody asked for.
fn systemd_inhibit() -> Tool {
    let mut command = Command::new("systemd-inhibit");
    command.args([
        "--no-ask-password",
        "--what=idle:sleep",
        "--who=Slingshot",
        "--why=Reachable by paired Clients",
        "--mode=block",
        "cat",
    ]);
    Tool {
        command,
        lifeline: true,
    }
}

/// caffeinate watches this process and exits with it.
fn caffeinate() -> Tool {
    let mut command = Command::new("caffeinate");
    command.args(["-i", "-w", &std::process::id().to_string()]);
    Tool {
        command,
        lifeline: false,
    }
}

/// A tool that is missing, or was refused the lock, exits at once. One still running after
/// a moment holds it.
fn start(mut tool: Tool) -> Option<Awake> {
    let stdin = match tool.lifeline {
        true => Stdio::piped(),
        false => Stdio::null(),
    };
    let mut child = tool
        .command
        .stdin(stdin)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    std::thread::sleep(Duration::from_millis(300));
    if !matches!(child.try_wait(), Ok(None)) {
        return None;
    }
    let lifeline = child.stdin.take();
    Some(Awake {
        child,
        _lifeline: lifeline,
    })
}
