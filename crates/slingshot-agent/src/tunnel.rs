//! The Agent's iroh endpoint. Paired Clients reach this machine's sshd through it from any
//! network. Each accepted stream is joined to sshd on loopback and nowhere else, so iroh
//! only ever carries ssh, which still decides who logs in.

use crate::clients;
use anyhow::Context;
use iroh::endpoint::{Incoming, RecvStream, SendStream, presets};
use iroh::{Endpoint, SecretKey};
use slingshot_core::tunnel::{ALPN, NOT_PAIRED};
use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::time::Duration;
use tokio::net::TcpStream;
use tracing::{debug, info, warn};

/// The Agent's own sshd. Slingshot already requires it on the usual port.
pub const SSHD: SocketAddr = SocketAddr::new(std::net::IpAddr::V4(Ipv4Addr::LOCALHOST), 22);

/// How long `start` waits to hear from a relay before saying other networks cannot reach
/// the Agent yet. The endpoint keeps trying afterwards.
const ONLINE_WAIT: Duration = Duration::from_secs(15);

/// Bind the endpoint on iroh's public relays and start accepting in the background.
pub async fn start(identity: SecretKey, root: PathBuf) -> anyhow::Result<Endpoint> {
    let endpoint = Endpoint::builder(presets::N0)
        .secret_key(identity)
        .alpns(vec![ALPN.to_vec()])
        .bind()
        .await
        .context("Could not start the iroh endpoint")?;
    tokio::spawn(accept(endpoint.clone(), root, SSHD));
    Ok(endpoint)
}

/// Whether a relay answered within `ONLINE_WAIT`, which is what makes the Agent reachable
/// from other networks.
pub async fn online(endpoint: &Endpoint) -> bool {
    tokio::time::timeout(ONLINE_WAIT, endpoint.online())
        .await
        .is_ok()
}

pub async fn accept(endpoint: Endpoint, root: PathBuf, sshd: SocketAddr) {
    while let Some(incoming) = endpoint.accept().await {
        let root = root.clone();
        tokio::spawn(async move {
            if let Err(e) = handle(incoming, root, sshd).await {
                warn!("iroh connection ended: {e:#}");
            }
        });
    }
}

/// Refuse keys that never paired, then join every stream the Client opens to sshd. ssh ends
/// its ProxyCommand without closing the connection, so a stream that stops abruptly is how
/// a finished command normally looks from here.
async fn handle(incoming: Incoming, root: PathBuf, sshd: SocketAddr) -> anyhow::Result<()> {
    let connection = incoming.await.context("iroh handshake failed")?;
    let client = connection.remote_id();
    if !clients::allowed(&root, &client)? {
        connection.close(1u32.into(), NOT_PAIRED.as_bytes());
        anyhow::bail!("refused {}, which has not paired", client.fmt_short());
    }
    info!("iroh connection from {}", client.fmt_short());
    while let Ok((send, recv)) = connection.accept_bi().await {
        tokio::spawn(async move {
            match TcpStream::connect(sshd).await {
                Ok(server) => {
                    if let Err(e) = splice(send, recv, server).await {
                        debug!("iroh stream to sshd ended: {e:#}");
                    }
                }
                Err(e) => warn!("Could not reach sshd at {sshd}: {e}"),
            }
        });
    }
    Ok(())
}

async fn splice(send: SendStream, recv: RecvStream, mut server: TcpStream) -> anyhow::Result<()> {
    let mut stream = tokio::io::join(recv, send);
    tokio::io::copy_bidirectional(&mut stream, &mut server).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::Root;
    use iroh::RelayMode;
    use tokio::net::TcpListener;

    async fn endpoint(alpns: Vec<Vec<u8>>) -> Endpoint {
        Endpoint::builder(presets::Minimal)
            .relay_mode(RelayMode::Disabled)
            .alpns(alpns)
            .bind()
            .await
            .unwrap()
    }

    /// Stands in for sshd: echoes whatever arrives.
    async fn echo_server() -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let (mut read, mut write) = stream.split();
                    let _ = tokio::io::copy(&mut read, &mut write).await;
                });
            }
        });
        addr
    }

    async fn agent(root: &Root) -> Endpoint {
        let agent = endpoint(vec![ALPN.to_vec()]).await;
        tokio::spawn(accept(agent.clone(), root.0.clone(), echo_server().await));
        agent
    }

    #[tokio::test]
    async fn a_paired_client_reaches_sshd() {
        let root = Root::new();
        let agent = agent(&root).await;
        let client = endpoint(Vec::new()).await;
        clients::allow(&root.0, "laptop", &client.id().to_string()).unwrap();

        let connection = client.connect(agent.addr(), ALPN).await.unwrap();
        let (mut send, mut recv) = connection.open_bi().await.unwrap();
        send.write_all(b"SSH-2.0-test\r\n").await.unwrap();
        send.finish().unwrap();
        let echoed = recv.read_to_end(1024).await.unwrap();

        assert_eq!(echoed, b"SSH-2.0-test\r\n");
    }

    #[tokio::test]
    async fn a_client_that_never_paired_is_refused() {
        let root = Root::new();
        let agent = agent(&root).await;
        let stranger = endpoint(Vec::new()).await;

        let connection = stranger.connect(agent.addr(), ALPN).await.unwrap();
        let reason = connection.closed().await;

        assert!(
            reason.to_string().contains("not paired"),
            "closed with: {reason}"
        );
    }
}
