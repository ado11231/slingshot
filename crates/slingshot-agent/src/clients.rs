//! Which Clients may reach this Agent over iroh. Pairing adds a Client's key and unlink
//! removes it. ssh still decides who logs in; this only decides who reaches sshd at all.

use iroh::PublicKey;
use serde::{Deserialize, Serialize};
use slingshot_core::storage;
use slingshot_core::tunnel;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Client name to iroh public key, one entry per paired Client.
#[derive(Default, Serialize, Deserialize)]
struct Clients {
    keys: BTreeMap<String, String>,
}

fn path(root: &Path) -> PathBuf {
    root.join("clients.json")
}

pub fn allow(root: &Path, client: &str, key: &str) -> anyhow::Result<()> {
    tunnel::public_key(key)?;
    change(root, |clients| {
        clients.keys.insert(client.to_string(), key.to_string());
    })
}

pub fn forget(root: &Path, client: &str) -> anyhow::Result<()> {
    change(root, |clients| {
        clients.keys.remove(client);
    })
}

/// Whether `key` belongs to a paired Client. Read fresh each time, so an unlink takes
/// effect for the next connection without restarting the daemon.
pub fn allowed(root: &Path, key: &PublicKey) -> anyhow::Result<bool> {
    let clients: Clients = storage::read_json(&path(root))?;
    Ok(clients
        .keys
        .values()
        .any(|text| tunnel::public_key(text).is_ok_and(|known| known == *key)))
}

/// The names of the paired Clients, for the start screen. A Client from before iroh is
/// linked through ssh alone, so it is not listed.
pub fn names(root: &Path) -> anyhow::Result<Vec<String>> {
    let clients: Clients = storage::read_json(&path(root))?;
    Ok(clients.keys.into_keys().collect())
}

fn change(root: &Path, update: impl FnOnce(&mut Clients)) -> anyhow::Result<()> {
    let _lock = storage::lock(&root.join("clients.lock"))?;
    let mut clients: Clients = storage::read_json(&path(root))?;
    update(&mut clients);
    storage::write_json(&path(root), &clients)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::Root;
    use iroh::SecretKey;

    #[test]
    fn a_paired_client_is_allowed_and_a_stranger_is_not() {
        let root = Root::new();
        let laptop = SecretKey::generate().public();
        let stranger = SecretKey::generate().public();
        allow(&root.0, "laptop", &laptop.to_string()).unwrap();

        assert!(allowed(&root.0, &laptop).unwrap());
        assert!(!allowed(&root.0, &stranger).unwrap());
    }

    #[test]
    fn forgetting_a_client_removes_only_that_client() {
        let root = Root::new();
        let laptop = SecretKey::generate().public();
        let desktop = SecretKey::generate().public();
        allow(&root.0, "laptop", &laptop.to_string()).unwrap();
        allow(&root.0, "desktop", &desktop.to_string()).unwrap();
        forget(&root.0, "laptop").unwrap();

        assert!(!allowed(&root.0, &laptop).unwrap());
        assert!(allowed(&root.0, &desktop).unwrap());
    }

    #[test]
    fn pairing_again_replaces_the_old_key() {
        let root = Root::new();
        let old = SecretKey::generate().public();
        let new = SecretKey::generate().public();
        allow(&root.0, "laptop", &old.to_string()).unwrap();
        allow(&root.0, "laptop", &new.to_string()).unwrap();

        assert!(!allowed(&root.0, &old).unwrap());
        assert!(allowed(&root.0, &new).unwrap());
    }

    #[test]
    fn a_malformed_key_is_refused() {
        let root = Root::new();

        assert!(allow(&root.0, "laptop", "not a key").is_err());
    }

    #[test]
    fn names_lists_each_paired_client_once() {
        let root = Root::new();
        assert!(names(&root.0).unwrap().is_empty());

        let laptop = SecretKey::generate().public();
        allow(&root.0, "laptop", &laptop.to_string()).unwrap();
        allow(
            &root.0,
            "desktop",
            &SecretKey::generate().public().to_string(),
        )
        .unwrap();
        allow(&root.0, "laptop", &laptop.to_string()).unwrap();

        assert_eq!(names(&root.0).unwrap(), ["desktop", "laptop"]);
    }

    #[test]
    fn nobody_is_allowed_before_anyone_pairs() {
        let root = Root::new();

        assert!(!allowed(&root.0, &SecretKey::generate().public()).unwrap());
    }
}
