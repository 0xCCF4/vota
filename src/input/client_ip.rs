//! The client IP of a request: the TCP peer, or, when the peer is a trusted
//! proxy, the right-most `X-Forwarded-For` address that isn't itself a trusted
//! proxy (addresses further left were supplied by the client and can be
//! spoofed).
//!
//! `X-Forwarded-For` is untrusted input and parsed strictly: a comma-separated
//! list of at most [`MAX_HOPS`] bare IPv4 or IPv6 addresses (no ports,
//! brackets or names), across all occurrences of the header in order. From a
//! trusted proxy, anything else is [`MalformedForwardedFor`]: the request is
//! refused (400) rather than counted as the proxy's own, which would let one
//! client spend the budget shared by everyone the proxy passes on without a
//! usable header. From any other peer the header is ignored.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use axum::http::HeaderMap;
use ipnet::IpNet;
use untrusted_value::{SanitizeValue, SanitizeWith as _, UntrustedValue};

use crate::config::Nat64Prefix;

/// The most addresses the header may list.
pub const MAX_HOPS: usize = 32;

/// The parsed `X-Forwarded-For` chain, left (client side) to right.
struct ForwardedFor(Vec<IpAddr>);

impl SanitizeValue<ForwardedFor> for UntrustedValue<Vec<String>> {
    type Error = &'static str;

    fn sanitize_value(self) -> Result<ForwardedFor, &'static str> {
        self.sanitize_with(|values| {
            let mut chain = Vec::new();
            for value in &values {
                for hop in value.split(',') {
                    let hop = hop.trim_matches([' ', '\t']);
                    // `IpAddr`'s parser rejects ports, brackets, zones and names.
                    let ip: IpAddr = hop.parse().map_err(|_| "not an IP address")?;
                    chain.push(ip.to_canonical());
                    if chain.len() > MAX_HOPS {
                        return Err("too many addresses");
                    }
                }
            }
            Ok(ForwardedFor(chain))
        })
    }
}

/// A trusted proxy passed on an `X-Forwarded-For` header that is not a list
/// of at most [`MAX_HOPS`] bare IP addresses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MalformedForwardedFor;

/// The client IP for a request from `peer` with these headers: the peer
/// itself, unless it is a trusted proxy that passed on `X-Forwarded-For`.
/// An IPv6 client address with an IPv4 address embedded in it
/// ([`embedded_ipv4`]) is that IPv4 address. Only the client found is
/// looked into: whether the peer and each hop are trusted proxies is decided
/// on the addresses as they are (with only `::ffff:a.b.c.d` taken as IPv4),
/// since an embedded address is only as good as the translator that put it
/// there, and that translator is not one of our proxies.
pub fn client_ip(
    peer: IpAddr,
    headers: &HeaderMap,
    trusted_proxies: &[IpNet],
    nat64_prefixes: &[Nat64Prefix],
) -> Result<IpAddr, MalformedForwardedFor> {
    let client = forwarded_client(peer, headers, trusted_proxies)?;
    Ok(match client {
        IpAddr::V6(v6) => embedded_ipv4(v6, nat64_prefixes).map_or(client, IpAddr::V4),
        IpAddr::V4(_) => client,
    })
}

/// The peer, or the client a trusted proxy named in `X-Forwarded-For`.
fn forwarded_client(
    peer: IpAddr,
    headers: &HeaderMap,
    trusted_proxies: &[IpNet],
) -> Result<IpAddr, MalformedForwardedFor> {
    let peer = peer.to_canonical();
    let is_trusted = |ip: &IpAddr| trusted_proxies.iter().any(|net| net.contains(ip));
    if !is_trusted(&peer) {
        return Ok(peer);
    }
    let mut values = Vec::new();
    for value in headers.get_all("x-forwarded-for") {
        // Only visible ASCII can be a list of addresses.
        let value = value.to_str().map_err(|_| MalformedForwardedFor)?;
        values.push(value.to_owned());
    }
    if values.is_empty() {
        return Ok(peer);
    }
    let ForwardedFor(chain) = UntrustedValue::from(values)
        .sanitize_value()
        .map_err(|_: &str| MalformedForwardedFor)?;
    // Right to left, skipping our own proxies. If every hop is trusted, the
    // request started inside the trusted network: take the left-most.
    Ok(chain
        .iter()
        .rev()
        .find(|ip| !is_trusted(ip))
        .or(chain.first())
        .copied()
        .unwrap_or(peer))
}

/// Whether a request comes straight from this host: its TCP peer (`None`
/// when unknown) is a loopback address, and it carries no `X-Forwarded-For`
/// or `Forwarded` header, whatever its value (even empty). A request a local
/// reverse proxy passes on carries one of them, so it doesn't count as local.
pub fn is_local_unproxied(peer: Option<IpAddr>, headers: &HeaderMap) -> bool {
    let loopback = peer.is_some_and(|peer| peer.to_canonical().is_loopback());
    loopback && !headers.contains_key("x-forwarded-for") && !headers.contains_key("forwarded")
}

/// The IPv4 address embedded in `ip`, if it has one:
///
/// - `::ffff:a.b.c.d`, IPv4-mapped;
/// - `::ffff:0:a.b.c.d`, IPv4-translated (`::ffff:0:0:0/96`, RFC 2765);
/// - `::a.b.c.d`, IPv4-compatible (`::/96`, deprecated), except `::` and
///   `::1`;
/// - `64:ff9b::a.b.c.d`, the well-known prefix `64:ff9b::/96` (RFC 6052);
/// - any address in the local-use prefix `64:ff9b:1::/48` (RFC 8215), taken
///   as a /96: its last 32 bits;
/// - any address in one of `nat64_prefixes` (the first that contains it),
///   decoded as RFC 6052 §2.2 specifies for its length.
pub fn embedded_ipv4(ip: Ipv6Addr, nat64_prefixes: &[Nat64Prefix]) -> Option<Ipv4Addr> {
    let bits = u128::from(ip);
    let low = Ipv4Addr::from(bits as u32);
    match bits >> 32 {
        // `::ffff:a.b.c.d`, IPv4-mapped.
        0xffff => return Some(low),
        // `::ffff:0:a.b.c.d`, IPv4-translated (RFC 2765).
        0xffff_0000 => return Some(low),
        // `::a.b.c.d`, IPv4-compatible (deprecated), but not `::` or `::1`.
        0 => return (bits > 1).then_some(low),
        // `64:ff9b::/96`, the well-known prefix (RFC 6052).
        WELL_KNOWN => return Some(low),
        _ => {}
    }
    // `64:ff9b:1::/48`, the local-use prefix (RFC 8215), taken as a /96.
    if bits >> 80 == LOCAL_USE {
        return Some(low);
    }
    let prefix = nat64_prefixes.iter().find(|p| p.net().contains(&ip))?;
    Some(rfc_6052_decode(ip, prefix.net().prefix_len()))
}

/// The IPv4 address after the first `prefix_len` bits of `ip` (a multiple of
/// 8, up to 96), skipping bits 64–71 (the u-octet), as RFC 6052 §2.2 places
/// it.
fn rfc_6052_decode(ip: Ipv6Addr, prefix_len: u8) -> Ipv4Addr {
    let octets = ip.octets();
    let mut v4 = (usize::from(prefix_len / 8)..16)
        .filter(|&i| i != 8)
        .map(|i| octets[i]);
    Ipv4Addr::from(std::array::from_fn(|_| v4.next().unwrap_or(0)))
}

/// `64:ff9b::/96`, as the address's top 96 bits.
const WELL_KNOWN: u128 = 0x0064_ff9b_0000_0000_0000_0000;

/// `64:ff9b:1::/48`, as the address's top 48 bits.
const LOCAL_USE: u128 = 0x0064_ff9b_0001;

#[cfg(test)]
mod tests {
    use super::*;

    fn embedded(ip: &str, prefixes: &[&str]) -> Option<Ipv4Addr> {
        let prefixes: Vec<Nat64Prefix> = prefixes.iter().map(|p| p.parse().unwrap()).collect();
        embedded_ipv4(ip.parse().unwrap(), &prefixes)
    }

    fn v4(ip: &str) -> Option<Ipv4Addr> {
        Some(ip.parse().unwrap())
    }

    #[test]
    fn an_ipv4_mapped_address_is_its_ipv4_address() {
        assert_eq!(embedded("::ffff:192.0.2.33", &[]), v4("192.0.2.33"));
    }

    #[test]
    fn an_ipv4_translated_address_is_its_ipv4_address() {
        assert_eq!(embedded("::ffff:0:192.0.2.33", &[]), v4("192.0.2.33"));
    }

    #[test]
    fn an_ipv4_compatible_address_is_its_ipv4_address() {
        assert_eq!(embedded("::192.0.2.33", &[]), v4("192.0.2.33"));
        assert_eq!(embedded("::0.0.0.2", &[]), v4("0.0.0.2"));
    }

    #[test]
    fn the_unspecified_and_loopback_addresses_are_not_ipv4_compatible() {
        assert_eq!(embedded("::", &[]), None);
        assert_eq!(embedded("::1", &[]), None);
    }

    #[test]
    fn an_address_in_the_well_known_prefix_is_its_ipv4_address() {
        assert_eq!(embedded("64:ff9b::192.0.2.33", &[]), v4("192.0.2.33"));
        assert_eq!(embedded("64:ff9b::7f00:1", &[]), v4("127.0.0.1"));
    }

    #[test]
    fn an_address_in_the_local_use_prefix_is_decoded_as_a_96() {
        assert_eq!(embedded("64:ff9b:1::192.0.2.33", &[]), v4("192.0.2.33"));
        assert_eq!(
            embedded("64:ff9b:1:abcd:1234:5678:c000:221", &[]),
            v4("192.0.2.33")
        );
    }

    /// The examples of RFC 6052 §2.4: 192.0.2.33 in each prefix length.
    const RFC_6052_EXAMPLES: [(&str, &str); 6] = [
        ("2001:db8::/32", "2001:db8:c000:221::"),
        ("2001:db8:100::/40", "2001:db8:1c0:2:21::"),
        ("2001:db8:122::/48", "2001:db8:122:c000:2:2100::"),
        ("2001:db8:122:300::/56", "2001:db8:122:3c0:0:221::"),
        ("2001:db8:122:344::/64", "2001:db8:122:344:c0:2:2100::"),
        ("2001:db8:122:344::/96", "2001:db8:122:344::192.0.2.33"),
    ];

    #[test]
    fn an_address_in_a_configured_prefix_is_decoded_as_rfc_6052_specifies() {
        for (prefix, ip) in RFC_6052_EXAMPLES {
            assert_eq!(
                embedded(ip, &[prefix]),
                v4("192.0.2.33"),
                "{ip} in {prefix}"
            );
        }
    }

    #[test]
    fn the_suffix_and_the_u_octet_are_ignored() {
        // Bits 64–71 (the u-octet) and the bits after the IPv4 address.
        assert_eq!(
            embedded("2001:db8:c000:221:ff00:0:0:1", &["2001:db8::/32"]),
            v4("192.0.2.33")
        );
        assert_eq!(
            embedded(
                "2001:db8:122:344:ffc0:2:21ff:ffff",
                &["2001:db8:122:344::/64"]
            ),
            v4("192.0.2.33")
        );
    }

    #[test]
    fn the_first_configured_prefix_that_contains_the_address_decodes_it() {
        let ip = "2001:db8:122:344:c0:2:2100::";
        assert_eq!(
            embedded(ip, &["2001:db8:ffff::/48", "2001:db8:122:344::/64"]),
            v4("192.0.2.33")
        );
        assert_eq!(embedded(ip, &["2001:db8:ffff::/48"]), None);
    }

    #[test]
    fn other_addresses_embed_none() {
        for ip in [
            "2001:db8::c000:221",
            "64:ff9b:0:0:1::c000:221",
            "64:ff9b:2::c000:221",
            "::1:0:c000:221",
            "fe80::c000:221",
        ] {
            assert_eq!(embedded(ip, &[]), None, "{ip}");
        }
    }
}
