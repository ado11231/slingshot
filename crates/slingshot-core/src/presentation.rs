//! Shared formatting for Slingshot output. Stream detection keeps redirected output plain.

use anyhow::Context;
use std::io::{IsTerminal, stderr, stdout};
use std::path::Path;
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum ColorMode {
    #[default]
    Auto,
    Always,
    Never,
}

impl ColorMode {
    pub fn enabled(self, terminal: bool, no_color: bool, dumb: bool) -> bool {
        match self {
            Self::Always => true,
            Self::Never => false,
            Self::Auto => terminal && !no_color && !dumb,
        }
    }
}

static COLOR: OnceLock<ColorMode> = OnceLock::new();

pub fn configure(mode: ColorMode) {
    let _ = COLOR.set(mode);
}

#[derive(Clone, Copy)]
pub enum Tone {
    Good,
    Warning,
    Error,
    Info,
}

#[derive(Clone, Copy)]
pub struct Style {
    color: bool,
}

impl Style {
    pub fn new(color: bool) -> Self {
        Self { color }
    }

    pub fn stdout() -> Self {
        Self::for_terminal(stdout().is_terminal())
    }

    pub fn stderr() -> Self {
        Self::for_terminal(stderr().is_terminal())
    }

    fn for_terminal(terminal: bool) -> Self {
        let mode = COLOR.get().copied().unwrap_or_default();
        Self::new(mode.enabled(terminal, no_color(), dumb_terminal()))
    }

    pub fn paint(self, text: impl std::fmt::Display, tone: Tone) -> String {
        if !self.color {
            return text.to_string();
        }
        let code = match tone {
            Tone::Good => 32,
            Tone::Warning => 33,
            Tone::Error => 31,
            Tone::Info => 36,
        };
        format!("\x1b[{code}m{text}\x1b[0m")
    }

    pub fn colored(self) -> bool {
        self.color
    }

    pub fn dim(self, text: impl std::fmt::Display) -> String {
        match self.color {
            true => format!("\x1b[2m{text}\x1b[0m"),
            false => text.to_string(),
        }
    }

    /// Bold rather than colored, so color stays reserved for things that need attention.
    pub fn heading(self, text: impl std::fmt::Display) -> String {
        match self.color {
            true => format!("\x1b[1m{text}\x1b[0m"),
            false => text.to_string(),
        }
    }

    pub fn status(self, text: impl std::fmt::Display, tone: Tone) -> String {
        let label = match tone {
            Tone::Good => "✓",
            Tone::Warning => "!",
            Tone::Error => "✗",
            Tone::Info => "▶",
        };
        format!("{} {text}", self.paint(label, tone))
    }
}

/// `1 file` or `3 files`: the plural form English needs for a count in a sentence.
pub fn plural(count: usize, noun: &str) -> String {
    match count {
        1 => format!("1 {noun}"),
        _ => format!("{count} {noun}s"),
    }
}

/// One indented `label   value` line. Colorless on purpose: the label is structure, and
/// any emphasis belongs to the value the caller passes in.
pub fn row(label: &str, value: impl std::fmt::Display) -> String {
    format!("  {label:<12} {value}\n")
}

/// A path with the home folder written as `~`, which is shorter to read.
pub fn home_path(path: &Path) -> String {
    let home = directories::BaseDirs::new().map(|dirs| dirs.home_dir().to_path_buf());
    tilde(path, home.as_deref())
}

fn tilde(path: &Path, home: Option<&Path>) -> String {
    match home.and_then(|home| path.strip_prefix(home).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

/// Whether the NO_COLOR convention is in effect.
pub fn no_color() -> bool {
    std::env::var_os("NO_COLOR").is_some_and(|value| !value.is_empty())
}

pub fn dumb_terminal() -> bool {
    std::env::var_os("TERM").is_some_and(|value| value == "dumb")
}

pub fn success(text: impl std::fmt::Display) {
    eprintln!("{}", Style::stderr().status(text, Tone::Good));
}

pub fn warning(text: impl std::fmt::Display) {
    eprintln!("{}", Style::stderr().status(text, Tone::Warning));
}

pub fn progress(text: impl std::fmt::Display) {
    eprintln!("{}", Style::stderr().status(text, Tone::Info));
}

/// A dim `label   value` line under a step, for facts worth keeping but not reading first.
pub fn detail(label: &str, value: impl std::fmt::Display) {
    eprintln!("{}", Style::stderr().dim(row(label, value).trim_end()));
}

/// A byte count in MiB, shown as MiB or GiB depending on size.
pub fn capacity(mib: u64) -> String {
    if mib < 1024 {
        format!("{:.1} MiB", mib as f64)
    } else {
        format!("{:.1} GiB", mib as f64 / 1024.0)
    }
}

/// Ask once, where Enter means yes and a closed input means no.
pub async fn confirm(question: String) -> anyhow::Result<bool> {
    eprint!("\n{question} [Y/n] ");
    let answer = tokio::task::spawn_blocking(|| {
        let mut line = String::new();
        std::io::stdin()
            .read_line(&mut line)
            .map(|read| (read > 0).then_some(line))
    })
    .await
    .context("Could not read the answer")?
    .context("Could not read the answer")?;
    eprintln!();
    Ok(answer.is_some_and(|answer| accepted(&answer)))
}

fn accepted(answer: &str) -> bool {
    matches!(answer.trim().to_lowercase().as_str(), "" | "y" | "yes")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enter_and_yes_accept_and_anything_else_declines() {
        assert!(accepted("\n"));
        assert!(accepted("Y\n"));
        assert!(accepted(" yes "));
        assert!(!accepted("n\n"));
        assert!(!accepted("no"));
        assert!(!accepted("later"));
    }

    #[test]
    fn capacity_preserves_small_values_and_fractional_gib() {
        assert_eq!(capacity(0), "0.0 MiB");
        assert_eq!(capacity(512), "512.0 MiB");
        assert_eq!(capacity(1024), "1.0 GiB");
        assert_eq!(capacity(1536), "1.5 GiB");
    }

    #[test]
    fn paths_under_home_start_with_a_tilde() {
        let home = Path::new("/home/ado");
        assert_eq!(
            tilde(Path::new("/home/ado/.ssh/key"), Some(home)),
            "~/.ssh/key"
        );
        assert_eq!(tilde(Path::new("/home/ado"), Some(home)), "~");
        assert_eq!(tilde(Path::new("/etc/hosts"), Some(home)), "/etc/hosts");
        assert_eq!(
            tilde(Path::new("/home/adonis/x"), Some(home)),
            "/home/adonis/x"
        );
    }

    #[test]
    fn automatic_color_requires_a_suitable_terminal() {
        assert!(ColorMode::Auto.enabled(true, false, false));
        assert!(!ColorMode::Auto.enabled(false, false, false));
        assert!(!ColorMode::Auto.enabled(true, true, false));
        assert!(!ColorMode::Auto.enabled(true, false, true));
    }

    #[test]
    fn explicit_modes_override_environment_and_stream() {
        assert!(ColorMode::Always.enabled(false, true, true));
        assert!(!ColorMode::Never.enabled(true, false, false));
    }

    #[test]
    fn plain_status_remains_readable_without_color() {
        assert_eq!(
            Style::new(false).status("Low memory", Tone::Warning),
            "! Low memory"
        );
        assert_eq!(
            Style::new(true).paint("Busy", Tone::Warning),
            "\x1b[33mBusy\x1b[0m"
        );
    }
}
