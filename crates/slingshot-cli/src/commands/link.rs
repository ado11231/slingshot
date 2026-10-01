//! `slingshot link <code>`: pair with an Agent and remember it.

use crate::client;
use crate::keys;
use crate::project;
use slingshot_core::config::{Agent, Config};
use slingshot_core::keys as core_keys;
use slingshot_core::preflight::{self, Check, State};
use slingshot_core::presentation::{self, Style, Tone, home_path};
use slingshot_core::protocol::Joining;
use slingshot_core::step;
use slingshot_core::tunnel;

/// Take a pairing code, install this machine's key on the Agent, and save what it
/// takes to reach it again. Every step says what it did.
///
/// One `ssh_port` feeds both the learned host keys and the saved Agent, because `unlink`
/// forgets those keys by the saved port. Learning them under a different one would leave
/// an entry nothing could ever remove.
pub async fn link(code: String, name: Option<String>) -> anyhow::Result<i32> {
    let (host, port, token) = parse_code(&code)?;
    let identity = tunnel::identity(&project::client_root()?)?;
    let mut config = Config::load_or_empty()?;
    let client_name = config.client_for(&host, &identity.public().to_string());

    let problems: Vec<Check> = [preflight::tool_check("rsync", Some("copying projects"))]
        .into_iter()
        .filter(|check| check.state != State::Pass)
        .collect();
    preflight::report(&problems);

    let reaching = step::start(format!("Reaching {host}"));
    if !preflight::is_listening(format!("{host}:{port}").parse()?) {
        reaching.clear();
        preflight::report(&[unreachable(&host, port)]);
        anyhow::bail!("Pairing stopped");
    }
    reaching.set(format!("Pairing with {host}"));

    let (private_key, public_key) = keys::ensure(&client_name)?;

    let paired = client::pair(
        &host,
        port,
        &token,
        &Joining {
            client: client_name.clone(),
            public_key,
            iroh: Some(identity.public().to_string()),
        },
    )
    .await?;

    let name = name.unwrap_or(paired.name.clone());
    let ssh_port = None;
    let known_hosts = core_keys::learn_host(&host, ssh_port, &paired.host_keys)?;

    config.upsert(Agent {
        name: name.clone(),
        host: host.clone(),
        user: paired.user.clone(),
        port: ssh_port,
        daemon_port: Some(port),
        identity_file: Some(private_key.clone()),
        known_hosts: Some(known_hosts.clone()),
        program: paired.program.clone(),
        addresses: paired.addresses.clone(),
        iroh: paired.iroh.clone(),
        client: Some(client_name),
        specs: Some(paired.specs),
    });

    let saved = config.save()?;

    reaching.done(format!("Paired with {name}"));
    presentation::detail("Key", home_path(&private_key));
    presentation::detail(
        "Installed",
        format!("{}@{}:~/.ssh/authorized_keys", paired.user, host),
    );
    presentation::detail(
        "Host keys",
        format!(
            "{} ({} learned)",
            home_path(&known_hosts),
            paired.host_keys.len()
        ),
    );
    if !paired.addresses.is_empty() {
        presentation::detail("Addresses", paired.addresses.join(", "));
    }
    presentation::detail("Saved", home_path(&saved));
    eprintln!();

    if let Err(error) = super::tools::offer(config.resolve(Some(&name))?).await {
        presentation::warning(format!(
            "Could not set up tools on {name}: {error:#}. Run slingshot tools to try again"
        ));
    }
    if let Err(error) = super::menubar::offer(&name).await {
        presentation::warning(format!(
            "Could not add {name} to the menu bar: {error:#}. Run slingshot menubar to try again"
        ));
    }
    eprintln!();
    eprintln!(
        "  Try it: {}",
        Style::stderr().paint("slingshot run uname -n", Tone::Info)
    );
    eprintln!();

    Ok(0)
}

/// Nothing answered the pairing port. The usual cause is being on another network,
/// because pairing only works where the Agent can be reached directly.
fn unreachable(host: &str, port: u16) -> Check {
    Check::fail(
        format!("Could not reach {host}:{port}"),
        "Check that slingshot start is running there, and that this machine is on the same network or tailnet",
    )
}

/// Split a pairing code into the Agent's address and the one time token. Codes are
/// readable on purpose, so you can see which machine you are about to trust.
fn parse_code(code: &str) -> anyhow::Result<(String, u16, String)> {
    let parts: Vec<&str> = code.trim().split(':').collect();

    let [host, port, token] = parts.as_slice() else {
        anyhow::bail!("That does not look like a pairing code. Expected host:port:code");
    };

    let port: u16 = port
        .parse()
        .map_err(|_| anyhow::anyhow!("'{port}' is not a port number"))?;

    Ok((host.to_string(), port, token.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_good_code_splits_into_three_parts() {
        let (host, port, token) = parse_code("192.168.1.9:7433:K7QW9ZR2").unwrap();

        assert_eq!(host, "192.168.1.9");
        assert_eq!(port, 7433);
        assert_eq!(token, "K7QW9ZR2");
    }

    #[test]
    fn surrounding_whitespace_is_forgiven() {
        assert_eq!(parse_code("  10.0.0.4:7433:ABCD2345\n").unwrap().1, 7433);
    }

    #[test]
    fn a_code_missing_a_part_is_rejected() {
        assert!(parse_code("10.0.0.4:7433").is_err());
    }

    #[test]
    fn a_port_that_is_not_a_number_is_reported() {
        let err = parse_code("10.0.0.4:door:ABCD2345")
            .unwrap_err()
            .to_string();

        assert!(err.contains("door"), "message was: {err}");
    }
}
