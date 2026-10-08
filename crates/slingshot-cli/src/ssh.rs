//! Building and running SSH commands to the Agent.

use crate::route::{self, Route};
use crate::tunnel;
use anyhow::Context;
use shell_words::join;
use slingshot_core::config;
use std::path::PathBuf;
use std::process::Stdio;
use tokio::process::Command;

/// The exit code reported when SSH ends without a remote exit status.
const EXIT_SIGNALLED: i32 = 130;

/// Disable password prompts, keep idle connections alive, and show SSH errors only.
const SSH_OPTIONS: &[&str] = &[
    "BatchMode=yes",
    "ConnectTimeout=10",
    "ServerAliveInterval=30",
    "ServerAliveCountMax=6",
    "LogLevel=ERROR",
];

/// Request a terminal only for interactive use.
/// A terminal lets Ctrl C reach remote work but merges stdout and stderr.
pub fn wants_terminal() -> bool {
    use std::io::IsTerminal;

    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

/// A command to run on the Agent. Arguments are quoted for the remote shell, so no
/// value can ever be read as shell syntax.
pub struct RemoteCommand {
    pub host: String,
    pub user: Option<String>,
    pub port: Option<u16>,
    pub identity_file: Option<PathBuf>,
    pub known_hosts: Option<PathBuf>,
    /// The name the Agent's host keys were learned under, which stays the same whichever
    /// address is dialed.
    pub host_key_alias: Option<String>,
    /// How ssh reaches the Agent when no address answers directly.
    pub proxy: Option<String>,
    /// The socket ssh shares one connection through across calls, used over iroh.
    pub shared: Option<PathBuf>,
    /// Whether to ask for a terminal on the Agent. See `wants_terminal`.
    pub tty: bool,
    /// A port on the Agent to reach at the same port on this machine while the command runs.
    pub forward: Option<u16>,
    /// Send the program's own output to `/dev/null` on the Agent. tmux draws through the
    /// terminal it reads from, so for `attach` this hides only its `[detached]` line.
    pub quiet: bool,
    pub program: String,
    pub args: Vec<String>,
}

impl RemoteCommand {
    /// Build a command aimed at a saved Agent, over whichever of its addresses answers.
    /// Everything ssh needs comes from the config, so nobody has to keep an entry in
    /// `~/.ssh/config` in step with this.
    pub fn to(agent: &config::Agent, program: String, args: Vec<String>) -> RemoteCommand {
        let (host, proxy, shared) = match route::resolve(agent) {
            Route::Direct { host, .. } => (host, None, None),
            Route::Iroh { key } => (
                agent.host.clone(),
                Some(tunnel::proxy_command(&key)),
                tunnel::shared_socket(&key),
            ),
        };
        RemoteCommand {
            host,
            proxy,
            shared,
            user: Some(agent.user.clone()),
            port: agent.port,
            identity_file: agent.identity_file.clone(),
            known_hosts: agent.known_hosts.clone(),
            host_key_alias: Some(agent.host.clone()),
            tty: wants_terminal(),
            forward: None,
            quiet: false,
            program,
            args,
        }
    }

    /// Who to log in as and where, in the form ssh expects.
    fn destination(&self) -> String {
        match &self.user {
            Some(user) => format!("{user}@{}", self.host),
            None => self.host.clone(),
        }
    }

    fn command_line(&self) -> String {
        let line = join(
            std::iter::once(self.program.as_str()).chain(self.args.iter().map(|s| s.as_str())),
        );
        match self.quiet {
            true => format!("{line} >/dev/null"),
            false => line,
        }
    }

    /// SSH options and the destination, without any remote command.
    fn connection_args(&self) -> Vec<String> {
        let mut argv = Vec::new();

        for option in SSH_OPTIONS {
            argv.push("-o".to_string());
            argv.push(option.to_string());
        }

        if let Some(port) = self.port {
            argv.push("-p".to_string());
            argv.push(port.to_string());
        }

        if let Some(key) = &self.identity_file {
            argv.push("-i".to_string());
            argv.push(key.display().to_string());
            argv.push("-o".to_string());
            argv.push("IdentitiesOnly=yes".to_string());
        }

        if let Some(known_hosts) = &self.known_hosts {
            argv.push("-o".to_string());
            argv.push(format!("UserKnownHostsFile=\"{}\"", known_hosts.display()));
            argv.push("-o".to_string());
            argv.push("StrictHostKeyChecking=yes".to_string());
        }

        if let Some(proxy) = &self.proxy {
            argv.push("-o".to_string());
            argv.push(format!("ProxyCommand={proxy}"));
        }

        if let Some(socket) = &self.shared {
            argv.push("-o".to_string());
            argv.push("ControlMaster=auto".to_string());
            argv.push("-o".to_string());
            argv.push(format!("ControlPath=\"{}\"", socket.display()));
            argv.push("-o".to_string());
            argv.push(format!("ControlPersist={}", tunnel::SHARED_FOR_SECONDS));
        }

        if let Some(alias) = &self.host_key_alias {
            argv.push("-o".to_string());
            argv.push(format!("HostKeyAlias={alias}"));
        }

        if self.tty {
            argv.push("-t".to_string());
        }

        if let Some(port) = self.forward {
            argv.push("-o".to_string());
            argv.push("ExitOnForwardFailure=yes".to_string());
            argv.push("-L".to_string());
            argv.push(format!("{port}:localhost:{port}"));
        }

        argv.push(self.destination());
        argv
    }

    /// The full argument list for `ssh`. Options first, then the destination, then
    /// the command, because ssh treats everything after the destination as payload.
    pub fn to_ssh_args(&self) -> Vec<String> {
        let mut argv = self.connection_args();
        argv.push(self.command_line());
        argv
    }

    /// Run with this terminal's input and output attached directly, so bytes, window
    /// size changes, and Ctrl C pass through unchanged. Returns the remote exit code.
    /// SSH's own messages go to a private log instead of the terminal, so a dropped
    /// connection becomes a `Disconnected` error and anything else is shown as before.
    pub async fn interactive(&self) -> anyhow::Result<i32> {
        let log = SshLog::create()?;
        let mut command = Command::new("ssh");
        if let Ok(local) = std::env::var("TERM") {
            command.env("TERM", remote_terminal(&local));
        }
        let mut child = command
            .arg("-E")
            .arg(&log.0)
            .args(self.to_ssh_args())
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
            .context("Could not start ssh. Check that it is installed and on PATH")?;

        let interrupt = tokio::signal::ctrl_c();
        tokio::pin!(interrupt);
        let status = loop {
            tokio::select! {
                status = child.wait() => break status.context("Waiting for ssh failed")?,
                _ = &mut interrupt => {}
            }
        };

        let code = status.code().unwrap_or(EXIT_SIGNALLED);
        let messages = log.read();
        match (code, messages.is_empty()) {
            (EXIT_SSH_FAILED, false) if connection_lost(&messages) => {
                Err(Disconnected::Certain.into())
            }
            (EXIT_SSH_FAILED, true) => Err(Disconnected::Possible.into()),
            _ => {
                eprint!("{messages}");
                Ok(code)
            }
        }
    }
}

impl RemoteCommand {
    /// Whether SSH can reach the Agent right now, without involving the daemon.
    pub async fn reachable(agent: &config::Agent) -> bool {
        let mut probe = RemoteCommand::to(agent, "true".to_string(), Vec::new());
        probe.tty = false;
        let status = Command::new("ssh")
            .args(probe.to_ssh_args())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .status();
        matches!(
            tokio::time::timeout(std::time::Duration::from_secs(15), status).await,
            Ok(Ok(status)) if status.success()
        )
    }
}

/// Terminal types every system can describe. Others, such as `xterm-kitty` or
/// `xterm-ghostty`, are often missing on the Agent, where tmux and full screen programs
/// then refuse to start, so ssh sends `xterm-256color` for them instead.
const UNIVERSAL_TERMINALS: &[&str] = &[
    "xterm-256color",
    "xterm",
    "screen-256color",
    "screen",
    "tmux-256color",
    "tmux",
    "linux",
    "vt100",
    "dumb",
];

fn remote_terminal(local: &str) -> &str {
    match UNIVERSAL_TERMINALS.contains(&local) {
        true => local,
        false => "xterm-256color",
    }
}

/// The exit code SSH uses for its own failures. A remote command can exit with it too,
/// which is why the log decides whether SSH failed.
const EXIT_SSH_FAILED: i32 = 255;

/// SSH ended with its failure code. `Certain` means its messages describe a dropped
/// connection. `Possible` means it printed nothing, which is both how a keepalive
/// timeout ends and how a remote command exiting with 255 ends.
#[derive(Debug, PartialEq)]
pub enum Disconnected {
    Certain,
    Possible,
}

impl std::fmt::Display for Disconnected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Disconnected::Certain => f.write_str("The SSH connection to the Agent was lost"),
            Disconnected::Possible => f.write_str("SSH exited with code 255"),
        }
    }
}

impl std::error::Error for Disconnected {}

/// True when SSH's messages describe an established connection that dropped, as opposed
/// to one that never opened, which keeps its own more specific message.
fn connection_lost(messages: &str) -> bool {
    const NEVER_OPENED: &[&str] = &[
        "connect to host",
        "Could not resolve",
        "kex_exchange_identification",
        "banner exchange",
        "Permission denied",
        "Host key verification failed",
        "REMOTE HOST IDENTIFICATION HAS CHANGED",
    ];
    const DROPPED: &[&str] = &[
        "client_loop",
        "Broken pipe",
        "not responding",
        "closed by remote host",
        "Connection reset",
        "Read from remote host",
        "packet_write_wait",
        "Software caused connection abort",
    ];
    !NEVER_OPENED.iter().any(|marker| messages.contains(marker))
        && DROPPED.iter().any(|marker| messages.contains(marker))
}

/// A private file for SSH's `-E` log, removed when dropped. SSH closes inherited file
/// descriptors at startup, so a pipe cannot be used here.
struct SshLog(PathBuf);

impl SshLog {
    fn create() -> anyhow::Result<SshLog> {
        use std::os::unix::fs::OpenOptionsExt;

        let path = std::env::temp_dir().join(format!(
            "slingshot-ssh-{}.log",
            slingshot_core::storage::new_id()
        ));
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .context("Could not create a temporary file for SSH messages")?;
        Ok(SshLog(path))
    }

    fn read(&self) -> String {
        std::fs::read(&self.0)
            .map(|bytes| String::from_utf8_lossy(&bytes).replace('\r', ""))
            .unwrap_or_default()
    }
}

impl Drop for SshLog {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// The `-e` helper rsync starts: SSH with Slingshot's saved options for one Agent. Running
/// through Slingshot avoids passing file paths inside rsync's own option parsing.
pub fn exec_for_rsync(agent: &config::Agent, args: Vec<String>) -> anyhow::Error {
    use std::os::unix::process::CommandExt;

    let mut rest = args.into_iter().peekable();
    if rest.peek().is_some_and(|arg| arg == "-l") {
        rest.next();
        rest.next();
    }
    rest.next();
    let mut remote = RemoteCommand::to(agent, String::new(), Vec::new());
    remote.tty = false;
    let error = std::process::Command::new("ssh")
        .args(remote.connection_args())
        .args(rest)
        .exec();
    anyhow::Error::from(error).context("Could not start ssh")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn remote(args: &[&str]) -> RemoteCommand {
        RemoteCommand {
            host: "localbox".to_string(),
            user: None,
            port: None,
            identity_file: None,
            known_hosts: None,
            host_key_alias: None,
            proxy: None,
            shared: None,
            tty: false,
            forward: None,
            quiet: false,
            program: "echo".to_string(),
            args: args.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn cmd(args: &[&str]) -> String {
        remote(args).command_line()
    }

    #[test]
    fn quotes_nothing_when_unnecessary() {
        assert_eq!(cmd(&["hello"]), "echo hello");
    }

    /// The redirect is a fixed suffix, so a quiet command still quotes every argument.
    #[test]
    fn a_quiet_command_discards_only_its_own_output() {
        let mut command = remote(&["a b", ">x"]);
        command.quiet = true;
        assert_eq!(command.command_line(), "echo 'a b' '>x' >/dev/null");
    }

    #[test]
    fn quotes_arguments_containing_spaces() {
        assert_eq!(cmd(&["hello world"]), "echo 'hello world'");
    }

    #[test]
    fn handles_embedded_single_quote() {
        assert_eq!(cmd(&["it's"]), r"echo 'it'\''s'");
    }

    #[test]
    fn dollar_sign_stays_literal() {
        assert_eq!(cmd(&["$HOME"]), "echo '$HOME'");
    }

    #[test]
    fn semicolon_stays_literal() {
        assert_eq!(cmd(&["a; whoami"]), "echo 'a; whoami'");
    }

    #[test]
    fn a_program_path_with_a_space_stays_one_word() {
        let mut command = remote(&["internal-control"]);
        command.program = "/Users/me/Application Support/slingshot".to_string();
        assert_eq!(
            command.command_line(),
            "'/Users/me/Application Support/slingshot' internal-control"
        );
    }

    #[test]
    fn the_command_is_the_last_argument() {
        let argv = remote(&["hi"]).to_ssh_args();

        assert_eq!(argv.last().unwrap(), "echo hi");
        assert_eq!(argv[argv.len() - 2], "localbox");
    }

    #[test]
    fn user_port_and_key_reach_ssh() {
        let mut command = remote(&["hi"]);
        command.user = Some("me".to_string());
        command.port = Some(2222);
        command.identity_file = Some(PathBuf::from("/keys/slingshot"));
        command.known_hosts = Some(PathBuf::from("/keys/known_hosts"));

        let argv = command.to_ssh_args();

        assert!(
            argv.contains(&"me@localbox".to_string()),
            "argv was: {argv:?}"
        );
        assert!(
            argv.windows(2).any(|w| w == ["-p", "2222"]),
            "argv was: {argv:?}"
        );
        assert!(
            argv.windows(2).any(|w| w == ["-i", "/keys/slingshot"]),
            "argv was: {argv:?}"
        );
        assert!(
            argv.contains(&"UserKnownHostsFile=\"/keys/known_hosts\"".to_string()),
            "argv was: {argv:?}"
        );
    }

    #[test]
    fn ssh_is_told_to_keep_quiet_about_itself() {
        let argv = remote(&["hi"]).to_ssh_args();

        assert!(
            argv.contains(&"LogLevel=ERROR".to_string()),
            "argv was: {argv:?}"
        );
    }

    #[test]
    fn a_terminal_is_requested_only_when_asked_for() {
        assert!(!remote(&["hi"]).to_ssh_args().contains(&"-t".to_string()));

        let mut interactive = remote(&["hi"]);
        interactive.tty = true;

        let argv = interactive.to_ssh_args();

        assert!(argv.contains(&"-t".to_string()), "argv was: {argv:?}");
        assert_eq!(argv.last().unwrap(), "echo hi");
    }

    #[test]
    fn a_forwarded_port_fails_loudly_when_it_is_taken() {
        assert!(!remote(&["hi"]).to_ssh_args().contains(&"-L".to_string()));

        let mut command = remote(&["hi"]);
        command.forward = Some(1455);

        let argv = command.to_ssh_args();
        let at = argv.iter().position(|arg| arg == "-L").unwrap();

        assert_eq!(argv[at + 1], "1455:localhost:1455");
        assert!(argv.contains(&"ExitOnForwardFailure=yes".to_string()));
        assert_eq!(argv.last().unwrap(), "echo hi");
    }

    #[test]
    fn terminals_the_agent_may_not_know_are_sent_as_xterm_256color() {
        for known in ["xterm-256color", "screen-256color", "tmux-256color", "dumb"] {
            assert_eq!(remote_terminal(known), known);
        }
        for unknown in ["xterm-kitty", "xterm-ghostty", "alacritty", "wezterm", ""] {
            assert_eq!(remote_terminal(unknown), "xterm-256color", "{unknown}");
        }
    }

    /// Quote the known hosts path because SSH splits this option on whitespace.
    #[test]
    fn a_known_hosts_path_with_a_space_stays_one_path() {
        let mut command = remote(&["hi"]);
        command.known_hosts = Some(PathBuf::from("/App Support/known_hosts"));

        let argv = command.to_ssh_args();

        assert!(
            argv.contains(&"UserKnownHostsFile=\"/App Support/known_hosts\"".to_string()),
            "argv was: {argv:?}"
        );
    }

    #[test]
    fn host_keys_are_checked_under_the_pairing_name_whatever_address_is_dialed() {
        let mut command = remote(&["hi"]);
        command.host = "100.67.90.119".to_string();
        command.host_key_alias = Some("192.168.1.20".to_string());

        let argv = command.to_ssh_args();

        assert!(
            argv.contains(&"HostKeyAlias=192.168.1.20".to_string()),
            "argv was: {argv:?}"
        );
        assert_eq!(argv[argv.len() - 2], "100.67.90.119");
    }

    #[test]
    fn a_shared_connection_is_asked_for_only_when_there_is_a_socket() {
        assert!(
            !remote(&["hi"])
                .to_ssh_args()
                .iter()
                .any(|arg| arg.starts_with("ControlMaster"))
        );

        let mut command = remote(&["hi"]);
        command.shared = Some(PathBuf::from("/tmp/slingshot-me/ab.sock"));
        let argv = command.to_ssh_args();

        assert!(argv.contains(&"ControlMaster=auto".to_string()), "{argv:?}");
        assert!(
            argv.contains(&"ControlPath=\"/tmp/slingshot-me/ab.sock\"".to_string()),
            "{argv:?}"
        );
        assert!(argv.contains(&"ControlPersist=30".to_string()), "{argv:?}");
    }

    #[test]
    fn no_key_configured_means_no_i_flag() {
        let argv = remote(&["hi"]).to_ssh_args();

        assert!(!argv.contains(&"-i".to_string()), "argv was: {argv:?}");
    }

    #[test]
    fn a_dropped_connection_is_told_apart_from_one_that_never_opened() {
        assert!(connection_lost(
            "Read from remote host 10.0.0.193: Can't assign requested address\nclient_loop: send disconnect: Broken pipe\n"
        ));
        assert!(connection_lost(
            "Timeout, server 10.0.0.193 not responding.\n"
        ));
        assert!(!connection_lost(
            "ssh: connect to host 10.0.0.193 port 22: Operation timed out\n"
        ));
        assert!(!connection_lost(
            "kex_exchange_identification: read: Connection reset by peer\n"
        ));
        assert!(!connection_lost(
            "ado@archbox: Permission denied (publickey).\n"
        ));
        assert!(!connection_lost(""));
    }

    #[test]
    fn the_ssh_log_is_private_and_removed_afterwards() {
        use std::os::unix::fs::PermissionsExt;

        let log = SshLog::create().unwrap();
        let path = log.0.clone();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        std::fs::write(&path, "client_loop: send disconnect: Broken pipe\r\n").unwrap();
        assert_eq!(log.read(), "client_loop: send disconnect: Broken pipe\n");
        drop(log);
        assert!(!path.exists());
    }
}
