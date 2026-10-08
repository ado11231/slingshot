//! Where the Client remembers which Agent to talk to.

use crate::keys;
use crate::network::Network;
use crate::protocol::{DEFAULT_PORT, Specs};
use anyhow::Context;
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// Shown whenever there is no Agent to talk to. Names the two commands that fix it,
/// because a stranger has no config file to look at yet.
const NO_AGENTS: &str = "No Agent configured yet\n\n\
                         On the Agent:   slingshot start\n\
                         On the Client:  slingshot link <code>";

/// The whole config file. `agents` is a list so it renders as readable `[[agents]]`
/// blocks for anyone who opens the file and edits it by hand.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    default: Option<String>,
    #[serde(default)]
    agents: Vec<Agent>,
}

/// One Agent. `name` is the nickname the user types; `host` is what ssh dials.
/// `port` of `None` means 22, and `identity_file` of `None` falls back to the
/// user's own ssh setup rather than the key that pairing installed.
///
/// `specs` sits last because toml cannot put a plain value after a table.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Agent {
    pub name: String,
    pub host: String,
    pub user: String,
    pub port: Option<u16>,
    /// Where the daemon listens. Separate from `port`, which is for ssh.
    pub daemon_port: Option<u16>,
    pub identity_file: Option<PathBuf>,
    /// The file holding this Agent's ssh host keys, learned at pairing.
    pub known_hosts: Option<PathBuf>,
    /// Where the Slingshot program lives on the Agent, reported at pairing.
    #[serde(default)]
    pub program: Option<String>,
    /// Other addresses the Agent reported at pairing. Configs saved before this have none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub addresses: Vec<String>,
    /// The Agent's iroh public key. Configs saved before iroh have none.
    #[serde(default)]
    pub iroh: Option<String>,
    /// The name this Agent knows this machine by. Links saved before unique names have none
    /// and go by the hostname.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client: Option<String>,
    /// What the Agent is, fetched once at pairing so `health` names it without asking again.
    pub specs: Option<Specs>,
}

impl Agent {
    /// The daemon port to dial, falling back to the built in default.
    pub fn daemon_port(&self) -> u16 {
        self.daemon_port.unwrap_or(DEFAULT_PORT)
    }

    /// The name this Agent knows this machine by. `link` labels the installed key with it and
    /// `unlink` removes the key by it, so both must get the same answer.
    pub fn client_name(&self) -> String {
        self.client.clone().unwrap_or_else(keys::client_name)
    }

    /// The Slingshot program to start over SSH. Pairing records the Agent's own path, so a
    /// login shell without Slingshot on its PATH still works.
    pub fn program(&self) -> &str {
        self.program.as_deref().unwrap_or("slingshot")
    }

    /// Every address to try, best first: this machine, the local network, anything else,
    /// then the tailnet. Ties keep the pairing address ahead of the rest.
    pub fn candidates(&self) -> Vec<&str> {
        let mut all: Vec<&str> = Vec::new();
        for host in std::iter::once(&self.host).chain(&self.addresses) {
            if !all.contains(&host.as_str()) {
                all.push(host);
            }
        }
        all.sort_by_key(|host| Network::of(host));
        all
    }
}

/// The platform's config directory for slingshot. The only place that knows this
/// path, so no OS specific path appears anywhere else.
pub fn dir() -> anyhow::Result<PathBuf> {
    let Some(dirs) = ProjectDirs::from("", "", "slingshot") else {
        anyhow::bail!("Could not determine home directory");
    };
    Ok(dirs.config_dir().to_path_buf())
}

fn path() -> anyhow::Result<PathBuf> {
    Ok(dir()?.join("config.toml"))
}

/// Where slingshot keeps the host keys of the Agents it has paired with. Kept apart
/// from your own `~/.ssh/known_hosts` so slingshot only ever edits its own files.
pub fn known_hosts_path() -> anyhow::Result<PathBuf> {
    Ok(dir()?.join("known_hosts"))
}

impl Config {
    /// Read and parse the config file. A missing file is the first run case, not a
    /// filesystem error, so it reports the pairing commands instead.
    pub fn load() -> anyhow::Result<Config> {
        let file = path()?;

        if !file.exists() {
            anyhow::bail!(NO_AGENTS);
        }

        let content = fs::read_to_string(&file)
            .with_context(|| format!("Could not read {}", file.display()))?;

        let config: Config = toml::from_str(&content)
            .with_context(|| format!("Could not parse {}", file.display()))?;

        Ok(config)
    }

    /// Look up one agent by nickname. A miss lists the names that do exist, so a
    /// typo is a fix that takes a second.
    fn find(&self, name: &str) -> anyhow::Result<&Agent> {
        let names: Vec<&str> = self.agents.iter().map(|a| a.name.as_str()).collect();

        self.agents.iter().find(|a| a.name == name).ok_or_else(|| {
            anyhow::anyhow!(
                "No Agent named '{name}'. Configured Agents: {}",
                names.join(", ")
            )
        })
    }

    /// Pick the agent to use. An explicit `--agent` wins, then `default`, then the
    /// sole agent if there is only one.
    pub fn resolve(&self, requested: Option<&str>) -> anyhow::Result<&Agent> {
        match (requested, self.default.as_deref()) {
            (Some(name), _) => self.find(name),
            (None, Some(default)) => self.find(default),
            (None, None) => match self.agents.as_slice() {
                [] => anyhow::bail!(NO_AGENTS),
                [only] => Ok(only),
                _ => {
                    let names: Vec<&str> = self.agents.iter().map(|a| a.name.as_str()).collect();
                    anyhow::bail!(
                        "Several Agents configured: {}\n\n\
                         Pass --agent <name>, or set default = \"<name>\" in {}",
                        names.join(", "),
                        path()?.display()
                    )
                }
            },
        }
    }

    /// Read the config, treating a missing file as an empty one. Used by the
    /// commands that are about to write, where nothing saved yet is normal.
    pub fn load_or_empty() -> anyhow::Result<Config> {
        match path()?.exists() {
            true => Config::load(),
            false => Ok(Config {
                default: None,
                agents: Vec::new(),
            }),
        }
    }

    /// The name this machine links under with the Agent at `host`. An Agent already saved
    /// keeps the name it knows, so linking again replaces the old key instead of leaving it
    /// behind. A new Agent gets a name unique to this machine, made from `id`.
    pub fn client_for(&self, host: &str, id: &str) -> String {
        self.agents
            .iter()
            .find(|agent| agent.host == host || agent.addresses.iter().any(|a| a == host))
            .map(Agent::client_name)
            .unwrap_or_else(|| keys::unique_client_name(&keys::client_name(), id))
    }

    /// Add an Agent, or replace the entry of the same name when pairing again. The
    /// first Agent paired becomes the default, so `run` works with no flags.
    pub fn upsert(&mut self, agent: Agent) {
        self.agents.retain(|a| a.name != agent.name);

        if self.default.is_none() && self.agents.is_empty() {
            self.default = Some(agent.name.clone());
        }

        self.agents.push(agent);
    }

    /// Forget an Agent. Clears the default too when it pointed at that Agent, so the
    /// config never names an agent that is not there.
    pub fn remove(&mut self, name: &str) -> anyhow::Result<Agent> {
        let Some(index) = self.agents.iter().position(|a| a.name == name) else {
            anyhow::bail!("No Agent named '{name}'");
        };

        if self.default.as_deref() == Some(name) {
            self.default = None;
        }

        Ok(self.agents.remove(index))
    }

    /// Write the config back out, creating the directory the first time.
    /// The names of every linked Agent.
    pub fn names(&self) -> Vec<String> {
        self.agents.iter().map(|agent| agent.name.clone()).collect()
    }

    pub fn save(&self) -> anyhow::Result<PathBuf> {
        let file = path()?;

        if let Some(parent) = file.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Could not create {}", parent.display()))?;
        }

        let body = toml::to_string_pretty(self).context("Could not encode the config")?;
        fs::write(&file, body).with_context(|| format!("Could not write {}", file.display()))?;

        Ok(file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TWO: &str = r#"
        default = "laptop"

        [[agents]]
        name = "archbox"
        host = "archbox.local"
        user = "me"

        [[agents]]
        name = "laptop"
        host = "laptop.local"
        user = "me"
    "#;

    const ONE: &str = r#"
        [[agents]]
        name = "archbox"
        host = "archbox.local"
        user = "me"
    "#;

    fn config(toml: &str) -> Config {
        toml::from_str(toml).expect("Test fixture should parse")
    }

    #[test]
    fn a_tailnet_pairing_still_tries_the_local_network_first() {
        let mut agent = config(ONE).resolve(None).unwrap().clone();
        agent.host = "100.67.90.119".to_string();
        agent.addresses = vec!["192.168.1.20".to_string(), "100.67.90.119".to_string()];

        assert_eq!(agent.candidates(), ["192.168.1.20", "100.67.90.119"]);
    }

    #[test]
    fn a_config_from_before_addresses_has_one_candidate() {
        assert_eq!(
            config(ONE).resolve(None).unwrap().candidates(),
            ["archbox.local"]
        );
    }

    #[test]
    fn default_is_used_when_no_agent_requested() {
        assert_eq!(config(TWO).resolve(None).unwrap().name, "laptop");
    }

    #[test]
    fn explicit_request_beats_the_default() {
        assert_eq!(
            config(TWO).resolve(Some("archbox")).unwrap().name,
            "archbox"
        );
    }

    #[test]
    fn sole_agent_is_used_without_a_default() {
        assert_eq!(config(ONE).resolve(None).unwrap().name, "archbox");
    }

    #[test]
    fn unknown_agent_lists_what_exists() {
        let err = config(TWO).resolve(Some("nope")).unwrap_err().to_string();

        assert!(err.contains("nope"), "message was: {err}");
        assert!(err.contains("archbox"), "message was: {err}");
        assert!(err.contains("laptop"), "message was: {err}");
    }

    #[test]
    fn default_naming_a_missing_agent_is_reported() {
        let err = config(
            r#"
            default = "ghost"

            [[agents]]
            name = "archbox"
            host = "archbox.local"
            user = "me"
            "#,
        )
        .resolve(None)
        .unwrap_err()
        .to_string();

        assert!(err.contains("ghost"), "message was: {err}");
    }

    #[test]
    fn several_agents_without_a_default_is_ambiguous() {
        let err = config(
            r#"
            [[agents]]
            name = "archbox"
            host = "archbox.local"
            user = "me"

            [[agents]]
            name = "laptop"
            host = "laptop.local"
            user = "me"
            "#,
        )
        .resolve(None)
        .unwrap_err()
        .to_string();

        assert!(err.contains("--agent"), "message was: {err}");
        assert!(err.contains("archbox"), "message was: {err}");
    }

    #[test]
    fn no_agents_points_at_pairing() {
        let err = config("").resolve(None).unwrap_err().to_string();

        assert!(err.contains("slingshot link"), "message was: {err}");
    }

    fn agent(name: &str) -> Agent {
        Agent {
            name: name.to_string(),
            host: format!("{name}.local"),
            user: "me".to_string(),
            port: None,
            daemon_port: None,
            identity_file: None,
            known_hosts: None,
            program: None,
            addresses: Vec::new(),
            iroh: None,
            client: None,
            specs: None,
        }
    }

    #[test]
    fn a_new_agent_gets_a_name_unique_to_this_machine() {
        let name = config("").client_for("192.168.1.9", "3f9c2ab71e");

        assert!(name.ends_with("-3f9c2a"), "name was: {name}");
    }

    #[test]
    fn linking_a_saved_agent_again_keeps_its_name() {
        let mut config = config("");
        let mut saved = agent("archbox");
        saved.client = Some("laptop-8d04e6".to_string());
        saved.addresses = vec!["100.64.0.2".to_string()];
        config.upsert(saved);

        assert_eq!(
            config.client_for("archbox.local", "3f9c2a"),
            "laptop-8d04e6"
        );
        assert_eq!(config.client_for("100.64.0.2", "3f9c2a"), "laptop-8d04e6");
    }

    #[test]
    fn an_agent_linked_before_unique_names_keeps_the_hostname() {
        let mut config = config("");
        config.upsert(agent("archbox"));

        assert_eq!(
            config.client_for("archbox.local", "3f9c2a"),
            keys::client_name()
        );
    }

    #[test]
    fn the_first_agent_paired_becomes_the_default() {
        let mut config = config("");
        config.upsert(agent("archbox"));

        assert_eq!(config.default.as_deref(), Some("archbox"));
        assert_eq!(config.resolve(None).unwrap().name, "archbox");
    }

    #[test]
    fn pairing_again_replaces_rather_than_duplicates() {
        let mut config = config("");
        config.upsert(agent("archbox"));

        let mut moved = agent("archbox");
        moved.host = "10.0.0.9".to_string();
        config.upsert(moved);

        assert_eq!(config.agents.len(), 1);
        assert_eq!(config.agents[0].host, "10.0.0.9");
    }

    #[test]
    fn removing_the_default_agent_clears_the_default() {
        let mut config = config(TWO);
        config.remove("laptop").unwrap();

        assert_eq!(config.default, None);
        assert!(config.remove("laptop").is_err());
    }

    #[test]
    fn specs_saved_with_the_old_tools_list_still_load() {
        let saved = format!(
            "{ONE}\n[agents.specs]\nname = \"archbox\"\nos = \"Arch Linux\"\nkernel = \"6.9.1\"\ncpu = \"i7\"\ncores = 16\nmemory_mib = 32000\ndisk_total_mib = 900000\ntools = [\"docker\"]\ngpus = []\n"
        );
        let loaded = config(&saved);
        assert_eq!(loaded.agents[0].specs.as_ref().unwrap().cores, 16);
    }

    #[test]
    fn an_agent_with_cached_specs_survives_a_toml_round_trip() {
        let mut with_specs = agent("archbox");
        with_specs.daemon_port = Some(7433);
        with_specs.specs = Some(Specs {
            name: "archbox".to_string(),
            os: "Arch Linux".to_string(),
            kernel: "6.9.1".to_string(),
            cpu: "Ryzen 5".to_string(),
            cores: 12,
            memory_mib: 32000,
            disk_total_mib: 900000,
            gpus: vec![crate::protocol::Gpu {
                name: "RTX 3070".to_string(),
                vram_mib: Some(8192),
            }],
        });

        let mut config = config("");
        config.upsert(with_specs);

        let text = toml::to_string_pretty(&config).expect("config should encode");
        let back: Config = toml::from_str(&text).expect("config should decode");

        assert_eq!(
            back.agents[0].specs.as_ref().unwrap().gpus[0].name,
            "RTX 3070"
        );
        assert_eq!(back.agents[0].daemon_port(), 7433);
    }

    #[test]
    fn unknown_field_is_rejected() {
        let result = toml::from_str::<Config>(
            r#"
            [[agents]]
            name = "archbox"
            hostname = "archbox.local"
            user = "me"
            "#,
        );

        assert!(result.is_err());
    }

    #[test]
    fn an_agent_with_no_recorded_program_falls_back_to_the_path() {
        assert_eq!(agent("archbox").program(), "slingshot");
    }
}
