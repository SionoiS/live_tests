use std::net::{IpAddr, Ipv4Addr};

use pnet::{
    datalink::{self},
    ipnetwork::IpNetwork,
};

/// Get the IP address of this container on the data network
pub fn get_data_network_ip(sidecar: bool, subnet: IpNetwork) -> IpAddr {
    if !sidecar {
        return IpAddr::V4(Ipv4Addr::LOCALHOST);
    }

    let all_interfaces = datalink::interfaces();

    for interface in all_interfaces {
        //println!("Interface => {:?}", interface);

        if !interface.is_up() || interface.is_loopback() {
            continue;
        }

        for ips in interface.ips {
            let ip = ips.ip();

            if ip.is_ipv4() && subnet.contains(ip) {
                return ip;
            }
        }
    }

    IpAddr::V4(Ipv4Addr::LOCALHOST)
}
