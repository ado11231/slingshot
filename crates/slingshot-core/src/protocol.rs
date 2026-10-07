//! Pairing messages, sent as newline separated JSON over TCP. Everything after pairing
//! uses the authenticated control channel in `control`.
//!
//! The pairing code never crosses the network. Both sides turn it into a shared key with
//! SPAKE2, then prove what they send with that key. Someone watching learns nothing they
//! can use, and a machine pretending to be the Agent cannot prove it knows the code.

use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use spake2::{Ed25519Group, Identity, Password, Spake2};

/// The pairing port `slingshot start` tries first.
pub const DEFAULT_PORT: u16 = 7433;

/// What the Client sends. A pairing is `Start`, then `Join`, on one connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Request {
    /// Retired. Older Clients sent these unauthenticated; the daemon now answers with
    /// an instruction to update. Health now travels over the control channel.
    Info,
    Health,
    /// Retired. Older Clients sent the code itself here. Its fields are ignored, so the
    /// daemon can still answer it with an instruction to update.
    Pair {},
    /// The Client's first SPAKE2 message.
    Start {
        spake: Vec<u8>,
    },
    /// The Client's details as JSON text, and its proof over that exact text.
    Join {
        details: String,
        proof: Vec<u8>,
    },
}

/// What the Agent answers. Every failure is an `Error` carrying a sentence meant for a human.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Response {
    /// The Agent's first SPAKE2 message.
    Start {
        spake: Vec<u8>,
    },
    /// A `Paired` as JSON text, and the Agent's proof over that exact text.
    Paired {
        details: String,
        proof: Vec<u8>,
    },
    Error {
        message: String,
    },
}

/// Who the Client is, sent in `Join`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Joining {
    pub client: String,
    pub public_key: String,
    /// The Client's iroh public key, which the Agent will accept connections from.
    pub iroh: Option<String>,
}

/// Which side made a proof. Each side proves with its own label, so a proof cannot be
/// sent back to the side that made it.
#[derive(Debug, Clone, Copy)]
pub enum Side {
    Client,
    Agent,
}

impl Side {
    fn label(self) -> &'static [u8] {
        match self {
            Side::Client => b"slingshot client",
            Side::Agent => b"slingshot agent",
        }
    }
}

/// One side of a SPAKE2 exchange, started from the pairing code.
pub struct Handshake(Spake2<Ed25519Group>);

impl Handshake {
    /// Start as the Client. Returns the message to send to the Agent.
    pub fn client(code: &str) -> (Handshake, Vec<u8>) {
        let (state, message) = Spake2::<Ed25519Group>::start_a(
            &Password::new(code.as_bytes()),
            &Identity::new(Side::Client.label()),
            &Identity::new(Side::Agent.label()),
        );
        (Handshake(state), message)
    }

    /// Start as the Agent. Returns the message to send to the Client.
    pub fn agent(code: &str) -> (Handshake, Vec<u8>) {
        let (state, message) = Spake2::<Ed25519Group>::start_b(
            &Password::new(code.as_bytes()),
            &Identity::new(Side::Client.label()),
            &Identity::new(Side::Agent.label()),
        );
        (Handshake(state), message)
    }

    /// Combine the other side's message into the shared key. Both sides get the same key
    /// only when both used the same code.
    pub fn finish(self, theirs: &[u8]) -> anyhow::Result<Key> {
        self.0
            .finish(theirs)
            .map(Key)
            .map_err(|error| anyhow::anyhow!("The pairing handshake was malformed: {error}"))
    }
}

/// The key both sides share after a handshake.
pub struct Key(Vec<u8>);

impl Key {
    fn mac(&self, side: Side, details: &str) -> Hmac<Sha256> {
        let mut mac =
            Hmac::<Sha256>::new_from_slice(&self.0).expect("HMAC accepts a key of any length");
        mac.update(side.label());
        mac.update(details.as_bytes());
        mac
    }

    pub fn prove(&self, side: Side, details: &str) -> Vec<u8> {
        self.mac(side, details).finalize().into_bytes().to_vec()
    }

    /// Whether `proof` was made by `side` over `details` with this key. The comparison
    /// takes the same time however much of the proof matches.
    pub fn check(&self, side: Side, details: &str, proof: &[u8]) -> bool {
        self.mac(side, details).verify_slice(proof).is_ok()
    }
}

/// Everything `link` needs to be able to reach the Agent from now on.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Paired {
    /// The display name chosen on the Agent. Config stores this so the
    /// "running on" line stays stable even if the machine gets renamed.
    pub name: String,
    /// The account the installed key belongs to, so ssh knows who to log in as.
    pub user: String,
    /// The Agent's ssh host keys, so the Client can recognize it later without
    /// anybody being asked to eyeball a fingerprint.
    pub host_keys: Vec<String>,
    /// The Agent's Slingshot program path, so the Client can start helpers over SSH.
    pub program: Option<String>,
    /// Every address the Agent answers on, so the Client can switch to another one when the
    /// address it paired on stops answering.
    #[serde(default)]
    pub addresses: Vec<String>,
    /// The Agent's iroh public key, which reaches it from any network.
    #[serde(default)]
    pub iroh: Option<String>,
    pub specs: Specs,
}

/// Static facts. These change only when hardware or the OS changes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Specs {
    pub name: String,
    pub os: String,
    pub kernel: String,
    pub cpu: String,
    pub cores: usize,
    pub memory_mib: u64,
    pub disk_total_mib: u64,
    /// Last, because toml cannot put a plain value after a list of tables.
    pub gpus: Vec<Gpu>,
}

/// A graphics card as reported at pairing time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Gpu {
    pub name: String,
    pub vram_mib: Option<u64>,
}

/// Current resource usage and available capacity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Health {
    pub cpu_percent: f32,
    pub memory_used_mib: u64,
    pub memory_total_mib: u64,
    pub swap_total_mib: u64,
    pub disk_free_mib: u64,
    /// Free space where Slingshot keeps project copies and build output.
    #[serde(default)]
    pub workspace_free_mib: Option<u64>,
    pub gpus: Vec<GpuHealth>,
    /// Why GPU numbers are missing when nvidia-smi is installed but failing.
    #[serde(default)]
    pub gpu_problem: Option<String>,
}

/// Live GPU numbers. VRAM is reported as free rather than total, because a
/// desktop session holds a few hundred megabytes even when the Agent looks idle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpuHealth {
    pub name: String,
    pub vram_free_mib: Option<u64>,
    pub vram_total_mib: Option<u64>,
    pub utilization_percent: Option<u32>,
    pub temperature_c: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(client_code: &str, agent_code: &str) -> (Key, Key) {
        let (client, to_agent) = Handshake::client(client_code);
        let (agent, to_client) = Handshake::agent(agent_code);
        (
            client.finish(&to_client).unwrap(),
            agent.finish(&to_agent).unwrap(),
        )
    }

    #[test]
    fn the_same_code_on_both_sides_proves_each_side_to_the_other() {
        let (client, agent) = keys("K7QW9ZR2", "K7QW9ZR2");

        let joining = client.prove(Side::Client, "laptop details");
        assert!(agent.check(Side::Client, "laptop details", &joining));
        let paired = agent.prove(Side::Agent, "agent details");
        assert!(client.check(Side::Agent, "agent details", &paired));
    }

    #[test]
    fn a_wrong_code_fails_on_both_sides() {
        let (client, agent) = keys("K7QW9ZR3", "K7QW9ZR2");

        assert!(!agent.check(
            Side::Client,
            "details",
            &client.prove(Side::Client, "details")
        ));
        assert!(!client.check(Side::Agent, "details", &agent.prove(Side::Agent, "details")));
    }

    #[test]
    fn a_proof_does_not_survive_changed_details() {
        let (client, agent) = keys("K7QW9ZR2", "K7QW9ZR2");
        let proof = client.prove(Side::Client, r#"{"client":"laptop"}"#);

        assert!(!agent.check(Side::Client, r#"{"client":"attacker"}"#, &proof));
    }

    #[test]
    fn a_proof_cannot_be_sent_back_as_the_other_side() {
        let (client, _) = keys("K7QW9ZR2", "K7QW9ZR2");
        let proof = client.prove(Side::Client, "details");

        assert!(!client.check(Side::Agent, "details", &proof));
    }

    #[test]
    fn a_malformed_handshake_message_is_an_error() {
        let (client, _) = Handshake::client("K7QW9ZR2");

        assert!(client.finish(b"short").is_err());
    }

    #[test]
    fn an_old_pairing_request_is_still_recognized() {
        let old = r#"{"kind":"pair","token":"K7QW9ZR2","client":"laptop","public_key":"ssh-ed25519 AAAA","user":"me","host_keys":[]}"#;

        assert!(matches!(
            serde_json::from_str::<Request>(old).unwrap(),
            Request::Pair {}
        ));
    }
}
