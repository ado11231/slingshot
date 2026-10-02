//! What both machines share for reaching an Agent over iroh: the protocol name each end
//! checks, and a persistent identity. The public half of an identity is how the other
//! machine addresses and recognizes this one.

use crate::storage;
use anyhow::Context;
use iroh::{PublicKey, SecretKey};
use std::path::Path;

/// Names what flows over a Slingshot iroh connection: a raw stream to the Agent's sshd.
pub const ALPN: &[u8] = b"slingshot/ssh/1";

/// Why the Agent closes a connection from a key that never paired. The Client looks for it
/// to tell a refusal apart from an Agent that is simply off.
pub const NOT_PAIRED: &str = "not paired with this Agent";

const IDENTITY_FILE: &str = "iroh.key";

/// Load the identity kept in `dir`, creating it on first use. The file is readable only
/// by this account, because whoever holds it can pose as this machine.
pub fn identity(dir: &Path) -> anyhow::Result<SecretKey> {
    let path = dir.join(IDENTITY_FILE);
    match std::fs::read_to_string(&path) {
        Ok(text) => text
            .trim()
            .parse()
            .with_context(|| format!("{} is not a valid iroh key", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let key = SecretKey::generate();
            storage::private_dir(dir)?;
            storage::write_bytes(&path, hex(&key.to_bytes()).as_bytes())?;
            Ok(key)
        }
        Err(e) => Err(e).with_context(|| format!("Could not read {}", path.display())),
    }
}

/// Parse a public key received from the other machine.
pub fn public_key(text: &str) -> anyhow::Result<PublicKey> {
    text.parse()
        .map_err(|_| anyhow::anyhow!("'{text}' is not an iroh public key"))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::testing::TempDir;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn an_identity_is_created_once_and_then_reused() {
        let temp = TempDir::new("tunnel");
        let first = identity(temp.path()).unwrap();
        let second = identity(temp.path()).unwrap();

        assert_eq!(first.public(), second.public());
    }

    #[test]
    fn the_identity_file_is_private() {
        let temp = TempDir::new("tunnel");
        identity(temp.path()).unwrap();
        let mode = std::fs::metadata(temp.path().join(IDENTITY_FILE))
            .unwrap()
            .permissions()
            .mode();

        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn a_public_key_survives_the_trip_as_text() {
        let key = SecretKey::generate().public();

        assert_eq!(public_key(&key.to_string()).unwrap(), key);
    }

    #[test]
    fn garbage_is_not_a_public_key() {
        assert!(public_key("not a key").is_err());
    }
}
