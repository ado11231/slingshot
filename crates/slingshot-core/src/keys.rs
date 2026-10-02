//! How slingshot labels the key it installs, and how a machine publishes its identity.

use crate::{config, storage};
use anyhow::Context;
use std::fs;
use std::path::{Path, PathBuf};

/// The comment written into slingshot's public key, and the marker `unlink` looks for
/// when taking that key back off an Agent. Both sides have to agree on it, which is
/// why it lives here rather than on either side.
pub fn marker(client_name: &str) -> String {
    format!("slingshot:{client_name}")
}

/// This machine's hostname, which Clients linked before unique names still go by.
pub fn client_name() -> String {
    sysinfo::System::host_name().unwrap_or_else(|| "client".to_string())
}

/// A name no other Client shares, even one with the same hostname: the hostname plus the
/// start of `id`, a key that belongs to this machine alone.
pub fn unique_client_name(hostname: &str, id: &str) -> String {
    let short: String = id.chars().take(6).collect();
    format!("{hostname}-{short}")
}

/// Where sshd publishes the public half of a machine's host keys.
const HOST_KEY_DIR: &str = "/etc/ssh";

/// Read public host keys for the pairing exchange.
pub fn host_keys() -> Vec<String> {
    let Ok(entries) = fs::read_dir(HOST_KEY_DIR) else {
        return Vec::new();
    };

    let mut keys: Vec<String> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                return false;
            };
            name.starts_with("ssh_host_") && name.ends_with("_key.pub")
        })
        .filter_map(|path| fs::read_to_string(path).ok())
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
        .collect();

    keys.sort();
    keys
}

/// How a host is written in a known_hosts file. Anything on a port other than the
/// usual one is wrapped in brackets, which is the format ssh expects.
fn host_pattern(host: &str, port: Option<u16>) -> String {
    match port {
        Some(port) if port != 22 => format!("[{host}]:{port}"),
        _ => host.to_string(),
    }
}

/// Record an Agent's ssh host keys so the first connection needs no fingerprint check.
/// Older entries are replaced, so pairing again after a rebuild still works.
pub fn learn_host(host: &str, port: Option<u16>, host_keys: &[String]) -> anyhow::Result<PathBuf> {
    let file = config::known_hosts_path()?;
    let pattern = host_pattern(host, port);

    let mut lines = without_host(&file, &pattern)?;

    for key in host_keys {
        let key = key.trim();

        match key.split_whitespace().collect::<Vec<&str>>().as_slice() {
            [kind, material, ..] => lines.push(format!("{pattern} {kind} {material}")),
            _ => continue,
        }
    }

    write_lines(&file, &lines)?;

    Ok(file)
}

/// Forget an Agent's host keys, so nothing is left pointing at a machine you unlinked.
pub fn forget_host(host: &str, port: Option<u16>) -> anyhow::Result<()> {
    let file = config::known_hosts_path()?;

    if !file.exists() {
        return Ok(());
    }

    let lines = without_host(&file, &host_pattern(host, port))?;
    write_lines(&file, &lines)
}

/// Every line of the file except the ones for this Agent.
fn without_host(file: &Path, pattern: &str) -> anyhow::Result<Vec<String>> {
    let existing = fs::read_to_string(file).unwrap_or_default();

    Ok(existing
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter(|line| line.split_whitespace().next() != Some(pattern))
        .map(|line| line.to_string())
        .collect())
}

/// Replace an SSH line file in one step. Truncating in place would leave the file empty
/// if the write failed partway, which for `authorized_keys` locks the owner out of the Agent.
fn write_lines(file: &Path, lines: &[String]) -> anyhow::Result<()> {
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Could not create {}", parent.display()))?;
    }

    let mut body = lines.join("\n");
    body.push('\n');

    storage::write_bytes(file, body.as_bytes())
}

/// Authorize a public key using the same marker that unlink uses to revoke it.
/// Rewrite complete lines so a missing trailing newline cannot merge keys.
/// Both machines use this to establish trust without sharing private keys.
pub fn authorize(peer: &str, public_key: &str) -> anyhow::Result<PathBuf> {
    let tag = marker(peer);
    let key = authorized_line(peer, public_key)?;

    let dir = ssh_dir()?;
    fs::create_dir_all(&dir).with_context(|| format!("Could not create {}", dir.display()))?;
    set_mode(&dir, 0o700)?;

    let file = dir.join("authorized_keys");
    let existing = fs::read_to_string(&file).unwrap_or_default();

    let mut lines: Vec<String> = existing
        .lines()
        .filter(|line| !line.trim().is_empty() && !line.ends_with(&tag))
        .map(|line| line.to_string())
        .collect();

    lines.push(key);
    write_lines(&file, &lines)?;

    Ok(file)
}

/// Set owner permissions for SSH files.
fn set_mode(path: &std::path::Path, mode: u32) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode))
            .with_context(|| format!("Could not set permissions on {}", path.display()))?;
    }

    let _ = (path, mode);
    Ok(())
}

/// One authorized_keys line: the key itself, relabelled with our own marker.
fn authorized_line(peer: &str, public_key: &str) -> anyhow::Result<String> {
    match public_key
        .split_whitespace()
        .collect::<Vec<&str>>()
        .as_slice()
    {
        [kind, material, ..] if kind.starts_with("ssh-") || kind.starts_with("ecdsa-") => {
            Ok(format!("{kind} {material} {}", marker(peer)))
        }
        _ => anyhow::bail!("That does not look like an SSH public key"),
    }
}

pub fn ssh_dir() -> anyhow::Result<PathBuf> {
    let Some(base) = directories::BaseDirs::new() else {
        anyhow::bail!("Could not determine home directory");
    };
    Ok(base.home_dir().join(".ssh"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_usual_port_is_written_plainly() {
        assert_eq!(host_pattern("10.0.0.9", None), "10.0.0.9");
        assert_eq!(host_pattern("10.0.0.9", Some(22)), "10.0.0.9");
    }

    #[test]
    fn an_unusual_port_gets_brackets() {
        assert_eq!(host_pattern("10.0.0.9", Some(2222)), "[10.0.0.9]:2222");
    }

    #[test]
    fn the_marker_names_the_client() {
        assert_eq!(marker("laptop"), "slingshot:laptop");
    }

    #[test]
    fn two_clients_with_one_hostname_get_different_names() {
        let first = unique_client_name("MacBook-Pro", "3f9c2ab71e");
        let second = unique_client_name("MacBook-Pro", "8d04e6c9b2");

        assert_eq!(first, "MacBook-Pro-3f9c2a");
        assert_ne!(first, second);
    }

    /// The bug this guards against: a key arrives already carrying somebody else's
    /// marker, and is revoked by this peer's name. Stored verbatim, it installs fine
    /// and then cannot ever be removed.
    #[test]
    fn an_authorized_line_ends_with_the_marker_used_to_revoke_it() {
        let key = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5 slingshot:mount";

        let line = authorized_line("archbox", key).unwrap();

        assert!(line.ends_with(&marker("archbox")), "line was: {line}");
        assert!(
            line.starts_with("ssh-ed25519 AAAAC3NzaC1lZDI1NTE5"),
            "line was: {line}"
        );
    }

    #[test]
    fn a_key_with_no_comment_still_gets_one() {
        let line = authorized_line("archbox", "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5").unwrap();

        assert_eq!(line, "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5 slingshot:archbox");
    }

    #[test]
    fn something_that_is_not_a_key_is_refused() {
        assert!(authorized_line("archbox", "hello there").is_err());
    }

    /// An SSH line file is replaced by a rename, so a failed write can never leave the
    /// caller with an empty authorized_keys and no way back into the machine.
    #[test]
    fn a_line_file_is_replaced_in_one_step_and_stays_private() {
        use std::os::unix::fs::PermissionsExt;

        let dir = crate::storage::testing::TempDir::new("keys");
        let file = dir.path().join("authorized_keys");

        write_lines(&file, &["ssh-ed25519 AAAA slingshot:laptop".to_string()]).unwrap();
        assert_eq!(
            fs::read_to_string(&file).unwrap(),
            "ssh-ed25519 AAAA slingshot:laptop\n"
        );
        assert_eq!(
            fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );

        write_lines(&file, &[]).unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), "\n");
        assert!(
            fs::read_dir(dir.path()).unwrap().flatten().all(|e| !e
                .file_name()
                .to_string_lossy()
                .starts_with(crate::storage::PARTIAL_PREFIX)),
            "a temporary file was left behind"
        );
    }

    /// `link` learns host keys and `unlink` forgets them. Both derive the known_hosts
    /// pattern from a port, so a mismatch leaves an entry nothing can ever remove.
    #[test]
    fn a_host_is_only_forgotten_under_the_port_it_was_learned_with() {
        let dir = crate::storage::testing::TempDir::new("hosts");
        let file = dir.path().join("known_hosts");
        let learned = host_pattern("archbox", Some(2222));
        write_lines(&file, &[format!("{learned} ssh-ed25519 AAAA")]).unwrap();

        let wrong_port = without_host(&file, &host_pattern("archbox", None)).unwrap();
        assert_eq!(wrong_port.len(), 1, "the entry should have survived");

        let right_port = without_host(&file, &learned).unwrap();
        assert!(right_port.is_empty(), "the entry should have been removed");
    }
}
