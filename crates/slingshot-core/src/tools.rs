//! The developer tools Slingshot can set up on the Agent. The Agent reports which it has,
//! and the Client decides what to offer and shows every command before anything runs.

use crate::preflight::install_command;
use crate::stack::Stack;
use crate::telemetry::is_installed;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tool {
    Git,
    Docker,
    Node,
    Python,
    Rust,
    ClaudeCode,
    Codex,
}

/// Every tool, in install order. Codex installs with npm, so Node comes before it.
pub const ALL: [Tool; 7] = [
    Tool::Git,
    Tool::Docker,
    Tool::Node,
    Tool::Python,
    Tool::Rust,
    Tool::ClaudeCode,
    Tool::Codex,
];

/// How to sign in to a tool on the Agent. Credentials are never copied from the Client,
/// so each tool signs in where it runs.
#[derive(Debug, PartialEq, Eq)]
pub struct SignIn {
    pub args: &'static [&'static str],
    /// Arguments that exit 0 when the tool is signed in and non zero when it is not.
    pub status: &'static [&'static str],
    /// A port on the Agent that the sign in page on the Client must reach, forwarded over ssh.
    pub forward: Option<u16>,
}

impl Tool {
    pub fn name(self) -> &'static str {
        match self {
            Tool::Git => "Git",
            Tool::Docker => "Docker",
            Tool::Node => "Node and npm",
            Tool::Python => "Python",
            Tool::Rust => "Rust",
            Tool::ClaudeCode => "Claude Code",
            Tool::Codex => "Codex",
        }
    }

    /// The program whose presence shows the tool is installed.
    pub fn program(self) -> &'static str {
        match self {
            Tool::Git => "git",
            Tool::Docker => "docker",
            Tool::Node => "node",
            Tool::Python => "python3",
            Tool::Rust => "cargo",
            Tool::ClaudeCode => "claude",
            Tool::Codex => "codex",
        }
    }

    fn needs(self) -> Option<Tool> {
        match self {
            Tool::Codex => Some(Tool::Node),
            _ => None,
        }
    }

    fn for_stack(stack: Stack) -> Tool {
        match stack {
            Stack::Rust => Tool::Rust,
            Stack::Node => Tool::Node,
            Stack::Python => Tool::Python,
        }
    }

    /// Packages for tools that come from the system package manager.
    fn packages(self, manager: &str) -> Option<&'static str> {
        match (self, manager) {
            (Tool::Git, _) => Some("git"),
            (Tool::Docker, "pacman" | "zypper") => Some("docker"),
            (Tool::Docker, "apt") => Some("docker.io"),
            (Tool::Docker, "dnf") => Some("moby-engine"),
            (Tool::Node, "brew") => Some("node"),
            (Tool::Node, "pacman" | "apt" | "dnf" | "apk") => Some("nodejs npm"),
            (Tool::Python, "pacman") => Some("python python-pip"),
            (Tool::Python, "apt") => Some("python3 python3-pip python3-venv"),
            (Tool::Python, "dnf" | "zypper") => Some("python3 python3-pip"),
            (Tool::Python, "apk") => Some("python3 py3-pip"),
            (Tool::Python, "brew") => Some("python"),
            _ => None,
        }
    }

    /// The commands that install this tool on an Agent using `manager`, where `npm_writable`
    /// says whether npm's global folder needs `sudo`. `None` means Slingshot has no command it
    /// trusts there, so the person installs it themselves.
    pub fn install(self, manager: Option<&str>, npm_writable: Option<bool>) -> Option<Vec<String>> {
        match self {
            Tool::Rust => Some(vec![
                "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y".into(),
            ]),
            Tool::ClaudeCode => Some(vec![
                "curl -fsSL https://claude.ai/install.sh | bash".into(),
            ]),
            Tool::Codex => {
                let sudo = npm_writable.map_or(manager != Some("brew"), |writable| !writable);
                let command = "npm install -g @openai/codex";
                Some(vec![match sudo {
                    true => format!("sudo {command}"),
                    false => command.to_string(),
                }])
            }
            Tool::Docker => {
                let manager = manager?;
                let mut commands = vec![install_command(manager, self.packages(manager)?)?];
                commands.push("sudo systemctl enable --now docker".into());
                commands.push("sudo usermod -aG docker \"$USER\"".into());
                Some(commands)
            }
            Tool::Git | Tool::Node | Tool::Python => {
                let manager = manager?;
                Some(vec![install_command(manager, self.packages(manager)?)?])
            }
        }
    }

    pub fn sign_in(self) -> Option<SignIn> {
        match self {
            Tool::ClaudeCode => Some(SignIn {
                args: &["auth", "login"],
                status: &["auth", "status"],
                forward: None,
            }),
            Tool::Codex => Some(SignIn {
                args: &["login"],
                status: &["login", "status"],
                forward: Some(1455),
            }),
            _ => None,
        }
    }
}

/// What to offer the Agent: tools the Client has or the project needs, plus anything those
/// need to install, minus what the Agent already has. Returned in install order.
pub fn missing(client: &[Tool], agent: &[Tool], stacks: &[Stack]) -> Vec<Tool> {
    let mut wanted: Vec<Tool> = client
        .iter()
        .copied()
        .chain(stacks.iter().map(|stack| Tool::for_stack(*stack)))
        .collect();
    let needed: Vec<Tool> = wanted.iter().filter_map(|tool| tool.needs()).collect();
    wanted.extend(needed);
    ALL.into_iter()
        .filter(|tool| wanted.contains(tool) && !agent.contains(tool))
        .collect()
}

/// The tools installed on this machine, as the current PATH sees them.
pub fn installed_here() -> Vec<Tool> {
    ALL.into_iter()
        .filter(|tool| is_installed(tool.program()))
        .collect()
}

/// Folders in the home folder where installers put programs for one user, such as Claude
/// Code in `~/.local/bin`. Login shells that are not interactive often leave them off PATH,
/// so Slingshot adds them to everything it starts on the Agent instead of editing any file.
pub const USER_FOLDERS: [&str; 2] = [".local/bin", ".cargo/bin"];

/// A shell line that puts `USER_FOLDERS` at the front of PATH.
pub fn user_path_line() -> String {
    let folders: Vec<String> = USER_FOLDERS
        .iter()
        .map(|folder| format!("$HOME/{folder}"))
        .collect();
    format!("export PATH=\"{}:$PATH\"", folders.join(":"))
}

/// `path` with each folder of `USER_FOLDERS` under `home` added in front, unless it is
/// already there, for programs started without a shell.
pub fn with_user_folders(path: &str, home: &Path) -> String {
    let current: Vec<&str> = path.split(':').filter(|part| !part.is_empty()).collect();
    let added: Vec<String> = USER_FOLDERS
        .iter()
        .map(|folder| home.join(folder).display().to_string())
        .filter(|folder| !current.contains(&folder.as_str()))
        .collect();
    added
        .iter()
        .map(String::as_str)
        .chain(current)
        .collect::<Vec<&str>>()
        .join(":")
}

/// A script that prints the program name of each tool it finds, whether npm's global
/// folder is writable, and each installed tool that is signed out. The Agent runs it in a
/// login shell with the per user folders added, so it sees what a session and a run do.
pub fn probe_script() -> String {
    let programs: Vec<&str> = ALL.iter().map(|tool| tool.program()).collect();
    let statuses: Vec<String> = ALL
        .iter()
        .filter_map(|tool| {
            let status = tool.sign_in()?.status;
            let command = format!("{} {}", tool.program(), status.join(" "));
            Some(format!(
                "if command -v {program} >/dev/null 2>&1 && ! {command} >/dev/null 2>&1 </dev/null; then echo {SIGNED_OUT}{program}; fi",
                program = tool.program()
            ))
        })
        .collect();
    format!(
        "{}; for program in {}; do if command -v \"$program\" >/dev/null 2>&1; then echo \"$program\"; fi; done; \
         if command -v npm >/dev/null 2>&1; then if [ -w \"$(npm prefix -g)\" ]; then echo {NPM_WRITABLE}; else echo {NPM_ROOT}; fi; fi; {}; \
         if [ \"$(id -u)\" = 0 ] || id -Gn | tr ' ' '\\n' | grep -qxE 'wheel|sudo|admin'; then echo {ADMIN}; else echo {NOT_ADMIN}; fi",
        user_path_line(),
        programs.join(" "),
        statuses.join("; ")
    )
}

const ADMIN: &str = "admin:yes";
const NOT_ADMIN: &str = "admin:no";

/// Whether the output of `probe_script` says the account can use `sudo`, from its groups.
/// `None` when the Agent did not say.
pub fn admin(output: &str) -> Option<bool> {
    output.lines().find_map(|line| match line.trim() {
        ADMIN => Some(true),
        NOT_ADMIN => Some(false),
        _ => None,
    })
}

/// Commands that only an admin account can run.
pub fn needs_admin(command: &str) -> bool {
    command.starts_with("sudo ")
}

const SIGNED_OUT: &str = "signed-out:";

/// The installed tools that `probe_script` found signed out.
pub fn signed_out(output: &str) -> Vec<Tool> {
    let names: Vec<&str> = output
        .lines()
        .filter_map(|line| line.trim().strip_prefix(SIGNED_OUT))
        .collect();
    ALL.into_iter()
        .filter(|tool| names.contains(&tool.program()))
        .collect()
}

const NPM_WRITABLE: &str = "npm:writable";
const NPM_ROOT: &str = "npm:root";

/// Whether the output of `probe_script` says npm's global folder is writable, or `None`
/// when npm is not installed.
pub fn npm_writable(output: &str) -> Option<bool> {
    output.lines().find_map(|line| match line.trim() {
        NPM_WRITABLE => Some(true),
        NPM_ROOT => Some(false),
        _ => None,
    })
}

/// The tools named in the output of `probe_script`.
pub fn found(output: &str) -> Vec<Tool> {
    let programs: Vec<&str> = output.lines().map(str::trim).collect();
    ALL.into_iter()
        .filter(|tool| programs.contains(&tool.program()))
        .collect()
}

/// The commands that install `tool` here, leaving out those that need `sudo` without `admin`.
pub fn install_commands(
    tool: Tool,
    manager: Option<&str>,
    npm_writable: Option<bool>,
    admin: bool,
) -> Vec<String> {
    tool.install(manager, npm_writable)
        .unwrap_or_default()
        .into_iter()
        .filter(|command| admin || !needs_admin(command))
        .collect()
}

/// One script that installs `tools` in order, with a numbered heading before each. It keeps
/// going after a failure, because the Agent is checked again afterwards to see what worked.
/// Tools without a command are left out.
///
/// Without `admin`, commands that need `sudo` are left out, and so is a tool with nothing
/// left to run. They are listed for an admin instead.
pub fn install_script(
    tools: &[Tool],
    manager: Option<&str>,
    npm_writable: Option<bool>,
    admin: bool,
) -> String {
    let planned: Vec<(Tool, Vec<String>)> = tools
        .iter()
        .filter_map(|tool| {
            let commands = install_commands(*tool, manager, npm_writable, admin);
            (!commands.is_empty()).then_some((*tool, commands))
        })
        .collect();
    let mut lines = Vec::new();
    for (index, (tool, commands)) in planned.iter().enumerate() {
        lines.push(format!(
            "echo; echo '▶ {} ({} of {})'; echo",
            tool.name(),
            index + 1,
            planned.len()
        ));
        lines.extend(commands.iter().cloned());
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tools_the_client_has_and_the_agent_lacks_are_offered() {
        let offered = missing(&[Tool::Git, Tool::Docker], &[Tool::Git], &[]);

        assert_eq!(offered, vec![Tool::Docker]);
    }

    #[test]
    fn the_project_adds_its_own_tools() {
        let offered = missing(&[], &[], &[Stack::Rust, Stack::Python]);

        assert_eq!(offered, vec![Tool::Python, Tool::Rust]);
    }

    #[test]
    fn codex_brings_node_when_the_agent_has_none() {
        assert_eq!(
            missing(&[Tool::Codex], &[], &[]),
            vec![Tool::Node, Tool::Codex]
        );
        assert_eq!(
            missing(&[Tool::Codex], &[Tool::Node], &[]),
            vec![Tool::Codex]
        );
    }

    #[test]
    fn nothing_is_offered_when_the_agent_has_everything() {
        assert!(missing(&ALL, &ALL, &[Stack::Node]).is_empty());
    }

    #[test]
    fn offers_come_in_install_order_without_repeats() {
        let offered = missing(&[Tool::Codex, Tool::Git, Tool::Node], &[], &[Stack::Node]);

        assert_eq!(offered, vec![Tool::Git, Tool::Node, Tool::Codex]);
    }

    #[test]
    fn docker_on_linux_also_starts_the_service_and_joins_the_group() {
        let commands = Tool::Docker.install(Some("pacman"), None).unwrap();

        assert_eq!(commands[0], "sudo pacman -S docker");
        assert!(
            commands
                .iter()
                .any(|c| c.contains("systemctl enable --now docker"))
        );
        assert!(commands.iter().any(|c| c.contains("usermod -aG docker")));
        assert_eq!(
            Tool::Docker.install(Some("apt"), None).unwrap()[0],
            "sudo apt install docker.io"
        );
    }

    #[test]
    fn tools_without_a_trusted_command_are_left_to_the_person() {
        assert_eq!(Tool::Docker.install(Some("brew"), None), None);
        assert_eq!(Tool::Git.install(None, None), None);
        assert_eq!(Tool::Node.install(Some("zypper"), None), None);
    }

    #[test]
    fn installers_that_need_no_package_manager_always_have_a_command() {
        for tool in [Tool::Rust, Tool::ClaudeCode, Tool::Codex] {
            assert!(tool.install(None, None).is_some(), "{tool:?}");
        }
    }

    #[test]
    fn the_install_script_names_each_tool_and_skips_ones_without_a_command() {
        let script = install_script(
            &[Tool::Git, Tool::Docker, Tool::Rust],
            Some("brew"),
            None,
            true,
        );

        assert!(script.contains("echo '▶ Git (1 of 2)'; echo\nbrew install git"));
        assert!(!script.contains("Docker"));
        assert!(script.contains("▶ Rust (2 of 2)"));
    }

    #[test]
    fn user_folders_go_in_front_once() {
        let home = Path::new("/home/me");

        assert_eq!(
            with_user_folders("/usr/local/bin:/usr/bin", home),
            "/home/me/.local/bin:/home/me/.cargo/bin:/usr/local/bin:/usr/bin"
        );
        assert_eq!(
            with_user_folders("/home/me/.cargo/bin:/usr/bin", home),
            "/home/me/.local/bin:/home/me/.cargo/bin:/usr/bin"
        );
        assert_eq!(
            with_user_folders("", home),
            "/home/me/.local/bin:/home/me/.cargo/bin"
        );
    }

    #[test]
    fn the_probe_looks_in_the_user_folders_first() {
        assert!(
            probe_script().starts_with("export PATH=\"$HOME/.local/bin:$HOME/.cargo/bin:$PATH\";")
        );
    }

    #[test]
    fn codex_uses_sudo_only_when_npm_needs_it() {
        let codex = |manager, writable| Tool::Codex.install(manager, writable).unwrap()[0].clone();

        assert_eq!(
            codex(Some("pacman"), Some(false)),
            "sudo npm install -g @openai/codex"
        );
        assert_eq!(
            codex(Some("pacman"), Some(true)),
            "npm install -g @openai/codex"
        );
        assert_eq!(
            codex(Some("pacman"), None),
            "sudo npm install -g @openai/codex"
        );
        assert_eq!(codex(Some("brew"), None), "npm install -g @openai/codex");
    }

    #[test]
    fn the_probe_reports_whether_npm_is_writable() {
        assert_eq!(npm_writable("git\nnode\nnpm:root\n"), Some(false));
        assert_eq!(npm_writable("node\nnpm:writable\n"), Some(true));
        assert_eq!(npm_writable("git\n"), None);
        assert_eq!(found("git\nnpm:root\n"), vec![Tool::Git]);
    }

    #[test]
    fn the_scripts_are_valid_shell() {
        let install = install_script(&ALL, Some("pacman"), Some(false), true);
        for script in [probe_script(), install] {
            let status = std::process::Command::new("sh")
                .args(["-n", "-c", &script])
                .status()
                .unwrap();
            assert!(status.success(), "{script}");
        }
    }

    #[test]
    fn probe_output_maps_back_to_tools() {
        assert!(probe_script().contains("claude"));
        assert_eq!(found("git\ncodex\nunknown\n"), vec![Tool::Git, Tool::Codex]);
    }

    #[test]
    fn the_probe_checks_sign_in_and_reads_it_back() {
        let script = probe_script();
        assert!(script.contains("! claude auth status"), "{script}");
        assert!(script.contains("! codex login status"), "{script}");
        assert_eq!(
            signed_out("claude\ncodex\nsigned-out:codex\n"),
            vec![Tool::Codex]
        );
        assert!(found("signed-out:codex\n").is_empty());
        assert!(signed_out("claude\n").is_empty());
    }

    #[test]
    fn an_account_without_sudo_runs_only_what_it_can() {
        let script = install_script(
            &[Tool::Docker, Tool::ClaudeCode],
            Some("pacman"),
            Some(true),
            false,
        );
        assert!(!script.contains("sudo"), "{script}");
        assert!(!script.contains("Docker"), "{script}");
        assert!(script.contains("▶ Claude Code (1 of 1)"), "{script}");
        assert!(script.contains("curl -fsSL https://claude.ai/install.sh | bash"));
        assert!(
            install_script(&[Tool::Docker], Some("pacman"), None, true).contains("sudo pacman")
        );
    }

    #[test]
    fn the_probe_says_whether_the_account_is_an_admin() {
        assert!(probe_script().contains("wheel|sudo|admin"));
        assert_eq!(admin("git\nadmin:no\n"), Some(false));
        assert_eq!(admin("admin:yes"), Some(true));
        assert_eq!(admin("git"), None);
        assert!(needs_admin("sudo pacman -S git"));
        assert!(!needs_admin("npm install -g @openai/codex"));
    }

    #[test]
    fn only_coding_agents_sign_in() {
        assert_eq!(Tool::Codex.sign_in().unwrap().forward, Some(1455));
        assert_eq!(Tool::ClaudeCode.sign_in().unwrap().forward, None);
        assert_eq!(Tool::Git.sign_in(), None);
    }
}
