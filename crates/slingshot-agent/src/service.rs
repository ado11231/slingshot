//! The private Agent control socket.
//!
//! SSH authenticates every caller. A hidden `slingshot internal-control` helper, started
//! through that SSH login, relays bytes between the connection and a Unix socket that
//! only the Agent account can open. The Agent OS account is the trust boundary.

use crate::{jobs, projects};
use anyhow::{Context, bail};
use slingshot_core::control::{self, AgentTools, Request, Response};
use slingshot_core::{preflight, storage, telemetry, tools};
use std::fs::File;
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::net::{UnixListener, UnixStream};
use tracing::warn;

/// Time allowed for the first request after connecting.
const FIRST_REQUEST: Duration = Duration::from_secs(15);

/// Idle time allowed between requests. Clients send pings during long transfers, so a
/// vanished Client releases its lease within this time.
const IDLE: Duration = Duration::from_secs(90);

const WRITE_TIMEOUT: Duration = Duration::from_secs(30);

/// Agent storage, separate from any Client data kept on the same machine.
pub fn root() -> anyhow::Result<PathBuf> {
    Ok(storage::data_dir()?.join("agent"))
}

pub fn socket(root: &Path) -> PathBuf {
    root.join("control.sock")
}

/// Keeps the daemon lock for as long as `slingshot start` runs.
pub struct Service {
    _guard: File,
}

pub fn start(name: String) -> anyhow::Result<Service> {
    let root = root()?;
    storage::private_dir(&root)?;
    let guard = storage::try_lock(&root.join("daemon.lock"))?
        .context("Another slingshot start is already running for this account")?;
    let path = socket(&root);
    if path.as_os_str().len() >= 100 {
        bail!(
            "The Slingshot storage path is too long for a control socket: {}. Use an account with a shorter home directory",
            path.display()
        );
    }
    if let Ok(meta) = std::fs::symlink_metadata(&path) {
        if !meta.file_type().is_socket() {
            bail!("{} exists and is not a socket", path.display());
        }
        std::fs::remove_file(&path)?;
    }
    let listener =
        UnixListener::bind(&path).with_context(|| format!("Could not open {}", path.display()))?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    tokio::spawn(accept(listener, Arc::new(root), Arc::new(name)));
    Ok(Service { _guard: guard })
}

async fn accept(listener: UnixListener, root: Arc<PathBuf>, name: Arc<String>) {
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let (root, name) = (root.clone(), name.clone());
                tokio::spawn(async move {
                    if let Err(error) = connection(stream, root, name).await {
                        warn!("control connection failed: {error:#}");
                    }
                });
            }
            Err(error) => {
                warn!("could not accept a control connection: {error}");
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }
    }
}

async fn connection(
    stream: UnixStream,
    root: Arc<PathBuf>,
    name: Arc<String>,
) -> anyhow::Result<()> {
    let peer = stream.peer_cred()?;
    if peer.uid() != unsafe { libc::getuid() } {
        bail!("Refused a control connection from another account");
    }
    let (mut reader, mut writer) = stream.into_split();
    let mut lease: Option<projects::Lease> = None;
    let mut limit = FIRST_REQUEST;

    loop {
        let request =
            match tokio::time::timeout(limit, control::read_frame::<_, Request>(&mut reader)).await
            {
                Err(_) | Ok(Ok(None)) => break,
                Ok(Err(error)) => {
                    let reply = Response::Error(format!("{error:#}"));
                    let _ = tokio::time::timeout(
                        WRITE_TIMEOUT,
                        control::write_frame(&mut writer, &reply),
                    )
                    .await;
                    break;
                }
                Ok(Ok(Some(request))) => request,
            };
        limit = IDLE;
        let (response, returned) =
            dispatch(request, root.clone(), name.clone(), lease.take()).await;
        lease = returned;
        let reply = response.unwrap_or_else(|error| Response::Error(format!("{error:#}")));
        tokio::time::timeout(WRITE_TIMEOUT, control::write_frame(&mut writer, &reply))
            .await
            .context("Timed out answering a control request")??;
    }
    let _ = writer.shutdown().await;
    if let Some(lease) = lease {
        let _ = tokio::task::spawn_blocking(move || drop(lease)).await;
    }
    Ok(())
}

type Handled = (anyhow::Result<Response>, Option<projects::Lease>);

async fn dispatch(
    request: Request,
    root: Arc<PathBuf>,
    name: Arc<String>,
    lease: Option<projects::Lease>,
) -> Handled {
    if let Request::Ping = request {
        return (Ok(Response::Pong), lease);
    }
    let joined =
        tokio::task::spawn_blocking(move || with_lease(&root, &name, request, lease)).await;
    match joined {
        Ok(outcome) => outcome,
        Err(_) => (
            Err(anyhow::anyhow!(
                "The Agent failed while handling the request"
            )),
            None,
        ),
    }
}

fn with_lease(
    root: &Path,
    name: &str,
    request: Request,
    lease: Option<projects::Lease>,
) -> Handled {
    match request {
        Request::Begin {
            project,
            excludes,
            pull,
        } => {
            if lease.is_some() {
                return (
                    Err(anyhow::anyhow!("A sync is already open on this connection")),
                    lease,
                );
            }
            match projects::begin(root, &project, excludes, pull) {
                Ok((lease, snapshot)) => (Ok(Response::Snapshot(snapshot)), Some(lease)),
                Err(error) => (Err(error), None),
            }
        }
        Request::Finish { token, manifest } => match lease {
            Some(lease) => (
                projects::finish(root, lease, &token, manifest)
                    .map(|changed| Response::Synced { changed }),
                None,
            ),
            None => (
                Err(anyhow::anyhow!("No sync is open. Run the sync again")),
                None,
            ),
        },
        Request::Release { token } => match lease {
            Some(lease) if lease.token != token => (
                Err(anyhow::anyhow!("Sync token does not match")),
                Some(lease),
            ),
            _ => (Ok(Response::Done), None),
        },
        other => (respond(root, name, other), lease),
    }
}

fn respond(root: &Path, name: &str, request: Request) -> anyhow::Result<Response> {
    Ok(match request {
        Request::Health => Response::Health(telemetry::health(Some(root))),
        Request::Warnings => Response::Warnings(telemetry::warnings(root)),
        Request::Open(project) => Response::Project(projects::open(root, &project)?),
        Request::Inspect { project, excludes } => {
            Response::Snapshot(projects::inspect(root, &project, &excludes)?)
        }
        Request::Session { project } => {
            Response::Session(jobs::session(root, name, project.as_deref())?)
        }
        Request::Jobs { all } => Response::Jobs(jobs::list(root, all)?),
        Request::Stop { job } => Response::Job(jobs::stop(root, &job)?),
        Request::EnvAdd {
            project,
            target,
            contents,
            replace,
        } => {
            projects::env_add(root, &project, &target, &contents, replace)?;
            Response::Done
        }
        Request::EnvList { project } => {
            Response::EnvironmentFiles(projects::env_list(root, &project)?)
        }
        Request::EnvRemove { project, target } => {
            projects::env_remove(root, &project, &target)?;
            Response::Done
        }
        Request::Unlink {
            client,
            projects: ids,
        } => {
            let environment_files = projects::unlink(root, &ids)?;
            crate::clients::forget(root, &client)?;
            Response::Unlinked { environment_files }
        }
        Request::Tools => Response::Tools(agent_tools(root)?),
        Request::Ping
        | Request::Begin { .. }
        | Request::Finish { .. }
        | Request::Release { .. } => {
            bail!("Unexpected control request")
        }
    })
}

/// The tools a session on this account would find. The probe runs in the login shell,
/// because tools such as rustup and Claude Code add themselves to PATH in startup files
/// that `slingshot start` may never have read.
fn agent_tools(root: &Path) -> anyhow::Result<AgentTools> {
    let shell = jobs::login_shell(root);
    let output = std::process::Command::new(&shell)
        .args(["-l", "-c", &tools::probe_script()])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .with_context(|| format!("Could not start the login shell {shell} on the Agent"))?;
    let output = String::from_utf8_lossy(&output.stdout);
    Ok(AgentTools {
        installed: tools::found(&output),
        manager: preflight::package_manager().map(str::to_string),
        shell,
        npm_writable: tools::npm_writable(&output),
    })
}

/// The hidden helper run over SSH. It only relays bytes; the daemon enforces versions,
/// limits, and timeouts.
pub async fn bridge() -> anyhow::Result<i32> {
    let path = socket(&root()?);
    let stream = UnixStream::connect(&path)
        .await
        .context("Slingshot is not running on the Agent. Run slingshot start there")?;
    let (mut from_agent, mut to_agent) = stream.into_split();
    let upload = async {
        let mut stdin = tokio::io::stdin();
        let _ = tokio::io::copy(&mut stdin, &mut to_agent).await;
        let _ = to_agent.shutdown().await;
        std::future::pending::<()>().await
    };
    let download = async {
        let mut stdout = tokio::io::stdout();
        let result = tokio::io::copy(&mut from_agent, &mut stdout).await;
        let _ = stdout.flush().await;
        result
    };
    tokio::select! {
        _ = upload => {}
        result = download => { result?; }
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::Root;
    use slingshot_core::control::ProjectRef;

    async fn call(stream: &mut UnixStream, request: Request) -> Response {
        control::write_frame(stream, &request).await.unwrap();
        control::read_frame(stream).await.unwrap().unwrap()
    }

    /// Socket paths have a small length limit, so tests use a short name under /tmp.
    fn serve(root: &Root) -> PathBuf {
        let path = PathBuf::from(format!("/tmp/slingshot-{}.sock", &storage::new_id()[..12]));
        let listener = UnixListener::bind(&path).unwrap();
        tokio::spawn(accept(
            listener,
            Arc::new(root.0.clone()),
            Arc::new("box".into()),
        ));
        path
    }

    #[tokio::test]
    async fn one_connection_carries_several_requests() {
        let root = Root::new();
        let path = serve(&root);
        let mut stream = UnixStream::connect(&path).await.unwrap();
        let _ = std::fs::remove_file(&path);
        assert!(matches!(
            call(&mut stream, Request::Ping).await,
            Response::Pong
        ));
        assert!(
            matches!(call(&mut stream, Request::Jobs { all: true }).await, Response::Jobs(jobs) if jobs.is_empty())
        );
        let error = call(&mut stream, Request::Stop { job: "zz".into() }).await;
        assert!(matches!(error, Response::Error(_)));
    }

    #[tokio::test]
    async fn closing_the_connection_releases_the_lease() {
        let root = Root::new();
        let path = serve(&root);
        let id = storage::new_id();
        let project = ProjectRef {
            id: id.clone(),
            name: "app".into(),
            client: "laptop".into(),
        };
        let begin = Request::Begin {
            project: id.clone(),
            excludes: Vec::new(),
            pull: false,
        };

        let mut first = UnixStream::connect(&path).await.unwrap();
        let mut second = UnixStream::connect(&path).await.unwrap();
        let _ = std::fs::remove_file(&path);
        assert!(matches!(
            call(&mut first, Request::Open(project)).await,
            Response::Project(_)
        ));
        let Response::Snapshot(snapshot) = call(&mut first, begin.clone()).await else {
            panic!("expected a snapshot");
        };
        let Response::Error(message) = call(&mut second, begin.clone()).await else {
            panic!("expected the project to be busy");
        };
        assert!(message.contains("busy"), "{message}");

        drop(first);
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(!Path::new(&snapshot.transfer_path).exists());
        assert!(matches!(
            call(&mut second, begin).await,
            Response::Snapshot(_)
        ));
    }

    #[tokio::test]
    async fn a_mismatched_version_is_answered_with_an_update_message() {
        let root = Root::new();
        let path = serve(&root);
        let mut stream = UnixStream::connect(&path).await.unwrap();
        let _ = std::fs::remove_file(&path);
        let body = serde_json::to_vec(&control::Envelope {
            version: 1,
            body: Request::Ping,
        })
        .unwrap();
        stream.write_u32(body.len() as u32).await.unwrap();
        stream.write_all(&body).await.unwrap();
        let reply: Response = control::read_frame(&mut stream).await.unwrap().unwrap();
        assert!(matches!(reply, Response::Error(message) if message.contains("Update Slingshot")));
    }

    #[tokio::test]
    async fn an_oversized_frame_is_refused() {
        let root = Root::new();
        let path = serve(&root);
        let mut stream = UnixStream::connect(&path).await.unwrap();
        let _ = std::fs::remove_file(&path);
        stream.write_u32(control::MAX_FRAME + 1).await.unwrap();
        let reply: Response = control::read_frame(&mut stream).await.unwrap().unwrap();
        assert!(matches!(reply, Response::Error(message) if message.contains("too large")));
    }
}
