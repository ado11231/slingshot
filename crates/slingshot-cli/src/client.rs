//! Client connections to the Agent: TCP for pairing, and authenticated SSH control for
//! everything else.

use crate::route::{self, Route};
use crate::ssh::RemoteCommand;
use crate::tunnel;
use anyhow::Context;
use slingshot_core::config::Agent;
use slingshot_core::control::{self, Request, Response};
use slingshot_core::presentation;
use slingshot_core::protocol;
use slingshot_core::step::{self, Step};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::task::JoinHandle;

/// Pair with a daemon using the code it printed. The code itself is never sent: both sides
/// prove they know it, and the Agent's details are trusted only after its proof checks out.
pub async fn pair(
    host: &str,
    port: u16,
    code: &str,
    joining: &protocol::Joining,
) -> anyhow::Result<protocol::Paired> {
    let stream = tokio::time::timeout(Duration::from_secs(10), TcpStream::connect((host, port)))
        .await
        .with_context(|| format!("Timed out reaching the slingshot daemon at {host}:{port}"))?
        .with_context(|| format!("Could not reach the slingshot daemon at {host}:{port}"))?;
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader.take(1024 * 1024));

    let (handshake, ours) = protocol::Handshake::client(code);
    exchange(&mut writer, &protocol::Request::Start { spake: ours }).await?;
    let theirs = match answer(&mut reader).await? {
        protocol::Response::Start { spake } => spake,
        protocol::Response::Error { message } if message.starts_with("Could not understand") => {
            anyhow::bail!(
                "The Agent at {host} runs an older Slingshot. Update Slingshot there, restart slingshot start, then link again"
            )
        }
        other => return Err(unexpected_pairing(other)),
    };
    let key = protocol::Handshake::finish(handshake, &theirs)?;

    let details =
        serde_json::to_string(joining).context("Could not encode this machine's details")?;
    let proof = key.prove(protocol::Side::Client, &details);
    exchange(&mut writer, &protocol::Request::Join { details, proof }).await?;
    let (details, proof) = match answer(&mut reader).await? {
        protocol::Response::Paired { details, proof } => (details, proof),
        other => return Err(unexpected_pairing(other)),
    };
    if !key.check(protocol::Side::Agent, &details, &proof) {
        anyhow::bail!(
            "Could not confirm that {host} is the Agent that printed this code. Check the code and try again. If it keeps failing, another machine on this network may be answering in its place"
        );
    }
    serde_json::from_str(&details).context("Could not understand the Agent's pairing details")
}

async fn exchange(
    writer: &mut tokio::net::tcp::OwnedWriteHalf,
    request: &protocol::Request,
) -> anyhow::Result<()> {
    let mut line = serde_json::to_string(request).context("Could not encode the request")?;
    line.push('\n');
    writer
        .write_all(line.as_bytes())
        .await
        .context("Could not send the pairing request")
}

async fn answer(
    reader: &mut BufReader<tokio::io::Take<tokio::net::tcp::OwnedReadHalf>>,
) -> anyhow::Result<protocol::Response> {
    let mut reply = String::new();
    tokio::time::timeout(Duration::from_secs(60), reader.read_line(&mut reply))
        .await
        .context("The Agent did not answer the pairing request")?
        .context("The Agent closed the connection without answering")?;
    serde_json::from_str(reply.trim()).context("Could not understand the daemon's answer")
}

/// An Agent error passes through as written. Anything else means the two sides disagree
/// about the order of pairing messages.
fn unexpected_pairing(response: protocol::Response) -> anyhow::Error {
    match response {
        protocol::Response::Error { message } => anyhow::anyhow!("{message}"),
        _ => anyhow::anyhow!(
            "The Agent answered pairing out of order. Update Slingshot on both machines, then link again"
        ),
    }
}

/// An error reported by the Agent itself, as opposed to a connection failure.
#[derive(Debug)]
pub struct Refused(pub String);

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Refused {}

/// One SSH connection to the Agent's private control socket.
pub struct Control {
    name: String,
    /// The Agent's iroh key when this connection went over iroh, for explaining a failure.
    iroh: Option<String>,
    child: Child,
    input: ChildStdin,
    output: ChildStdout,
    errors: JoinHandle<Vec<u8>>,
    /// Set once the Agent's first answer has been checked for a different release.
    checked_release: bool,
}

impl Control {
    pub async fn connect(agent: &Agent) -> anyhow::Result<Control> {
        let mut remote = RemoteCommand::to(
            agent,
            agent.program().to_string(),
            vec!["internal-control".to_string()],
        );
        remote.tty = false;
        let mut child = tokio::process::Command::new("ssh")
            .args(remote.to_ssh_args())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .context("Could not start ssh. Check that it is installed and on PATH")?;
        let input = child.stdin.take().context("Missing SSH input")?;
        let output = child.stdout.take().context("Missing SSH output")?;
        let mut stderr = child.stderr.take().context("Missing SSH error output")?;
        let errors = tokio::spawn(async move {
            let mut bytes = Vec::new();
            let _ = (&mut stderr).take(64 * 1024).read_to_end(&mut bytes).await;
            bytes
        });
        Ok(Control {
            name: agent.name.clone(),
            iroh: match route::resolve(agent) {
                Route::Iroh { key } => Some(key),
                Route::Direct { .. } => None,
            },
            child,
            input,
            output,
            errors,
            checked_release: false,
        })
    }

    /// Send a request and wait for its answer. Agent errors come back as `Refused`.
    pub async fn call(&mut self, request: Request) -> anyhow::Result<Response> {
        let limit = match request {
            Request::Begin { .. } | Request::Finish { .. } | Request::Inspect { .. } => {
                Duration::from_secs(30 * 60)
            }
            _ => Duration::from_secs(90),
        };
        let exchange = async {
            control::write_frame(&mut self.input, &request).await?;
            control::read_envelope::<_, Response>(&mut self.output).await
        };
        let answer = match tokio::time::timeout(limit, exchange).await {
            Err(_) => anyhow::bail!("{} did not answer in time", self.name),
            Ok(Ok(Some(envelope))) => {
                if !self.checked_release {
                    self.checked_release = true;
                    if let Some(warning) = release_differs(&self.name, envelope.release.as_deref())
                    {
                        presentation::warning(warning);
                    }
                }
                envelope.body
            }
            Ok(Err(error)) if error.downcast_ref::<control::Mismatch>().is_some() => {
                let mismatch = error
                    .downcast_ref::<control::Mismatch>()
                    .expect("the error was just checked to be a mismatch");
                anyhow::bail!(update_advice(&self.name, mismatch))
            }
            Ok(Err(error)) if matches!(self.child.try_wait(), Ok(None)) => {
                return Err(error.context(format!(
                    "Could not understand {}. Update Slingshot on both machines",
                    self.name
                )));
            }
            Ok(Ok(None)) | Ok(Err(_)) => return Err(self.unreachable().await),
        };
        match answer {
            Response::Error(message) => Err(Refused(message).into()),
            other => Ok(other),
        }
    }

    /// Explain why the connection ended. Over iroh a direct check names the cause, since
    /// ssh hides what the tunnel printed. Otherwise use what ssh or the remote helper said.
    async fn unreachable(&mut self) -> anyhow::Error {
        if let Some(key) = &self.iroh
            && let Some(message) = tunnel::diagnose(key).await.explain(&self.name)
        {
            return anyhow::anyhow!(message);
        }
        let _ = tokio::time::timeout(Duration::from_secs(5), self.child.wait()).await;
        let errors = tokio::time::timeout(Duration::from_secs(2), &mut self.errors)
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or_default();
        let text = String::from_utf8_lossy(&errors);
        let detail = text
            .lines()
            .map(|line| {
                line.trim()
                    .trim_start_matches(['✗'])
                    .trim_start_matches("Error:")
                    .trim()
            })
            .rfind(|line| !line.is_empty())
            .unwrap_or("the SSH connection closed");
        let hint = match detail {
            d if d.contains("command not found") || d.contains("No such file") => {
                ". Slingshot was not found on the Agent. Run slingshot link again after installing it there"
            }
            d if d.contains("unrecognized subcommand") => {
                ". Slingshot versions differ between the machines. Update Slingshot on both machines"
            }
            _ => "",
        };
        anyhow::anyhow!("Could not reach {}: {detail}{hint}", self.name)
    }

    /// Dropping the input pipe is what closes it. The helper then exits on end of input.
    pub async fn close(self) {
        let Control {
            mut child, input, ..
        } = self;
        drop(input);
        let _ = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;
    }
}

/// How to update after a protocol mismatch, naming the machine that runs the older build.
fn update_advice(name: &str, mismatch: &control::Mismatch) -> String {
    let theirs = mismatch
        .release
        .as_deref()
        .map(|release| format!(" {release}"))
        .unwrap_or_default();
    match mismatch.other_is_older() {
        true => format!(
            "{name} runs an older Slingshot{theirs} than this machine ({}). Update it there with: cargo install slingshot-cli, then restart slingshot start",
            control::RELEASE
        ),
        false => format!(
            "This machine runs an older Slingshot ({}) than {name}{theirs}. Update it with: cargo install slingshot-cli",
            control::RELEASE
        ),
    }
}

/// A warning when both machines speak the same protocol but run different releases. It
/// works, but one side is missing fixes.
fn release_differs(name: &str, theirs: Option<&str>) -> Option<String> {
    let theirs = theirs.filter(|theirs| *theirs != control::RELEASE)?;
    Some(format!(
        "{name} runs Slingshot {theirs} and this machine runs {}. Update the older one with: cargo install slingshot-cli",
        control::RELEASE
    ))
}

/// Shown until the first answer arrives, which is when ssh has actually connected.
pub fn connecting(agent: &Agent) -> Step {
    step::start(format!("Connecting to {}", agent.name))
}

pub fn connected(step: Step, agent: &Agent) {
    step.done(format!(
        "Connected to {} via {}",
        agent.name,
        route::resolve(agent).name()
    ));
}

/// `request` with a spinner while it waits, for commands that print only the answer.
pub async fn fetch(agent: &Agent, request: Request) -> anyhow::Result<Response> {
    let _connecting = connecting(agent);
    self::request(agent, request).await
}

/// Open a connection, send one request, and close it.
pub async fn request(agent: &Agent, request: Request) -> anyhow::Result<Response> {
    let mut control = Control::connect(agent).await?;
    let answer = control.call(request).await;
    control.close().await;
    answer
}

pub fn unexpected() -> anyhow::Error {
    anyhow::anyhow!("The Agent returned an unexpected response. Update Slingshot on both machines")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mismatch_names_the_machine_to_update() {
        let older = control::Mismatch {
            theirs: control::VERSION - 1,
            release: Some("0.0.9".into()),
        };
        let advice = update_advice("archbox", &older);
        assert!(
            advice.starts_with("archbox runs an older Slingshot 0.0.9 than this machine"),
            "{advice}"
        );
        assert!(advice.contains("restart slingshot start"));

        let newer = control::Mismatch {
            theirs: control::VERSION + 1,
            release: None,
        };
        let advice = update_advice("archbox", &newer);
        assert!(
            advice.starts_with("This machine runs an older Slingshot"),
            "{advice}"
        );
        assert!(
            advice.contains(" than archbox. Update it with: cargo install slingshot-cli"),
            "{advice}"
        );
    }

    #[test]
    fn only_a_different_release_is_worth_a_warning() {
        assert_eq!(release_differs("archbox", Some(control::RELEASE)), None);
        assert_eq!(release_differs("archbox", None), None);
        let warning = release_differs("archbox", Some("0.0.1")).unwrap();
        assert!(warning.starts_with("archbox runs Slingshot 0.0.1 and this machine runs"));
    }
}
