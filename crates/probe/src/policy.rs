//! Destination address policy: a file-controlled CIDR allowlist on top of a
//! fixed set of address classes that are never reachable, whatever the list says.

use crate::Failure;
use ipnet::IpNet;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

pub(crate) struct AddressPolicy {
    allowed: Vec<IpNet>,
}

impl AddressPolicy {
    /// Entries are CIDRs or bare addresses (a single-host network).
    pub(crate) fn new(cidrs: &[String]) -> Result<Self, Failure> {
        let mut allowed = Vec::with_capacity(cidrs.len());
        for raw in cidrs {
            let entry = raw.trim();
            let net = entry
                .parse::<IpNet>()
                .or_else(|_| entry.parse::<IpAddr>().map(IpNet::from))
                .map_err(|_| Failure::new(format!("network policy has invalid CIDR `{entry}`")))?;
            let net = net.trunc();
            // Allowlist entries are compared with canonical addresses, so an
            // IPv4-mapped IPv6 entry is the IPv4 network it embeds.
            let net = match net {
                IpNet::V6(v6) if v6.prefix_len() >= 96 => match v6.addr().to_ipv4_mapped() {
                    Some(v4) => IpNet::new(IpAddr::V4(v4), v6.prefix_len() - 96)
                        .expect("prefix within IPv4 range"),
                    None => net,
                },
                _ => net,
            };
            allowed.push(net);
        }
        Ok(Self { allowed })
    }

    /// Returns the canonical address when the destination may be contacted.
    pub(crate) fn permit(&self, ip: IpAddr) -> Result<IpAddr, String> {
        let ip = ip.to_canonical();
        if let Some(reason) = denied_class(ip) {
            return Err(format!("{ip} ({reason})"));
        }
        if self.allowed.iter().any(|net| net.contains(&ip)) {
            Ok(ip)
        } else {
            Err(format!("{ip} (not in allowed_cidrs)"))
        }
    }
}

/// Notification destinations: public addresses are reachable, private and
/// shared networks only when the allowlist covers them, and the always-denied
/// classes never. Returns the canonical address when permitted.
pub(crate) fn permit_notification(policy: &AddressPolicy, ip: IpAddr) -> Result<IpAddr, String> {
    let ip = ip.to_canonical();
    if let Some(reason) = denied_class(ip) {
        return Err(format!("{ip} ({reason})"));
    }
    let private = match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            v4.is_private()
                || (o[0] == 100 && (64..128).contains(&o[1]))
                || (o[0] == 198 && (o[1] == 18 || o[1] == 19))
        }
        IpAddr::V6(v6) => v6.segments()[0] & 0xfe00 == 0xfc00,
    };
    if private {
        policy
            .permit(ip)
            .map_err(|_| format!("{ip} (private address not in allowed_cidrs)"))
    } else {
        Ok(ip)
    }
}

/// Address classes that are denied even when an allowlist entry covers them.
pub(crate) fn denied_class(ip: IpAddr) -> Option<&'static str> {
    match ip.to_canonical() {
        IpAddr::V4(v4) => denied_v4(v4),
        IpAddr::V6(v6) => denied_v6(v6),
    }
}

const METADATA_V4: [Ipv4Addr; 3] = [
    Ipv4Addr::new(169, 254, 169, 254), // AWS, GCP, Azure, OCI, OpenStack
    Ipv4Addr::new(100, 100, 100, 200), // Alibaba Cloud
    Ipv4Addr::new(168, 63, 129, 16),   // Azure WireServer
];
const METADATA_V6: [Ipv6Addr; 1] = [Ipv6Addr::new(0xfd00, 0x0ec2, 0, 0, 0, 0, 0, 0x0254)];

fn denied_v4(ip: Ipv4Addr) -> Option<&'static str> {
    let o = ip.octets();
    if METADATA_V4.contains(&ip) {
        Some("cloud metadata")
    } else if ip.is_loopback() {
        Some("loopback")
    } else if ip.is_link_local() {
        Some("link-local")
    } else if o[0] == 0 {
        Some("unspecified")
    } else if ip.is_multicast() {
        Some("multicast")
    } else if o[0] >= 240 {
        Some("reserved or broadcast")
    } else {
        None
    }
}

fn denied_v6(ip: Ipv6Addr) -> Option<&'static str> {
    let s = ip.segments();
    let embedded = |hi: u16, lo: u16| Ipv4Addr::from(((hi as u32) << 16) | lo as u32);
    if METADATA_V6.contains(&ip) {
        Some("cloud metadata")
    } else if ip.is_unspecified() {
        Some("unspecified")
    } else if ip.is_loopback() {
        Some("loopback")
    } else if s[0] & 0xffc0 == 0xfe80 {
        Some("link-local")
    } else if s[0] & 0xffc0 == 0xfec0 {
        Some("site-local")
    } else if ip.is_multicast() {
        Some("multicast")
    } else if s[..6] == [0; 6] {
        // ::a.b.c.d, the deprecated IPv4-compatible form.
        Some("IPv4-compatible IPv6")
    } else if s[0] == 0x0064 && s[1] == 0xff9b {
        // 64:ff9b::/96 translates to the embedded IPv4 address; local-use
        // NAT64 (64:ff9b:1::/48) embeds it at operator-chosen offsets.
        if s[2..6] == [0; 4] {
            denied_v4(embedded(s[6], s[7])).map(|_| "NAT64 to a denied IPv4 address")
        } else {
            Some("local-use NAT64")
        }
    } else if s[0] == 0x2002 {
        denied_v4(embedded(s[1], s[2])).map(|_| "6to4 to a denied IPv4 address")
    } else if s[0] == 0x2001 && s[1] == 0 {
        Some("Teredo tunnel")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(cidrs: &[&str]) -> AddressPolicy {
        AddressPolicy::new(&cidrs.iter().map(|c| c.to_string()).collect::<Vec<_>>()).unwrap()
    }

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn denied_classes_win_over_broad_allowlists() {
        let p = policy(&["0.0.0.0/0", "::/0"]);
        for addr in [
            "127.0.0.1",
            "127.8.9.10",
            "169.254.169.254",
            "169.254.0.21",
            "100.100.100.200",
            "168.63.129.16",
            "0.0.0.0",
            "0.1.2.3",
            "224.0.0.1",
            "239.255.255.250",
            "255.255.255.255",
            "240.0.0.1",
            "::",
            "::1",
            "fe80::1",
            "febf::1",
            "fec0::1",
            "ff02::1",
            "fd00:ec2::254",
            "::ffff:127.0.0.1",
            "::ffff:169.254.169.254",
            "::ffff:0.0.0.0",
            "::127.0.0.1",
            "::10.0.0.1",
            "64:ff9b::7f00:1",
            "64:ff9b::a9fe:a9fe",
            "2002:7f00:1::",
            "2002:a9fe:a9fe::1",
            "2001:0:4136:e378::1",
        ] {
            assert!(p.permit(ip(addr)).is_err(), "{addr} should be denied");
        }
    }

    #[test]
    fn allowlist_is_deny_by_default_and_exact() {
        let empty = policy(&[]);
        assert!(empty.permit(ip("10.0.0.1")).is_err());
        assert!(empty.permit(ip("8.8.8.8")).is_err());

        let p = policy(&["10.203.44.0/24", "192.168.1.7", "fd7a:5e57:44::/64"]);
        assert_eq!(p.permit(ip("10.203.44.9")), Ok(ip("10.203.44.9")));
        assert!(p.permit(ip("10.203.45.1")).is_err());
        assert!(p.permit(ip("10.203.43.255")).is_err());
        assert!(p.permit(ip("192.168.1.7")).is_ok());
        assert!(p.permit(ip("192.168.1.8")).is_err());
        assert!(p.permit(ip("fd7a:5e57:44::1")).is_ok());
        assert!(p.permit(ip("fd7a:5e57:45::1")).is_err());
        // Mapped forms are the same destination as their IPv4 address.
        assert_eq!(p.permit(ip("::ffff:10.203.44.9")), Ok(ip("10.203.44.9")));
        assert!(p.permit(ip("::ffff:10.203.45.9")).is_err());
        // Public NAT64 and 6to4 are only reachable when allowlisted themselves.
        assert!(p.permit(ip("64:ff9b::acb:2c09")).is_err());
    }

    #[test]
    fn mapped_allowlist_entries_match_ipv4() {
        let p = policy(&["::ffff:10.0.0.0/104"]);
        assert!(p.permit(ip("10.1.2.3")).is_ok());
        assert!(p.permit(ip("11.1.2.3")).is_err());
    }

    #[test]
    fn invalid_cidrs_are_rejected() {
        assert!(AddressPolicy::new(&["10.0.0.0/33".into()]).is_err());
        assert!(AddressPolicy::new(&["lan".into()]).is_err());
    }

    #[test]
    fn notifications_reach_public_addresses_and_only_listed_private_ones() {
        let p = policy(&["10.203.44.0/24"]);
        assert!(permit_notification(&p, ip("149.154.167.220")).is_ok());
        assert!(permit_notification(&p, ip("2606:4700::6810:84e5")).is_ok());
        assert!(permit_notification(&p, ip("10.203.44.9")).is_ok());
        for addr in [
            "10.1.1.1",
            "192.168.1.5",
            "172.16.0.1",
            "100.64.0.1",
            "fd00::1",
            "::ffff:192.168.1.5",
        ] {
            assert!(permit_notification(&p, ip(addr)).is_err(), "{addr}");
        }
        let all = policy(&["0.0.0.0/0", "::/0"]);
        for addr in ["127.0.0.1", "169.254.169.254", "::1", "fe80::1"] {
            assert!(permit_notification(&all, ip(addr)).is_err(), "{addr}");
        }
    }

    #[test]
    fn tailscale_cgnat_is_allowed_when_listed() {
        let p = policy(&["100.64.0.0/10"]);
        assert!(p.permit(ip("100.101.102.103")).is_ok());
        assert!(p.permit(ip("100.100.100.200")).is_err());
    }
}
