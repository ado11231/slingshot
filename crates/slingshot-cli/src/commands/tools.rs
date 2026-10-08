//! `slingshot tools`: install the developer tools this machine uses on the Agent, after
//! showing every command and asking once. `link` runs the same step when it finishes.

use crate::client::{self, unexpected};
use crate::project;
use crate::route;
use crate::ssh::{self, RemoteCommand};
use slingshot_core::config::{Agent, Config};
use slingshot_core::control::{AgentTools, Request, Response};
use slingshot_core::presentation::{self, Style, Tone, plural, row};
use slingshot_core::step;
use slingshot_core::tools::{self, Tool};

pub async fn tools(agent: Option<String>) -> anyhow::Result<i32> {
    let config = Config::load()?;
    let target = config.resolve(agent.as_deref())?;
    offer(target).await?;
    Ok(0)
}

/// Compare both machines, list what the Agent lacks with the exact commands, and install
/// after one yes. Without a terminal it only prints the list, because nobody can answer.
pub async fn offer(target: &Agent) -> anyhow::Result<()> {
    presentation::section("Tools");
    let checking = step::start(format!("Checking tools on {}", target.name));
    let before = match check(target).await {
        Ok(before) => before,
        Err(error) => {
            checking.clear();
            return Err(error);
        }
    };
    let stacks = project::locate(&std::env::current_dir()?)
        .map(|local| local.stacks)
        .unwrap_or_default();
    let here = tools::installed_here();
    let offered = tools::missing(&here, &before.installed, &stacks);
    if offered.is_empty() {
        checking.done(format!("{} has every tool this machine uses", target.name));
        return offer_sign_in(target, &before, &here, &[], true).await;
    }
    checking.warn(format!(
        "{} is missing {}",
        target.name,
        plural(offered.len(), "tool")
    ));

    let manager = before.manager.as_deref();
    let admin = before.admin.unwrap_or(true);
    eprint!(
        "\n{}\n",
        plan(
            &offered,
            manager,
            before.npm_writable,
            admin,
            Style::stderr()
        )
    );
    let script = tools::install_script(&offered, manager, before.npm_writable, admin);
    if script.is_empty() {
        return Ok(());
    }
    if !ssh::wants_terminal() {
        presentation::warning("Run slingshot tools in a terminal to install them");
        return Ok(());
    }
    let them = match offered.len() {
        1 => "it",
        _ => "them",
    };
    if !presentation::confirm(format!("Install {them} on {} now?", target.name)).await? {
        eprintln!(
            "  {}",
            Style::stderr().dim("Run slingshot tools any time to do this later")
        );
        return Ok(());
    }

    let lost = format!(
        "Lost connection to {} while installing. Run slingshot tools again to see what finished",
        target.name
    );
    let (with_sudo, without_sudo) = by_sudo(&offered, manager, before.npm_writable, admin);
    let script = tools::install_script(&with_sudo, manager, before.npm_writable, admin);
    if !script.is_empty() {
        announce("Installing on", target);
        let remote = login_shell(target, &before.shell, script);
        crate::commands::run::interact(target, &remote, lost.clone()).await?;
    }
    for tool in without_sudo {
        let commands = tools::install_commands(tool, manager, before.npm_writable, admin);
        announce(&format!("Installing {} on", tool.name()), target);
        let mut remote = login_shell(
            target,
            &before.shell,
            format!(
                "exec 2>&1\n{}\n{}",
                tools::user_path_line(),
                commands.join("\n")
            ),
        );
        remote.tty = false;
        crate::commands::run::framed(target, &remote, lost.clone()).await?;
    }

    let after = check(target).await?;
    eprintln!();
    for tool in &offered {
        match after.installed.contains(tool) {
            true => presentation::success(format!("{} installed", tool.name())),
            false => presentation::warning(format!(
                "{} is still missing. Check the output above, then run slingshot tools again",
                tool.name()
            )),
        }
    }
    if offered.contains(&Tool::Docker) && after.installed.contains(&Tool::Docker) {
        eprintln!(
            "  {}",
            Style::stderr()
                .dim("Docker works in new sessions. End open ones with exit before using it there")
        );
    }

    let installed: Vec<Tool> = offered
        .iter()
        .copied()
        .filter(|tool| after.installed.contains(tool) && after.signed_out.contains(tool))
        .collect();
    if !installed.is_empty() {
        presentation::section("Sign in");
    }
    for tool in &installed {
        sign_in(target, &after.shell, *tool).await?;
    }
    offer_sign_in(target, &after, &here, &offered, installed.is_empty()).await
}

/// Tools that need `sudo` install with a terminal, so a password can be typed. The rest
/// need no input, so their output is framed under the step, where an installer that
/// redraws the screen cannot wipe what came before. `sudo` ones go first, since a tool
/// such as Codex needs Node.
fn by_sudo(
    offered: &[Tool],
    manager: Option<&str>,
    npm_writable: Option<bool>,
    admin: bool,
) -> (Vec<Tool>, Vec<Tool>) {
    offered
        .iter()
        .copied()
        .filter(|tool| !tools::install_commands(*tool, manager, npm_writable, admin).is_empty())
        .partition(|tool| {
            tools::install_commands(*tool, manager, npm_writable, admin)
                .iter()
                .any(|command| tools::needs_admin(command))
        })
}

/// Tools this machine uses that are installed on the Agent but signed out, apart from
/// `skip`, which were just installed and signed in.
fn waiting_for_sign_in(agent: &AgentTools, here: &[Tool], skip: &[Tool]) -> Vec<Tool> {
    agent
        .signed_out
        .iter()
        .copied()
        .filter(|tool| here.contains(tool) && !skip.contains(tool))
        .collect()
}

/// Offer to sign in to tools that were installed some other way and never signed in, so
/// the first use inside a session does not stop to ask.
async fn offer_sign_in(
    target: &Agent,
    agent: &AgentTools,
    here: &[Tool],
    skip: &[Tool],
    heading: bool,
) -> anyhow::Result<()> {
    let waiting = waiting_for_sign_in(agent, here, skip);
    if waiting.is_empty() {
        return Ok(());
    }
    let names: Vec<&str> = waiting.iter().map(|tool| tool.name()).collect();
    if heading {
        presentation::section("Sign in");
    }
    presentation::warning(format!(
        "{} not signed in on {}",
        match names.as_slice() {
            [one] => format!("{one} is"),
            _ => format!("{} are", names.join(" and ")),
        },
        target.name
    ));
    if !ssh::wants_terminal() {
        presentation::warning("Run slingshot tools in a terminal to sign in");
        return Ok(());
    }
    if !presentation::confirm(format!("Sign in on {} now?", target.name)).await? {
        eprintln!(
            "  {}",
            Style::stderr().dim("Run slingshot tools any time to do this later")
        );
        return Ok(());
    }
    for tool in waiting {
        sign_in(target, &agent.shell, tool).await?;
    }
    Ok(())
}

async fn check(target: &Agent) -> anyhow::Result<AgentTools> {
    match client::request(target, Request::Tools).await? {
        Response::Tools(tools) => Ok(tools),
        _ => Err(unexpected()),
    }
}

/// Sign in on the Agent, where the tool runs. Credentials are never copied from here.
async fn sign_in(target: &Agent, shell: &str, tool: Tool) -> anyhow::Result<()> {
    let Some(sign_in) = tool.sign_in() else {
        return Ok(());
    };
    let command =
        shell_words::join(std::iter::once(tool.program()).chain(sign_in.args.iter().copied()));
    announce(&format!("Signing in to {} on", tool.name()), target);
    if sign_in.forward.is_some() {
        eprintln!(
            "  {}",
            Style::stderr().dim("Open the link it prints in a browser on this machine")
        );
    }
    let mut remote = login_shell(
        target,
        shell,
        format!("{}; {command}", tools::user_path_line()),
    );
    remote.forward = sign_in.forward;
    let lost = format!(
        "Lost connection to {} while signing in. Try again in slingshot attach with: {command}",
        target.name
    );
    let code = crate::commands::run::interact(target, &remote, lost).await?;
    match code {
        0 => presentation::success(format!("Signed in to {}", tool.name())),
        _ => presentation::warning(format!(
            "Sign in to {} did not finish. Try again in slingshot attach with: {command}",
            tool.name()
        )),
    }
    Ok(())
}

/// A command run by the Agent account's login shell, so it sees the same PATH a session does.
fn login_shell(target: &Agent, shell: &str, script: String) -> RemoteCommand {
    let mut remote = RemoteCommand::to(
        target,
        shell.to_string(),
        vec!["-l".to_string(), "-c".to_string(), script],
    );
    remote.tty = true;
    remote
}

fn announce(action: &str, target: &Agent) {
    let style = Style::stderr();
    eprintln!(
        "{} {action} {} via {}\n",
        style.paint("▶", Tone::Info),
        style.paint(&target.name, Tone::Info),
        style.paint(route::resolve(target).name(), Tone::Info),
    );
}

/// Each missing tool with the exact commands that will run, or a warning when Slingshot has
/// none it trusts on that Agent. Without `admin`, commands that need `sudo` are listed apart,
/// for someone who can run them.
fn plan(
    offered: &[Tool],
    manager: Option<&str>,
    npm_writable: Option<bool>,
    admin: bool,
    style: Style,
) -> String {
    let mut output = String::new();
    let mut for_admin = String::new();
    for tool in offered {
        match tool.install(manager, npm_writable) {
            Some(commands) => {
                let (mine, theirs): (Vec<String>, Vec<String>) = commands
                    .into_iter()
                    .partition(|command| admin || !tools::needs_admin(command));
                for (index, command) in mine.iter().enumerate() {
                    let label = if index == 0 { tool.name() } else { "" };
                    output.push_str(&row(label, command));
                }
                for (index, command) in theirs.iter().enumerate() {
                    let label = if index == 0 { tool.name() } else { "" };
                    for_admin.push_str(&row(label, command));
                }
            }
            None => output.push_str(&row(
                tool.name(),
                style.paint(manual(manager), Tone::Warning),
            )),
        }
    }
    if !for_admin.is_empty() {
        output.push_str(&format!(
            "\n{}\n{for_admin}",
            style.status(
                "This account cannot use sudo on the Agent. Ask an admin to run:",
                Tone::Warning
            )
        ));
    }
    if offered.contains(&Tool::Docker) {
        output.push_str(&format!(
            "\n  {}\n",
            style.dim("The docker group can control the whole Agent, as an admin can. Add only accounts you trust")
        ));
    }
    output
}

fn manual(manager: Option<&str>) -> String {
    match manager {
        Some(manager) => {
            format!("No install command for {manager}. Install it on the Agent yourself")
        }
        None => "No package manager found. Install it on the Agent yourself".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tools_that_need_sudo_keep_the_terminal_and_the_rest_are_framed() {
        let offered = [Tool::Docker, Tool::ClaudeCode];
        let (with_sudo, without_sudo) = by_sudo(&offered, Some("pacman"), Some(false), true);
        assert_eq!(with_sudo, [Tool::Docker]);
        assert_eq!(without_sudo, [Tool::ClaudeCode]);

        let (with_sudo, without_sudo) = by_sudo(&offered, Some("pacman"), Some(false), false);
        assert!(with_sudo.is_empty());
        assert_eq!(without_sudo, [Tool::ClaudeCode]);
    }

    #[test]
    fn only_signed_out_tools_this_machine_uses_are_offered() {
        let agent = AgentTools {
            installed: vec![Tool::ClaudeCode, Tool::Codex],
            manager: None,
            shell: "/bin/sh".into(),
            npm_writable: None,
            signed_out: vec![Tool::ClaudeCode, Tool::Codex],
            admin: None,
        };
        assert_eq!(
            waiting_for_sign_in(&agent, &[Tool::Codex], &[]),
            vec![Tool::Codex]
        );
        assert!(waiting_for_sign_in(&agent, &[Tool::Codex], &[Tool::Codex]).is_empty());
        assert!(waiting_for_sign_in(&agent, &[Tool::Git], &[]).is_empty());
    }

    #[test]
    fn the_plan_shows_every_command_under_its_tool() {
        let text = plan(
            &[Tool::Git, Tool::Docker],
            Some("pacman"),
            None,
            true,
            Style::new(false),
        );

        assert!(text.contains("  Git          sudo pacman -S git\n"));
        assert!(text.contains("  Docker       sudo pacman -S docker\n"));
        assert!(text.contains("               sudo systemctl enable --now docker\n"));
        assert!(text.contains("The docker group can control the whole Agent"));
        assert!(!text.contains("Ask an admin"));
    }

    #[test]
    fn without_sudo_the_plan_lists_what_an_admin_must_run() {
        let text = plan(
            &[Tool::Git, Tool::ClaudeCode],
            Some("pacman"),
            None,
            false,
            Style::new(false),
        );
        let (mine, theirs) = text.split_once("Ask an admin to run:").unwrap();

        assert!(
            mine.contains("Claude Code  curl -fsSL https://claude.ai/install.sh | bash"),
            "{text}"
        );
        assert!(!mine.contains("sudo pacman"), "{text}");
        assert!(theirs.contains("Git          sudo pacman -S git"), "{text}");
    }

    #[test]
    fn the_plan_says_when_a_tool_must_be_installed_by_hand() {
        let text = plan(&[Tool::Docker], Some("brew"), None, true, Style::new(false));

        assert!(text.contains("No install command for brew"), "{text}");
        assert!(
            plan(&[Tool::Git], None, None, true, Style::new(false))
                .contains("No package manager found")
        );
    }
}
