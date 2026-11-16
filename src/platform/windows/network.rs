//! Network: default route (`GetAdaptersAddresses` + gateway) and, if there is one, a
//! TCP probe to `1.1.1.1:443` without sending data. Same semantics as on Linux.

use std::ffi::c_void;
use std::net::{SocketAddr, TcpStream};
use std::ptr::null;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::ERROR_BUFFER_OVERFLOW;
use windows_sys::Win32::NetworkManagement::IpHelper::{
    GAA_FLAG_INCLUDE_GATEWAYS, GetAdaptersAddresses, IF_TYPE_SOFTWARE_LOOPBACK, IF_TYPE_TUNNEL,
    IP_ADAPTER_ADDRESSES_LH,
};
use windows_sys::Win32::NetworkManagement::Ndis::IfOperStatusUp;
use windows_sys::Win32::Networking::WinSock::AF_UNSPEC;

use crate::event::{Event, Network};

const PROBE_TARGET: SocketAddr = SocketAddr::V4(std::net::SocketAddrV4::new(
    std::net::Ipv4Addr::new(1, 1, 1, 1),
    443,
));
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);
const ROUTE_POLL: Duration = Duration::from_secs(5);
const PROBE_EVERY: Duration = Duration::from_secs(30);

pub(crate) fn listen(emit: impl Fn(Event) -> bool + Send + 'static) {
    std::thread::spawn(move || {
        let mut last: Option<Network> = None;
        let mut had_route = false;
        let mut last_probe: Option<(Instant, bool)> = None;

        loop {
            let has_route = has_default_route();
            let network = if has_route {
                // Probe when the route appears and then every PROBE_EVERY.
                let due =
                    !had_route || last_probe.is_none_or(|(at, _)| at.elapsed() >= PROBE_EVERY);
                if due {
                    last_probe = Some((Instant::now(), probe()));
                }
                match last_probe {
                    Some((_, true)) => Network::Online,
                    _ => Network::NoInternet,
                }
            } else {
                last_probe = None;
                Network::Offline
            };

            had_route = has_route;
            if last != Some(network) {
                last = Some(network);
                if !emit(Event::Network(network)) {
                    return;
                }
            }

            std::thread::sleep(ROUTE_POLL);
        }
    });
}

fn probe() -> bool {
    TcpStream::connect_timeout(&PROBE_TARGET, PROBE_TIMEOUT).is_ok()
}

/// There is at least one active, non-loopback/tunnel adapter with a gateway.
fn has_default_route() -> bool {
    let flags = GAA_FLAG_INCLUDE_GATEWAYS;
    let mut size = 0u32;

    // SAFETY: size query; a null buffer is valid and `size` is an out.
    unsafe {
        GetAdaptersAddresses(
            AF_UNSPEC as u32,
            flags,
            null::<c_void>(),
            std::ptr::null_mut(),
            &mut size,
        );
    }

    if size == 0 {
        return false;
    }

    let mut buffer = Vec::<u64>::new();
    let mut succeeded = false;

    for _ in 0..3 {
        buffer = vec![0u64; (size as usize).div_ceil(8)];

        // SAFETY: `buffer` is 8-byte aligned and at least `size` long;
        // the API writes the adapter list into it.
        let result = unsafe {
            GetAdaptersAddresses(
                AF_UNSPEC as u32,
                flags,
                null::<c_void>(),
                buffer.as_mut_ptr().cast(),
                &mut size,
            )
        };

        if result == 0 {
            succeeded = true;
            break;
        }
        if result != ERROR_BUFFER_OVERFLOW {
            return false;
        }
    }

    if !succeeded {
        return false;
    }

    // SAFETY: the list pointers point into `buffer`, which stays alive
    // until the function returns.
    let mut current = buffer.as_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>();
    unsafe {
        while !current.is_null() {
            let adapter = &*current;
            if usable(
                adapter.OperStatus,
                adapter.IfType,
                !adapter.FirstGatewayAddress.is_null(),
            ) {
                return true;
            }
            current = adapter.Next;
        }
    }

    false
}

fn usable(oper_status: i32, if_type: u32, has_gateway: bool) -> bool {
    oper_status == IfOperStatusUp
        && if_type != IF_TYPE_SOFTWARE_LOOPBACK
        && if_type != IF_TYPE_TUNNEL
        && has_gateway
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_up_non_virtual_adapters_with_gateway() {
        assert!(usable(IfOperStatusUp, 6, true));
        assert!(!usable(IfOperStatusUp, IF_TYPE_SOFTWARE_LOOPBACK, true));
        assert!(!usable(IfOperStatusUp, IF_TYPE_TUNNEL, true));
        assert!(!usable(IfOperStatusUp, 6, false));
        assert!(!usable(2, 6, true), "adapter down");
    }

    /// `cargo test -- --ignored`: uses the machine's network.
    #[test]
    #[ignore = "uses the machine's network"]
    fn checks_real_network() {
        let route = has_default_route();
        println!("default route: {route}, internet: {}", route && probe());
    }
}
