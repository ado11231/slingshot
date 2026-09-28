//! Versioned messages for the private Agent control socket.
//!
//! Requests travel through an authenticated SSH connection to a hidden helper, which
//! relays them to a Unix socket readable only by the Agent account. Each message is a
//! length prefixed JSON frame, and one connection may carry several requests. A source
//! transfer lease lives exactly as long as the connection that took it.

use crate::protocol::Health;
use crate::source::Manifest;
use crate::tools::Tool;
use anyhow::{Context, bail, ensure};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Wire version of this control protocol. Both machines must agree on it, so any change
/// to a request or response shape has to raise it.
pub const VERSION: u32 = 9;

/// Largest frame in either direction. Manifests for very large projects are the limit.
pub const MAX_FRAME: u32 = 16 * 1024 * 1024;

/// Largest environment file accepted by `slingshot env add`.
pub const MAX_ENVIRONMENT_FILE: usize = 1024 * 1024;

#[derive(Debug, Serialize, Deserialize)]
pub struct Envelope<T> {
    pub version: u32,
    pub body: T,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum Request {
    Ping,
    Health,
    /// Resource warnings worth showing before starting work.
    Warnings,
    /// Create the project storage if needed and describe it.
    Open(ProjectRef),
    /// Take the project lease and describe both manifests for a sync.
    Begin {
        project: String,
        excludes: Vec<String>,
        pull: bool,
    },
    /// Describe the Agent source without taking a lease, for previews.
    Inspect {
        project: String,
        excludes: Vec<String>,
    },
    /// Apply staged Client files, or record the result of a pull, then release.
    Finish {
        token: String,
        manifest: Manifest,
    },
    Release {
        token: String,
    },
    /// Open or return to a session: in a project's copy, or in the Agent account's home
    /// folder when `project` is `None`.
    Session {
        project: Option<String>,
    },
    Jobs {
        all: bool,
    },
    Stop {
        job: String,
    },
    EnvAdd {
        project: String,
        target: String,
        contents: Vec<u8>,
        replace: bool,
    },
    EnvList {
        project: String,
    },
    EnvRemove {
        project: String,
        target: String,
    },
    /// Remove environment files for these projects and stop accepting this Client over
    /// iroh. Source and backups stay.
    Unlink {
        client: String,
        projects: Vec<String>,
    },
    /// Which developer tools a session on the Agent would find.
    Tools,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "result", content = "data", rename_all = "snake_case")]
pub enum Response {
    Pong,
    Done,
    Error(String),
    Health(Health),
    Warnings(Vec<String>),
    Project(ProjectInfo),
    Snapshot(Snapshot),
    Synced { changed: usize },
    Session(SessionInfo),
    Jobs(Vec<Job>),
    Job(Job),
    EnvironmentFiles(Vec<String>),
    Unlinked { environment_files: usize },
    Tools(AgentTools),
}

/// The developer tools on the Agent, and what installing more of them needs there.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTools {
    pub installed: Vec<Tool>,
    /// The package manager found on the Agent, such as `pacman`.
    pub manager: Option<String>,
    /// The account's login shell, which runs install commands so they see a session's PATH.
    pub shell: String,
    /// Whether this account can write npm's global folder, so Codex installs without
    /// `sudo`. `None` when npm is not installed yet.
    #[serde(default)]
    pub npm_writable: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectRef {
    pub id: String,
    pub name: String,
    pub client: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectInfo {
    pub id: String,
    pub name: String,
    pub initialized: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    /// Present only when a lease was taken.
    pub token: Option<String>,
    pub manifest: Manifest,
    pub baseline: Manifest,
    /// Where rsync reads or writes on the Agent: the staging folder for a push, or the
    /// source folder for a pull.
    pub transfer_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub job: Job,
    pub socket: String,
    pub tmux: String,
    pub created: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    Run,
    Session,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Running,
    Completed,
    Failed,
    Stopped,
    Ended,
    Interrupted,
}

impl JobState {
    pub fn active(self) -> bool {
        self == JobState::Running
    }

    pub fn label(self) -> &'static str {
        match self {
            JobState::Running => "Running",
            JobState::Completed => "Completed",
            JobState::Failed => "Failed",
            JobState::Stopped => "Stopped",
            JobState::Ended => "Ended",
            JobState::Interrupted => "Interrupted",
        }
    }
}

/// One Slingshot run or session on the Agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub kind: JobKind,
    pub project: Option<String>,
    pub project_name: Option<String>,
    pub command: String,
    pub state: JobState,
    pub started: u64,
    pub ended: Option<u64>,
    pub exit_code: Option<i32>,
    /// The process group leader of a run, with its start time. Both must match before
    /// Slingshot signals anything, so a reused process ID is never touched.
    pub pid: Option<u32>,
    pub process_start: Option<u64>,
    /// Boot time when the job started, to tell a reboot apart from a normal exit.
    pub boot: u64,
    #[serde(default)]
    pub stop_requested: bool,
}

pub async fn write_frame<W, T>(writer: &mut W, body: &T) -> anyhow::Result<()>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let bytes = serde_json::to_vec(&Envelope {
        version: VERSION,
        body,
    })?;
    ensure!(
        bytes.len() <= MAX_FRAME as usize,
        "Message is too large. The project may have too many files"
    );
    writer.write_u32(bytes.len() as u32).await?;
    writer.write_all(&bytes).await?;
    writer.flush().await?;
    Ok(())
}

/// Read one frame. `None` means the other side closed the connection cleanly.
pub async fn read_frame<R, T>(reader: &mut R) -> anyhow::Result<Option<T>>
where
    R: AsyncRead + Unpin,
    T: DeserializeOwned,
{
    let length = match reader.read_u32().await {
        Ok(length) => length,
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    ensure!(length <= MAX_FRAME, "Control message is too large");
    let mut bytes = vec![0u8; length as usize];
    reader
        .read_exact(&mut bytes)
        .await
        .context("Control connection closed in the middle of a message")?;
    let version: VersionOnly = serde_json::from_slice(&bytes).context("Invalid control message")?;
    if version.version != VERSION {
        bail!(
            "Slingshot versions differ between the machines (protocol {} and {VERSION}). Update Slingshot on both machines",
            version.version
        );
    }
    let envelope: Envelope<T> =
        serde_json::from_slice(&bytes).context("Invalid control message")?;
    Ok(Some(envelope.body))
}

#[derive(Deserialize)]
struct VersionOnly {
    version: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn frames_round_trip_and_enforce_version_and_size() {
        let (mut a, mut b) = tokio::io::duplex(1024 * 1024);
        write_frame(&mut a, &Request::Jobs { all: true })
            .await
            .unwrap();
        let back: Request = read_frame(&mut b).await.unwrap().unwrap();
        assert!(matches!(back, Request::Jobs { all: true }));

        let old = serde_json::to_vec(&Envelope {
            version: 2,
            body: Request::Ping,
        })
        .unwrap();
        a.write_u32(old.len() as u32).await.unwrap();
        a.write_all(&old).await.unwrap();
        let error = read_frame::<_, Request>(&mut b)
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("Update Slingshot"), "{error}");

        a.write_u32(MAX_FRAME + 1).await.unwrap();
        assert!(read_frame::<_, Request>(&mut b).await.is_err());

        drop(a);
        let (mut c, d) = tokio::io::duplex(64);
        drop(d);
        assert!(read_frame::<_, Request>(&mut c).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn the_tools_reply_round_trips() {
        let (mut a, mut b) = tokio::io::duplex(1024);
        let reply = Response::Tools(AgentTools {
            installed: vec![Tool::Git, Tool::ClaudeCode],
            manager: Some("pacman".into()),
            shell: "/usr/bin/bash".into(),
            npm_writable: Some(false),
        });
        write_frame(&mut a, &reply).await.unwrap();

        let Some(Response::Tools(back)) = read_frame(&mut b).await.unwrap() else {
            panic!("expected a tools reply");
        };
        assert_eq!(back.installed, vec![Tool::Git, Tool::ClaudeCode]);
        assert_eq!(back.manager.as_deref(), Some("pacman"));
        assert_eq!(back.shell, "/usr/bin/bash");
        assert_eq!(back.npm_writable, Some(false));
    }
}
