//! How to reach a box. Every saved address is probed at once and the most preferred one
//! that answers wins, so an unreachable home address costs one short timeout rather than
//! one per candidate. When none answers, iroh reaches the box from any network.

use slingshot_core::config::Agent;
use slingshot_core::network::Network;
use slingshot_core::tunnel;
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Mutex;
use std::time::Duration;

const PROBE_TIMEOUT: Duration = Duration::from_millis(800);
const SSH_PORT: u16 = 22;

/// Set by the Client for the rsync helper it starts, so that process skips probing.
pub const ROUTE_ENV: &str = "SLINGSHOT_RSH_HOST";

const IROH: &str = "iroh";

#[derive(Debug, Clone, PartialEq)]
pub enum Route {
    Direct { host: String, network: Network },
    Iroh { key: String },
}

impl Route {
    fn to(host: &str) -> Route {
        Route::Direct {
            host: host.to_string(),
            network: Network::of(host),
        }
    }

    /// How the path is named in output, as in "via tailnet".
    pub fn name(&self) -> &'static str {
        match self {
            Route::Direct { network, .. } => network.name(),
            Route::Iroh { .. } => IROH,
        }
    }

    /// What the rsync helper is told, so it takes the same path.
    pub fn token(&self) -> &str {
        match self {
            Route::Direct { host, .. } => host,
            Route::Iroh { .. } => IROH,
        }
    }
}

/// One process talks to one box, so the first answer is kept for every later ssh call.
static CHOSEN: Mutex<Option<(String, Route)>> = Mutex::new(None);

/// The path to take to `agent`. When nothing answers and the box has no iroh key, the
/// pairing address is returned so ssh reports the failure in its own words.
pub fn resolve(agent: &Agent) -> Route {
    let mut chosen = CHOSEN.lock().expect("route lock was poisoned");
    if let Some((name, route)) = chosen.as_ref()
        && *name == agent.name
    {
        return route.clone();
    }
    let route = probe(agent);
    *chosen = Some((agent.name.clone(), route.clone()));
    route
}

/// Probe again on the next `resolve`. A process that outlives one network, such as the
/// menu bar helper, calls this after losing the box so a new network gets a new path.
pub fn forget() {
    *CHOSEN.lock().expect("route lock was poisoned") = None;
}

/// Take the path a parent process already chose, from its `token`. Ignored unless it is
/// one of the box's own paths.
pub fn assume(agent: &Agent, token: &str) {
    let route = match (token, iroh_key(agent)) {
        (IROH, Some(key)) => Route::Iroh { key },
        (host, _) if agent.candidates().contains(&host) => Route::to(host),
        _ => return,
    };
    *CHOSEN.lock().expect("route lock was poisoned") = Some((agent.name.clone(), route));
}

/// Probing waits on the network for up to a second. Inside the async runtime the wait
/// is handed off, so other tasks on this worker keep running meanwhile.
fn probe(agent: &Agent) -> Route {
    use tokio::runtime::{Handle, RuntimeFlavor};

    match Handle::try_current() {
        Ok(handle) if handle.runtime_flavor() == RuntimeFlavor::MultiThread => {
            tokio::task::block_in_place(|| probe_now(agent))
        }
        _ => probe_now(agent),
    }
}

fn probe_now(agent: &Agent) -> Route {
    let candidates = agent.candidates();
    let key = iroh_key(agent);
    if let ([only], None) = (candidates.as_slice(), &key) {
        return Route::to(only);
    }
    let port = agent.port.unwrap_or(SSH_PORT);
    let attempts: Vec<_> = candidates
        .iter()
        .map(|host| {
            let host = host.to_string();
            std::thread::spawn(move || answers(&host, port))
        })
        .collect();
    for (host, attempt) in candidates.iter().zip(attempts) {
        if attempt.join().unwrap_or(false) {
            return Route::to(host);
        }
    }
    match key {
        Some(key) => Route::Iroh { key },
        None => Route::to(&agent.host),
    }
}

/// The box's iroh key, if pairing saved one that parses. Checked here so nothing
/// malformed ever reaches ssh's ProxyCommand.
fn iroh_key(agent: &Agent) -> Option<String> {
    agent
        .iroh
        .clone()
        .filter(|key| tunnel::public_key(key).is_ok())
}

fn answers(host: &str, port: u16) -> bool {
    let Ok(addresses) = (host, port).to_socket_addrs() else {
        return false;
    };
    addresses
        .into_iter()
        .any(|address| TcpStream::connect_timeout(&address, PROBE_TIMEOUT).is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    const KEY: &str = "ae58ff8833241ac82d6ff7611046ed67b5072d142c588d0063e942d9a75502b6";

    fn agent(host: &str, addresses: &[&str], port: Option<u16>) -> Agent {
        with_iroh(host, addresses, port, None)
    }

    fn with_iroh(host: &str, addresses: &[&str], port: Option<u16>, key: Option<&str>) -> Agent {
        Agent {
            name: "archbox".to_string(),
            host: host.to_string(),
            user: "me".to_string(),
            port,
            daemon_port: None,
            identity_file: None,
            known_hosts: None,
            program: None,
            addresses: addresses.iter().map(|a| a.to_string()).collect(),
            iroh: key.map(str::to_string),
            client: None,
            specs: None,
        }
    }

    #[test]
    fn the_first_preferred_address_that_answers_wins() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let box_ = agent("192.0.2.1", &["127.0.0.1"], Some(port));

        assert_eq!(probe(&box_), Route::to("127.0.0.1"));
    }

    #[test]
    fn with_nothing_answering_the_pairing_address_is_used() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let box_ = agent("192.0.2.1", &["127.0.0.1"], Some(port));

        assert_eq!(probe(&box_), Route::to("192.0.2.1"));
    }

    #[test]
    fn a_single_address_is_used_without_probing() {
        let box_ = agent("192.0.2.1", &[], None);

        assert_eq!(probe(&box_), Route::to("192.0.2.1"));
    }

    #[test]
    fn with_nothing_answering_iroh_is_used_when_the_box_has_a_key() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let box_ = with_iroh("127.0.0.1", &[], Some(port), Some(KEY));

        assert_eq!(
            probe(&box_),
            Route::Iroh {
                key: KEY.to_string()
            }
        );
    }

    #[test]
    fn probing_works_from_every_kind_of_runtime() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let box_ = agent("192.0.2.1", &["127.0.0.1"], Some(port));
        let expected = Route::to("127.0.0.1");

        let threads = tokio::runtime::Builder::new_multi_thread().build().unwrap();
        assert_eq!(threads.block_on(async { probe(&box_) }), expected);
        let single = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        assert_eq!(single.block_on(async { probe(&box_) }), expected);
    }

    #[test]
    fn a_direct_address_still_beats_iroh() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let box_ = with_iroh("127.0.0.1", &[], Some(port), Some(KEY));

        assert_eq!(probe(&box_), Route::to("127.0.0.1"));
    }
}
