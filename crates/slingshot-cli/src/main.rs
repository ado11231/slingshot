//! slingshot: run heavy work on another machine, from a light one.

mod client;
mod commands;
mod keys;
mod live;
mod project;
mod route;
mod ssh;
mod transfer;
mod tunnel;
mod watch;

use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};
use slingshot_core::presentation::{self, ColorMode, Style, Tone};
use slingshot_core::protocol::DEFAULT_PORT;
use std::path::PathBuf;

/// Global options precede run so the remote command can receive its own flags.
#[derive(Parser)]
#[command(name = "slingshot", version, about = "Run heavy work on another machine", long_about = None)]
struct Cli {
    /// Control terminal colors.
    #[arg(long, global = true, value_parser = ["auto", "always", "never"], default_value = "auto")]
    color: String,

    /// Which box to use. Defaults to the only one, or the one marked default.
    #[arg(long, short, global = true)]
    agent: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Relays control messages between SSH and the Agent's private socket.
    #[command(hide = true)]
    InternalControl,

    /// Runs one foreground command in a project copy on the Agent.
    #[command(hide = true)]
    InternalRun {
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        cwd: Option<String>,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        cmd: Vec<String>,
    },

    /// ssh's ProxyCommand when the Agent is reached over iroh.
    #[command(hide = true)]
    InternalTunnel { key: String },

    /// Prints the menu bar app's numbers and notifications as JSON lines until input closes.
    #[command(hide = true)]
    InternalWatch,

    /// The remote shell rsync uses, with Slingshot's saved SSH options.
    #[command(hide = true)]
    InternalRsh {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },

    #[command(about = "Agent: start the daemon and print a pairing code")]
    Start {
        /// The name this box will be known by. Defaults to its hostname.
        #[arg(long)]
        name: Option<String>,

        #[arg(long, default_value_t = DEFAULT_PORT)]
        port: u16,
    },

    #[command(about = "Client: pair with a box using the code slingshot start printed")]
    Link {
        code: String,

        /// Save the box under a different name than the one it calls itself.
        #[arg(long)]
        name: Option<String>,
    },

    #[command(about = "Remove Slingshot access and this machine's environment files from a box")]
    Unlink,

    #[command(about = "Install the developer tools this machine uses on the Agent, after asking")]
    Tools,

    /// `trailing_var_arg` stops clap parsing after `run`, so flags like `--release`
    /// reach the remote program untouched rather than being claimed by slingshot.
    #[command(about = "Sync the project, then run a command on the Agent")]
    Run {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        cmd: Vec<String>,
    },

    #[command(about = "Open or rejoin the project's persistent session on the Agent")]
    Attach {
        /// Project folder. Defaults to the current project.
        path: Option<PathBuf>,
    },

    #[command(about = "Copy project source changes to the Agent, or back with --pull")]
    Sync {
        /// Retrieve edits made on the Agent.
        #[arg(long)]
        pull: bool,

        /// Show what would change without changing anything.
        #[arg(long)]
        check: bool,

        /// Project folder. Defaults to the current project.
        path: Option<PathBuf>,
    },

    #[command(about = "Manage environment files kept on the Agent")]
    Env {
        #[command(subcommand)]
        action: EnvAction,
    },

    #[command(about = "List Slingshot runs and sessions on the Agent")]
    Ps {
        /// Include the most recent 100 finished jobs.
        #[arg(long)]
        all: bool,
    },

    #[command(about = "Stop a Slingshot run or session")]
    Stop {
        /// The job ID from slingshot ps. A unique prefix is enough.
        id: String,
    },

    #[command(about = "Static specs of the box, from the cache")]
    Info {
        /// Ask the box again instead of using what was saved at pairing.
        #[arg(long)]
        refresh: bool,
    },

    #[command(about = "Current CPU, RAM, GPU, and disk usage")]
    Health {
        /// Keep refreshing every two seconds until Q or Ctrl C.
        #[arg(long)]
        watch: bool,
    },

    #[command(about = "Live resource use with active Slingshot jobs")]
    Top,

    #[command(about = "Show the box's live status in the macOS menu bar")]
    Menubar {
        /// Quit the app, stop it starting at login, and delete it.
        #[arg(long)]
        remove: bool,
    },
}

#[derive(Subcommand)]
enum EnvAction {
    #[command(about = "Store a local environment file for this project on the Agent")]
    Add {
        /// The local file to upload. Its contents are never printed.
        #[arg(long)]
        file: PathBuf,

        /// Where it appears in the Agent copy, such as .env or api/.env.local.
        #[arg(long)]
        target: String,

        /// Replace an existing file at the same target.
        #[arg(long)]
        replace: bool,
    },

    #[command(about = "List environment file names for this project")]
    List,

    #[command(about = "Remove an environment file from the Agent")]
    Remove { target: String },
}

/// Slingshot's own warnings show by default. Libraries such as iroh only show real errors,
/// because their internals mean nothing to the person running a command.
const DEFAULT_LOG: &str = "error,slingshot_cli=warn,slingshot_agent=warn,slingshot_core=warn";

/// Pass through command exit codes. Slingshot failures exit with code 1.
#[tokio::main]
async fn main() {
    let args: Vec<_> = std::env::args_os().collect();
    let mode = color_mode(&args);
    presentation::configure(mode);
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("SLINGSHOT_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(DEFAULT_LOG)),
        )
        .with_writer(std::io::stderr)
        .with_ansi(mode.enabled(
            std::io::IsTerminal::is_terminal(&std::io::stderr()),
            presentation::no_color(),
            presentation::dumb_terminal(),
        ))
        .init();

    let color = match mode {
        ColorMode::Always => clap::ColorChoice::Always,
        ColorMode::Never => clap::ColorChoice::Never,
        ColorMode::Auto if presentation::no_color() || presentation::dumb_terminal() => {
            clap::ColorChoice::Never
        }
        ColorMode::Auto => clap::ColorChoice::Auto,
    };
    let matches = Cli::command().color(color).get_matches_from(args);
    let cli = Cli::from_arg_matches(&matches).unwrap_or_else(|error| error.exit());

    let result = match cli.command {
        Commands::InternalControl => slingshot_agent::service::bridge().await,
        Commands::InternalRun { project, cwd, cmd } => {
            slingshot_agent::runner::run(project, cwd, cmd).await
        }
        Commands::InternalRsh { args } => internal_rsh(args),
        Commands::InternalTunnel { key } => tunnel::run(key).await,
        Commands::InternalWatch => watch::run(cli.agent).await,
        Commands::Start { name, port } => slingshot_agent::start(name, port).await,
        Commands::Link { code, name } => commands::link::link(code, name).await,
        Commands::Unlink => commands::unlink::unlink(cli.agent).await,
        Commands::Tools => commands::tools::tools(cli.agent).await,
        Commands::Run { cmd } => commands::run::run(cli.agent, cmd).await,
        Commands::Attach { path } => commands::attach::attach(cli.agent, path).await,
        Commands::Sync { pull, check, path } => {
            commands::sync::sync(cli.agent, path, pull, check).await
        }
        Commands::Env { action } => match action {
            EnvAction::Add {
                file,
                target,
                replace,
            } => commands::env::add(cli.agent, file, target, replace).await,
            EnvAction::List => commands::env::list(cli.agent).await,
            EnvAction::Remove { target } => commands::env::remove(cli.agent, target).await,
        },
        Commands::Ps { all } => commands::ps::ps(cli.agent, all).await,
        Commands::Stop { id } => commands::ps::stop(cli.agent, id).await,
        Commands::Info { refresh } => commands::info::info(cli.agent, refresh).await,
        Commands::Health { watch } => commands::health::health(cli.agent, watch).await,
        Commands::Top => commands::top::top(cli.agent).await,
        Commands::Menubar { remove } => commands::menubar::menubar(cli.agent, remove).await,
    };

    match result {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            eprintln!("{}", Style::stderr().status(format!("{e:#}"), Tone::Error));
            std::process::exit(1);
        }
    }
}

/// rsync starts this as its remote shell. The Agent name arrives in the environment, so
/// nothing about it has to survive rsync's own argument splitting.
fn internal_rsh(args: Vec<String>) -> anyhow::Result<i32> {
    let name =
        std::env::var("SLINGSHOT_RSH_AGENT").map_err(|_| anyhow::anyhow!("Missing Agent name"))?;
    let config = slingshot_core::config::Config::load()?;
    let agent = config.resolve(Some(&name))?;
    if let Ok(token) = std::env::var(route::ROUTE_ENV) {
        route::assume(agent, &token);
    }
    Err(ssh::exec_for_rsync(agent, args))
}

/// Read color before Clap renders help. Stop at run so remote flags remain untouched.
fn color_mode(args: &[std::ffi::OsString]) -> ColorMode {
    let mut mode = ColorMode::Auto;
    let mut args = args.iter().skip(1);
    while let Some(arg) = args.next() {
        let arg = arg.to_string_lossy();
        if matches!(arg.as_ref(), "run" | "internal-run" | "internal-rsh" | "--") {
            break;
        }
        if matches!(arg.as_ref(), "--agent" | "-a" | "--name" | "--port") {
            args.next();
            continue;
        }
        let value = if arg == "--color" {
            args.next().map(|value| value.to_string_lossy())
        } else {
            arg.strip_prefix("--color=").map(std::borrow::Cow::Borrowed)
        };
        mode = match value.as_deref() {
            Some("always") => ColorMode::Always,
            Some("never") => ColorMode::Never,
            Some("auto") => ColorMode::Auto,
            _ => mode,
        };
    }
    mode
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_flags_stay_with_the_remote_command() {
        let cli = Cli::try_parse_from([
            "slingshot",
            "--color",
            "never",
            "run",
            "cargo",
            "--color",
            "always",
            "--release",
        ])
        .unwrap();
        let Commands::Run { cmd } = cli.command else {
            panic!("Expected run")
        };
        assert_eq!(cmd, ["cargo", "--color", "always", "--release"]);
        assert_eq!(cli.color, "never");
    }

    #[test]
    fn internal_helpers_keep_their_arguments() {
        let cli = Cli::try_parse_from([
            "slingshot",
            "internal-run",
            "--project",
            "abc",
            "--",
            "cargo",
            "--color",
            "always",
        ])
        .unwrap();
        let Commands::InternalRun { project, cwd, cmd } = cli.command else {
            panic!("Expected internal run")
        };
        assert_eq!(project.as_deref(), Some("abc"));
        assert_eq!(cwd, None);
        assert_eq!(cmd, ["cargo", "--color", "always"]);

        let cli = Cli::try_parse_from([
            "slingshot",
            "internal-rsh",
            "slingshot",
            "rsync",
            "--server",
            "-a",
            ".",
            ".",
        ])
        .unwrap();
        let Commands::InternalRsh { args } = cli.command else {
            panic!("Expected internal rsh")
        };
        assert_eq!(args, ["slingshot", "rsync", "--server", "-a", ".", "."]);
    }

    #[test]
    fn sync_and_env_commands_parse() {
        let cli = Cli::try_parse_from(["slingshot", "sync", "--pull", "--check", "app"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Sync {
                pull: true,
                check: true,
                path: Some(_)
            }
        ));
        let cli = Cli::try_parse_from([
            "slingshot",
            "env",
            "add",
            "--file",
            "local.env",
            "--target",
            "api/.env",
            "--replace",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Commands::Env {
                action: EnvAction::Add { replace: true, .. }
            }
        ));
        assert!(Cli::try_parse_from(["slingshot", "stop"]).is_err());
    }

    #[test]
    fn color_scan_respects_option_values_and_run_boundary() {
        for (args, expected) in [
            (
                vec!["slingshot", "--agent", "run", "--color=always", "info"],
                ColorMode::Always,
            ),
            (
                vec!["slingshot", "run", "echo", "--color=always"],
                ColorMode::Auto,
            ),
            (
                vec!["slingshot", "--color", "never", "--help"],
                ColorMode::Never,
            ),
        ] {
            let args: Vec<_> = args.into_iter().map(std::ffi::OsString::from).collect();
            assert_eq!(color_mode(&args), expected);
        }
    }
}
