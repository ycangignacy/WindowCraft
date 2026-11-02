// Made by ycangignacy
//! Adapter enumeration and DNS configuration through IP Helper.

use crate::error::{EngineError, Result};
use crate::win::{from_pcwstr, wide};
use std::net::IpAddr;
use windows::Win32::Foundation::{ERROR_BUFFER_OVERFLOW, ERROR_NO_DATA};
use windows::Win32::NetworkManagement::IpHelper::*;
use windows::Win32::NetworkManagement::Ndis::IfOperStatusUp;
use windows::Win32::Networking::WinSock::{AF_INET, AF_INET6, AF_UNSPEC, SOCKADDR_IN, SOCKADDR_IN6, SOCKET_ADDRESS};
use windows::core::{GUID, PCWSTR, PWSTR};
use windowcraft_config::DnsProvider;

const IF_TYPE_SOFTWARE_LOOPBACK: u32 = 24;
const IF_TYPE_TUNNEL: u32 = 131;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Adapter {
    pub guid: GUID,
    pub name: String,
    pub description: String,
    pub up: bool,
    pub dns_servers: Vec<IpAddr>,
}

/// # Safety
/// `address` must be a socket address returned by `GetAdaptersAddresses`.
unsafe fn socket_ip(address: &SOCKET_ADDRESS) -> Option<IpAddr> {
    if address.lpSockaddr.is_null() {
        return None;
    }
    // SAFETY: Guaranteed by the caller. The family field is common to all
    // sockaddr types and read before the concrete type is chosen.
    unsafe {
        match (*address.lpSockaddr).sa_family {
            AF_INET => {
                let raw = address.lpSockaddr.cast::<SOCKADDR_IN>();
                Some(IpAddr::from((*raw).sin_addr.S_un.S_addr.to_ne_bytes()))
            }
            AF_INET6 => {
                let raw = address.lpSockaddr.cast::<SOCKADDR_IN6>();
                Some(IpAddr::from((*raw).sin6_addr.u.Byte))
            }
            _ => None,
        }
    }
}

fn parse_guid(text: &str) -> Option<GUID> {
    let trimmed = text.trim_matches(['{', '}']).replace('-', "");
    u128::from_str_radix(&trimmed, 16).ok().map(GUID::from_u128)
}

pub fn adapters() -> Result<Vec<Adapter>> {
    let flags = GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST;
    let family = u32::from(AF_UNSPEC.0);
    let mut size = 16 * 1024u32;
    for _ in 0..4 {
        // u64 storage keeps the linked records aligned.
        let mut storage = vec![0u64; (size as usize).div_ceil(8)];
        let first = storage.as_mut_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>();
        // SAFETY: `storage` provides `size` writable, aligned bytes.
        let status = unsafe { GetAdaptersAddresses(family, flags, None, Some(first), &mut size) };
        if status == ERROR_BUFFER_OVERFLOW.0 {
            continue;
        }
        if status == ERROR_NO_DATA.0 {
            return Ok(Vec::new());
        }
        if status != 0 {
            return Err(EngineError::win32("GetAdaptersAddresses", "adapters", status));
        }

        let mut adapters = Vec::new();
        let mut cursor = first.cast_const();
        while !cursor.is_null() {
            // SAFETY: The API wrote a linked list inside `storage`, which
            // stays alive until this loop ends.
            let record = unsafe { &*cursor };
            cursor = record.Next;
            if record.IfType == IF_TYPE_SOFTWARE_LOOPBACK || record.IfType == IF_TYPE_TUNNEL {
                continue;
            }
            // SAFETY: The name is a valid ANSI string inside `storage`.
            let guid_text = unsafe { record.AdapterName.to_string() }.unwrap_or_default();
            let Some(guid) = parse_guid(&guid_text) else { continue };

            let mut dns_servers = Vec::new();
            let mut server = record.FirstDnsServerAddress.cast_const();
            while !server.is_null() {
                // SAFETY: As above, each node is inside `storage`.
                let node = unsafe { &*server };
                // SAFETY: The address belongs to the same API result.
                if let Some(ip) = unsafe { socket_ip(&node.Address) } {
                    dns_servers.push(ip);
                }
                server = node.Next;
            }
            adapters.push(Adapter {
                guid,
                // SAFETY: Both strings are valid and inside `storage`.
                name: unsafe { from_pcwstr(PCWSTR(record.FriendlyName.0)) },
                // SAFETY: As above.
                description: unsafe { from_pcwstr(PCWSTR(record.Description.0)) },
                up: record.OperStatus == IfOperStatusUp,
                dns_servers,
            });
        }
        return Ok(adapters);
    }
    Err(EngineError::Invalid("adapter list kept growing".to_owned()))
}

fn apply(guid: GUID, servers: &str, ipv6: bool, doh_template: Option<&str>, count: usize) -> Result<()> {
    let mut server_text = wide(servers);
    let mut template = doh_template.map(wide).unwrap_or_default();
    let mut doh = DNS_DOH_SERVER_SETTINGS {
        Template: PWSTR(template.as_mut_ptr()),
        // Fallback keeps name resolution alive if the endpoint is unreachable.
        Flags: u64::from(DNS_DOH_SERVER_SETTINGS_ENABLE | DNS_DOH_SERVER_SETTINGS_FALLBACK_TO_UDP),
    };
    let mut properties: Vec<DNS_SERVER_PROPERTY> = (0..count)
        .map(|index| DNS_SERVER_PROPERTY {
            Version: DNS_SERVER_PROPERTY_VERSION1,
            ServerIndex: index as u32,
            Type: DnsServerDohProperty,
            Property: DNS_SERVER_PROPERTY_TYPES { DohSettings: &raw mut doh },
        })
        .collect();

    let mut flags = u64::from(DNS_SETTING_NAMESERVER);
    if ipv6 {
        flags |= u64::from(DNS_SETTING_IPV6);
    }
    if doh_template.is_some() {
        flags |= u64::from(DNS_SETTING_DOH);
    }
    let settings = DNS_INTERFACE_SETTINGS3 {
        Version: DNS_INTERFACE_SETTINGS_VERSION3,
        Flags: flags,
        NameServer: PWSTR(server_text.as_mut_ptr()),
        cServerProperties: if doh_template.is_some() { properties.len() as u32 } else { 0 },
        ServerProperties: if doh_template.is_some() { properties.as_mut_ptr() } else { core::ptr::null_mut() },
        ..DNS_INTERFACE_SETTINGS3::default()
    };
    // SAFETY: The structure and everything it points at outlive the call. The
    // version field tells the API this is the larger V3 layout.
    let status = unsafe { SetInterfaceDnsSettings(guid, (&raw const settings).cast()) };
    if status.is_err() {
        return Err(EngineError::win32("SetInterfaceDnsSettings", servers, status.0));
    }
    Ok(())
}

/// Sets IPv4, IPv6 and DoH templates on every connected adapter.
pub fn set_provider(provider: &DnsProvider) -> Result<usize> {
    let v4 = format!("{},{}", provider.primary, provider.secondary);
    let v6 = format!("{},{}", provider.primary6, provider.secondary6);
    let mut changed = 0;
    for adapter in adapters()?.into_iter().filter(|adapter| adapter.up) {
        apply(adapter.guid, &v4, false, Some(&provider.doh_template), 2)?;
        apply(adapter.guid, &v6, true, Some(&provider.doh_template), 2)?;
        changed += 1;
    }
    flush_cache();
    Ok(changed)
}

/// Clears static servers so adapters return to DHCP-assigned DNS.
pub fn reset_to_dhcp() -> Result<usize> {
    let mut changed = 0;
    for adapter in adapters()?.into_iter().filter(|adapter| adapter.up) {
        apply(adapter.guid, "", false, None, 0)?;
        apply(adapter.guid, "", true, None, 0)?;
        changed += 1;
    }
    flush_cache();
    Ok(changed)
}

/// Flushes the resolver cache. Failure only delays propagation.
pub fn flush_cache() {
    type Flush = unsafe extern "system" fn() -> windows::core::BOOL;
    if let Ok(library) = crate::dynlib::DynLib::load("dnsapi.dll") {
        // SAFETY: The export takes no arguments and returns BOOL.
        if let Ok(flush) = unsafe { library.symbol::<Flush>(c"DnsFlushResolverCache") } {
            // SAFETY: `library` outlives the call.
            let _ = unsafe { flush() };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_guid_strings() {
        let guid = parse_guid("{A1B2C3D4-0000-1111-2222-333344445555}").unwrap();
        assert_eq!(guid, GUID::from_u128(0xA1B2C3D4_0000_1111_2222_333344445555));
        assert!(parse_guid("not-a-guid").is_none());
    }

    #[test]
    fn parse_guid_handles_unbraced_lowercase_boundaries_and_rejects_invalid_hex() {
        assert_eq!(
            parse_guid("a1b2c3d4-e5f6-7890-abcd-ef0123456789"),
            Some(GUID::from_u128(0xa1b2c3d4_e5f6_7890_abcd_ef0123456789))
        );
        assert_eq!(
            parse_guid("{00000000-0000-0000-0000-000000000000}"),
            Some(GUID::from_u128(0))
        );
        assert_eq!(
            parse_guid("{FFFFFFFF-FFFF-FFFF-FFFF-FFFFFFFFFFFF}"),
            Some(GUID::from_u128(u128::MAX))
        );
        assert!(parse_guid("").is_none());
        assert!(parse_guid("{}").is_none());
        assert!(parse_guid("----").is_none());
        assert!(parse_guid("{00000000-0000-0000-0000-00000000000G}").is_none());
        assert!(parse_guid("100000000000000000000000000000000").is_none());
    }

    #[test]
    fn enumerates_adapters_without_elevation() {
        let adapters = adapters().unwrap();
        assert!(adapters.iter().all(|adapter| !adapter.name.is_empty()));
        assert!(adapters.iter().all(|adapter| !adapter.description.is_empty()));
        assert!(adapters.iter().all(|adapter| adapter.guid != GUID::zeroed()));
    }

    #[test]
    fn settings_v3_is_the_larger_layout() {
        assert!(size_of::<DNS_INTERFACE_SETTINGS3>() > size_of::<DNS_INTERFACE_SETTINGS>());
    }
}
