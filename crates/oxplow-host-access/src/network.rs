//! This machine on its networks: whether an address or a name is on a
//! private network (what the host's own certificate may vouch for), this
//! machine's private addresses, and the name it answers to on its local
//! network.

use std::net::IpAddr;

/// The names the host's own root may vouch for, below these.
pub const PRIVATE_DNS: &[&str] = &["local", "lan", "home.arpa", "internal", "ts.net"];

/// Whether `ip` is in a range the root may vouch for.
pub fn is_private(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || (o[0] == 100 && (o[1] & 0xc0) == 64)
        }
        IpAddr::V6(v6) => {
            let s = v6.segments();
            v6.is_loopback() || (s[0] & 0xfe00) == 0xfc00 || (s[0] & 0xffc0) == 0xfe80
        }
    }
}

/// Whether `name` is under a suffix the root may vouch for.
pub fn is_private_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    PRIVATE_DNS
        .iter()
        .any(|d| lower == *d || lower.ends_with(&format!(".{d}")))
}

/// This machine's addresses on private networks, for the Devices window to
/// offer: the ones another device can reach it by.
pub fn private_addresses() -> Vec<IpAddr> {
    let mut ips: Vec<IpAddr> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .map(|i| i.ip())
        .filter(|ip| is_private(*ip) && !ip.is_loopback())
        .filter(|ip| !matches!(ip, IpAddr::V6(v6) if (v6.segments()[0] & 0xffc0) == 0xfe80))
        .collect();
    ips.sort();
    ips.dedup();
    ips
}

/// The name this machine answers to on its local network: its local
/// hostname with `.local` (macOS's System Settings ▸ General ▸ Sharing ▸
/// Local hostname; elsewhere the hostname), the name a device on the same
/// network may reach it by. None when it has none.
pub fn local_name() -> Option<String> {
    #[cfg(target_os = "macos")]
    let raw = std::process::Command::new("scutil")
        .args(["--get", "LocalHostName"])
        .output();
    #[cfg(not(target_os = "macos"))]
    let raw = std::process::Command::new("hostname").output();
    let out = raw.ok().filter(|o| o.status.success())?;
    local_name_from(&String::from_utf8_lossy(&out.stdout))
}

/// `names` with this computer's own name (`local`, [`local_name`]) added,
/// as a host with its own certificate answers to it without being told: once,
/// and only a private name (the host's own certificate covers no other).
/// A device that cannot resolve it uses the address. Pure.
pub fn with_local_name(names: &[String], local: Option<&str>) -> Vec<String> {
    let mut all = names.to_vec();
    if let Some(local) = local.filter(|l| is_private_name(l)) {
        if !all.iter().any(|n| n.trim().eq_ignore_ascii_case(local)) {
            all.push(local.to_string());
        }
    }
    all
}

/// `raw` (a hostname as the system gives it) as a `.local` name: trimmed,
/// lower case, `.local` added to a bare name; none when blank. A name that
/// has a domain already keeps it. Pure.
pub fn local_name_from(raw: &str) -> Option<String> {
    let name = raw.trim().trim_end_matches('.').to_ascii_lowercase();
    if name.is_empty() {
        return None;
    }
    Some(if name.contains('.') {
        name
    } else {
        format!("{name}.local")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_computers_own_name_joins_the_names_once_and_only_when_private() {
        let given = vec!["mac.lan".to_string()];
        assert_eq!(
            with_local_name(&given, Some("studio.local")),
            ["mac.lan", "studio.local"]
        );
        // Already there (in any case): once.
        assert_eq!(
            with_local_name(&["Studio.local".to_string()], Some("studio.local")),
            ["Studio.local"]
        );
        // A hostname with a public domain is no name the host's own
        // certificate may cover: left out.
        assert_eq!(
            with_local_name(&given, Some("box.example.com")),
            ["mac.lan"]
        );
        assert_eq!(with_local_name(&given, None), ["mac.lan"]);
    }

    #[test]
    fn a_hostname_is_named_on_the_local_network_as_dot_local() {
        assert_eq!(
            local_name_from("Nathans-MacBook-Pro\n"),
            Some("nathans-macbook-pro.local".into())
        );
        assert_eq!(local_name_from("studio.local"), Some("studio.local".into()));
        assert_eq!(local_name_from("  \n"), None);
        assert!(
            is_private_name(&local_name_from("studio").unwrap()),
            "the host's own certificate covers it"
        );
    }

    #[test]
    fn private_means_the_ranges_a_home_or_a_tailnet_uses() {
        for ip in [
            "192.168.1.20",
            "10.0.0.5",
            "172.20.1.1",
            "100.101.102.103",
            "fd7a:115c::1",
        ] {
            assert!(is_private(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["8.8.8.8", "172.32.0.1", "100.128.0.1", "2001:db8::1"] {
            assert!(!is_private(ip.parse().unwrap()), "{ip}");
        }
    }
}
