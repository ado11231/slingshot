//! What kind of network an address belongs to. The Agent uses it to label pairing codes and
//! the Client uses it to decide which address to try first, so both read addresses alike.

use std::net::IpAddr;

/// Declared in order of preference, so sorting by it puts the best path first. Judged from
/// the address itself rather than from how it was found, because an Agent routing everything
/// over a VPN would otherwise be mislabeled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Network {
    ThisMachine,
    Local,
    Other,
    Tailnet,
}

impl Network {
    /// Host names cannot be judged without a lookup, so they count as `Other`.
    pub fn of(host: &str) -> Network {
        let Ok(ip) = host.parse::<IpAddr>() else {
            return Network::Other;
        };
        let IpAddr::V4(v4) = ip else {
            return match ip.is_loopback() {
                true => Network::ThisMachine,
                false => Network::Other,
            };
        };
        let [first, second, ..] = v4.octets();

        match () {
            _ if v4.is_loopback() => Network::ThisMachine,
            _ if first == 100 && (64..128).contains(&second) => Network::Tailnet,
            _ if v4.is_private() => Network::Local,
            _ => Network::Other,
        }
    }

    /// How a path over this network is named in output, as in "via tailnet".
    pub fn name(self) -> &'static str {
        match self {
            Network::ThisMachine => "this machine",
            Network::Local => "local network",
            Network::Other => "network",
            Network::Tailnet => "tailnet",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tailnet_address_is_named() {
        assert_eq!(Network::of("100.67.90.119"), Network::Tailnet);
    }

    #[test]
    fn a_home_network_address_is_named() {
        assert_eq!(Network::of("192.168.1.20"), Network::Local);
        assert_eq!(Network::of("10.0.0.193"), Network::Local);
    }

    #[test]
    fn a_public_hundred_address_is_not_tailscale() {
        assert_eq!(Network::of("100.20.0.1"), Network::Other);
    }

    #[test]
    fn loopback_is_this_machine() {
        assert_eq!(Network::of("127.0.0.1"), Network::ThisMachine);
        assert_eq!(Network::of("::1"), Network::ThisMachine);
    }

    #[test]
    fn a_host_name_is_other() {
        assert_eq!(Network::of("archbox.local"), Network::Other);
    }

    #[test]
    fn the_local_network_is_preferred_over_the_tailnet() {
        assert!(Network::Local < Network::Tailnet);
        assert!(Network::Other < Network::Tailnet);
    }
}
