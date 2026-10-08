//! Slingshot uses a dedicated SSH key so unlink can revoke its access.

use anyhow::Context;
use slingshot_core::keys::{marker, ssh_dir};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

/// The private key file. The public one is the same path with .pub on the end.
const KEY_NAME: &str = "slingshot_ed25519";

fn private_key_path() -> anyhow::Result<PathBuf> {
    Ok(ssh_dir()?.join(KEY_NAME))
}

/// Slingshot's private and public key files, whether or not they exist.
pub fn files() -> anyhow::Result<[PathBuf; 2]> {
    let private = private_key_path()?;
    let public = private.with_extension("pub");
    Ok([private, public])
}

/// Find slingshot's key, creating it the first time. Also says whether it was just made, so
/// `link` can announce a new key with its path, because a tool that quietly makes keys cannot
/// be audited.
pub fn ensure(client_name: &str) -> anyhow::Result<(PathBuf, String, bool)> {
    let private = private_key_path()?;
    let public = private.with_extension("pub");

    let created = !public.exists();
    if created {
        let dir = ssh_dir()?;
        fs::create_dir_all(&dir).with_context(|| format!("Could not create {}", dir.display()))?;

        let status = Command::new("ssh-keygen")
            .args([
                "-t",
                "ed25519",
                "-N",
                "",
                "-q",
                "-C",
                &marker(client_name),
                "-f",
            ])
            .arg(&private)
            .status()
            .context("Could not run ssh-keygen. Install OpenSSH, then run slingshot link again")?;

        if !status.success() {
            anyhow::bail!(
                "Could not create the SSH key {}. Check that the folder is writable, then run slingshot link again",
                private.display()
            );
        }
    }

    let text = fs::read_to_string(&public)
        .with_context(|| format!("Could not read {}", public.display()))?;

    Ok((private, text.trim().to_string(), created))
}
