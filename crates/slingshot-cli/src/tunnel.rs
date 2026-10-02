//! The Client end of reaching an Agent over iroh. ssh starts `slingshot internal-tunnel` as
//! its ProxyCommand and speaks to the Agent's sshd through this process's standard input
//! and output, so ssh itself is unchanged whichever path carries it.

use crate::project;
use anyhow::Context;
use iroh::endpoint::{Connection, presets};
use iroh::{Endpoint, EndpointAddr};
use slingshot_core::tunnel::{self, ALPN};
use std::path::PathBuf;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};

/// How long a shared iroh connection outlives its last ssh call. Long enough to carry one
/// command's control, sync, and run calls, short enough that an Agent that went away is
/// noticed on the next command rather than minutes later.
pub const SHARED_FOR_SECONDS: u32 = 30;

/// The ProxyCommand line for reaching the Agent with iroh key `key`, which the route has
/// already checked. ssh runs it through a shell after expanding its own `%` tokens, so
/// every part is quoted and every `%` doubled.
pub fn proxy_command(key: &str) -> String {
    let exe = std::env::current_exe()
        .ok()
        .and_then(|path| path.to_str().map(str::to_string))
        .unwrap_or_else(|| "slingshot".to_string());
    proxy_line(&exe, key)
}

/// Where ssh keeps the connection it shares between calls over iroh, so one command pays
/// for the iroh handshake once. Kept in a private folder and named by the Agent's key, so
/// each Agent gets its own. `None` when no folder fits or none can be made private, which
/// only means no sharing.
pub fn shared_socket(key: &str) -> Option<PathBuf> {
    let user = std::env::var("USER").unwrap_or_else(|_| "user".to_string());
    let name = format!("{}.sock", &key[..key.len().min(12)]);
    [std::env::temp_dir(), PathBuf::from("/tmp")]
        .into_iter()
        .map(|base| base.join(format!("slingshot-{user}")))
        .find(|dir| {
            dir.join(&name).as_os_str().len() + SOCKET_SUFFIX < SOCKET_LIMIT
                && slingshot_core::storage::private_dir(dir).is_ok()
        })
        .map(|dir| dir.join(name))
}

/// The longest Unix socket path macOS accepts, and what ssh appends to the name while it
/// creates the socket.
const SOCKET_LIMIT: usize = 104;
const SOCKET_SUFFIX: usize = 17;

fn proxy_line(exe: &str, key: &str) -> String {
    shell_words::join([exe, "internal-tunnel", key]).replace('%', "%%")
}

/// How long to wait for the Agent to answer, and for this machine to reach a relay.
const CONNECT_WAIT: Duration = Duration::from_secs(20);
const ONLINE_WAIT: Duration = Duration::from_secs(10);

/// Connect to the Agent and carry ssh's bytes until either side closes.
pub async fn run(key: String) -> anyhow::Result<i32> {
    let agent = tunnel::public_key(&key)?;
    let endpoint = bind().await?;
    let connection = tokio::time::timeout(CONNECT_WAIT, endpoint.connect(agent, ALPN))
        .await
        .context("The Agent did not answer over iroh")?
        .context("Could not reach the Agent over iroh")?;
    let result = pipe(&connection, tokio::io::stdin(), tokio::io::stdout()).await;
    connection.close(0u32.into(), b"done");
    endpoint.close().await;
    result.map(|_| 0)
}

async fn bind() -> anyhow::Result<Endpoint> {
    let identity = tunnel::identity(&project::client_root()?)?;
    Endpoint::builder(presets::N0)
        .secret_key(identity)
        .bind()
        .await
        .context("Could not start iroh")
}

/// Why an Agent could not be reached over iroh. Found by trying once directly, because ssh
/// hides whatever its ProxyCommand printed.
#[derive(Debug, PartialEq)]
pub enum Diagnosis {
    NoInternet,
    Offline,
    NotPaired,
    Reachable,
}

impl Diagnosis {
    /// A sentence naming the cause and the fix, or `None` when iroh is fine and the problem
    /// lies with ssh, whose own message is then the useful one.
    pub fn explain(&self, name: &str) -> Option<String> {
        match self {
            Diagnosis::NoInternet => Some(format!(
                "Could not reach {name}, because this machine cannot reach iroh's relays. Check its internet connection"
            )),
            Diagnosis::Offline => Some(format!(
                "{name} is not reachable. It may be off or asleep, or slingshot start is not running there"
            )),
            Diagnosis::NotPaired => Some(format!(
                "{name} no longer accepts this machine. Run slingshot link again from the same network as {name}"
            )),
            Diagnosis::Reachable => None,
        }
    }
}

pub async fn diagnose(key: &str) -> Diagnosis {
    let (Ok(agent), Ok(endpoint)) = (tunnel::public_key(key), bind().await) else {
        return Diagnosis::Offline;
    };
    let diagnosis = match tokio::time::timeout(ONLINE_WAIT, endpoint.online()).await {
        Err(_) => Diagnosis::NoInternet,
        Ok(()) => check(&endpoint, agent, CONNECT_WAIT).await,
    };
    endpoint.close().await;
    diagnosis
}

/// A paired Client's connection stays open, while a refused one is closed straight after
/// the handshake, so a short wait for the close tells the two apart.
async fn check(endpoint: &Endpoint, agent: impl Into<EndpointAddr>, wait: Duration) -> Diagnosis {
    let Ok(Ok(connection)) = tokio::time::timeout(wait, endpoint.connect(agent, ALPN)).await else {
        return Diagnosis::Offline;
    };
    let refused = matches!(
        tokio::time::timeout(Duration::from_secs(2), connection.closed()).await,
        Ok(reason) if reason.to_string().contains(tunnel::NOT_PAIRED)
    );
    connection.close(0u32.into(), b"done");
    match refused {
        true => Diagnosis::NotPaired,
        false => Diagnosis::Reachable,
    }
}

async fn pipe(
    connection: &Connection,
    input: impl AsyncRead + Unpin,
    output: impl AsyncWrite + Unpin,
) -> anyhow::Result<()> {
    let (send, recv) = connection
        .open_bi()
        .await
        .context("Could not open a stream to the Agent")?;
    let mut remote = tokio::io::join(recv, send);
    let mut local = tokio::io::join(input, output);
    tokio::io::copy_bidirectional(&mut local, &mut remote).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use iroh::RelayMode;
    use iroh::SecretKey;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn the_proxy_command_names_the_key() {
        let key = SecretKey::generate().public().to_string();
        let command = proxy_command(&key);

        assert!(
            command.ends_with(&format!(" internal-tunnel {key}")),
            "{command}"
        );
    }

    #[test]
    fn nothing_in_the_command_can_become_shell_syntax_or_an_ssh_token() {
        assert_eq!(
            proxy_line("/App 100%/slingshot", "k"),
            "'/App 100%%/slingshot' internal-tunnel k"
        );
    }

    /// Unix sockets fail to bind past roughly 104 bytes of path on macOS.
    #[test]
    fn the_shared_socket_path_is_short_and_private() {
        let key = SecretKey::generate().public().to_string();
        let path = shared_socket(&key).unwrap();

        assert!(
            path.as_os_str().len() + SOCKET_SUFFIX < SOCKET_LIMIT,
            "{}",
            path.display()
        );
        let mode = std::fs::metadata(path.parent().unwrap())
            .unwrap()
            .permissions();
        assert_eq!(
            std::os::unix::fs::PermissionsExt::mode(&mode) & 0o777,
            0o700
        );
    }

    async fn endpoint(alpns: Vec<Vec<u8>>) -> Endpoint {
        Endpoint::builder(presets::Minimal)
            .relay_mode(RelayMode::Disabled)
            .alpns(alpns)
            .bind()
            .await
            .unwrap()
    }

    /// Stands in for the Agent: echoes every stream back.
    async fn echo_agent() -> Endpoint {
        let agent = endpoint(vec![ALPN.to_vec()]).await;
        let accepting = agent.clone();
        tokio::spawn(async move {
            while let Some(incoming) = accepting.accept().await {
                let connection = incoming.await.unwrap();
                while let Ok((mut send, mut recv)) = connection.accept_bi().await {
                    tokio::io::copy(&mut recv, &mut send).await.unwrap();
                    send.finish().unwrap();
                }
            }
        });
        agent
    }

    fn agent_root() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "slingshot-diagnose-{}",
            slingshot_core::storage::new_id()
        ));
        slingshot_core::storage::private_dir(&root).unwrap();
        root
    }

    /// The Agent's own accept loop, with sshd pointed at a port nothing answers on.
    async fn real_agent(root: &std::path::Path) -> Endpoint {
        let agent = endpoint(vec![ALPN.to_vec()]).await;
        let sshd = std::net::SocketAddr::from(([127, 0, 0, 1], 9));
        tokio::spawn(slingshot_agent::tunnel::accept(
            agent.clone(),
            root.to_path_buf(),
            sshd,
        ));
        agent
    }

    #[tokio::test]
    async fn a_paired_client_is_diagnosed_as_reachable() {
        let root = agent_root();
        let agent = real_agent(&root).await;
        let client = endpoint(Vec::new()).await;
        slingshot_agent::clients::allow(&root, "laptop", &client.id().to_string()).unwrap();

        let diagnosis = check(&client, agent.addr(), Duration::from_secs(5)).await;
        std::fs::remove_dir_all(&root).unwrap();

        assert_eq!(diagnosis, Diagnosis::Reachable);
    }

    #[tokio::test]
    async fn an_unpaired_client_is_told_to_link_again() {
        let root = agent_root();
        let agent = real_agent(&root).await;
        let client = endpoint(Vec::new()).await;

        let diagnosis = check(&client, agent.addr(), Duration::from_secs(5)).await;
        std::fs::remove_dir_all(&root).unwrap();

        assert_eq!(diagnosis, Diagnosis::NotPaired);
    }

    #[tokio::test]
    async fn an_agent_that_went_away_is_offline() {
        let agent = endpoint(vec![ALPN.to_vec()]).await;
        let addr = agent.addr();
        agent.close().await;
        let client = endpoint(Vec::new()).await;

        assert_eq!(
            check(&client, addr, Duration::from_secs(2)).await,
            Diagnosis::Offline
        );
    }

    #[test]
    fn every_failure_names_the_agent_and_a_fix() {
        for diagnosis in [
            Diagnosis::NoInternet,
            Diagnosis::Offline,
            Diagnosis::NotPaired,
        ] {
            let message = diagnosis.explain("archbox").unwrap();
            assert!(message.contains("archbox"), "{message}");
        }
        assert_eq!(Diagnosis::Reachable.explain("archbox"), None);
    }

    #[tokio::test]
    async fn bytes_cross_in_both_directions() {
        let agent = echo_agent().await;
        let client = endpoint(Vec::new()).await;
        let connection = client.connect(agent.addr(), ALPN).await.unwrap();
        let (mut ssh_in, input) = tokio::io::duplex(64);
        let (output, mut ssh_out) = tokio::io::duplex(64);

        ssh_in.write_all(b"SSH-2.0-test\r\n").await.unwrap();
        drop(ssh_in);
        pipe(&connection, input, output).await.unwrap();
        let mut echoed = Vec::new();
        ssh_out.read_to_end(&mut echoed).await.unwrap();

        assert_eq!(echoed, b"SSH-2.0-test\r\n");
    }
}
