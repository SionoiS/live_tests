use std::net::{IpAddr, Ipv4Addr};

use pnet::{datalink, ipnetwork::IpNetwork};

use ipfs_bitswap::Block;

use rand::RngCore;

use cid::{Cid, Codec};

use multihash::Sha2_256;

/// Get the IP address of this container on the data network
pub fn data_network_ip(sidecar: bool, subnet: IpNetwork) -> IpAddr {
    if !sidecar {
        return IpAddr::V4(Ipv4Addr::LOCALHOST);
    }

    let all_interfaces = datalink::interfaces();

    for interface in all_interfaces {
        //println!("Interface => {:?}", interface);

        if !interface.is_up() || interface.is_loopback() || interface.ips.is_empty() {
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

pub fn get_random_block(rng: &mut impl RngCore) -> Block {
    let mut data = [0u8; 1024];
    rng.fill_bytes(&mut data);

    let cid = Cid::new_v1(Codec::Raw, Sha2_256::digest(&data));

    Block::new(Box::new(data), cid.clone())
}
