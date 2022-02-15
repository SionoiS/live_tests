#![allow(unused)]

use core::num;
use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr},
};

use chrono::Utc;
use influxdb::Timestamp;
use ipnetwork::IpNetwork;

use ipfs_bitswap::Block;

use rand::RngCore;

use cid::Cid;

use multihash::{Code, MultihashDigest};

use serde::{Deserialize, Serialize};
use serde_with::{serde_as, DisplayFromStr};

use crate::ipfs::IpfsClient;

/// Get the IP address of this container on the data network
pub fn data_network_ip(sidecar: bool, subnet: IpNetwork) -> IpAddr {
    if !sidecar {
        return IpAddr::V4(Ipv4Addr::LOCALHOST);
    }

    let all_interfaces = pnet_datalink::interfaces();

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

#[serde_as]
#[derive(Serialize, Deserialize)]
pub struct StreamerMessage {
    pub count: u64,

    pub timestamp: i64,

    #[serde_as(as = "Vec<DisplayFromStr>")]
    pub cids: Vec<Cid>,
}

pub fn generate_blocks(
    ipfs: &IpfsClient,
    rng: &mut impl RngCore,
    blocks: usize,
    max_size: bool,
) -> Vec<Cid> {
    let mut cids = Vec::with_capacity(blocks);

    for _ in 0..blocks {
        let block = if max_size {
            random_max_block(rng)
        } else {
            random_std_block(rng)
        };

        cids.push(*block.cid());

        tokio::spawn({
            let ipfs = ipfs.clone();

            async move { ipfs.add_block(block).await }
        });
    }

    cids
}

/// Randomly generate a block.
///
/// 262144 bytes is standard chunker length.
pub fn random_std_block(rng: &mut impl RngCore) -> Block {
    let mut data = [0u8; 262144];
    rng.fill_bytes(&mut data);

    let cid = Cid::new_v1(0x0, Code::Sha2_256.digest(&data));

    Block::new(Box::new(data), cid.clone())
}

/// Randomly generate a block.
///
/// 524288 bytes is unofficial max length.
pub fn random_max_block(rng: &mut impl RngCore) -> Block {
    let mut data = [0u8; 524288];
    rng.fill_bytes(&mut data);

    let cid = Cid::new_v1(0x0, Code::Sha2_256.digest(&data));

    Block::new(Box::new(data), cid.clone())
}

#[derive(PartialEq, Debug)]
pub struct TestCaseParams {
    pub log: bool,
    pub log_id: usize,

    pub max_concurrent_send: usize,

    pub max_size_block: bool,

    pub segment_length: usize,

    pub sim_time: usize,
}

pub fn custom_instance_params(params: &str) -> TestCaseParams {
    let mut table: HashMap<String, String> = Default::default();

    for item in params.split('|') {
        let mut iter = item.split('=');

        let name = match iter.next() {
            Some(n) => n,
            None => continue,
        };

        let value = match iter.next() {
            Some(v) => v,
            None => continue,
        };

        table.insert(name.to_owned(), value.to_owned());
    }

    let log = table["log"].parse().expect("Boolean Parsing");
    let log_id = table["log_id"].parse().expect("Integer Parsing");
    let max_concurrent_send = table["max_concurrent_send"]
        .parse()
        .expect("Integer Parsing");
    let max_size_block = table["max_size_block"].parse().expect("Boolean Parsing");
    let segment_length = table["segment_length"].parse().expect("Integer Parsing");
    let sim_time = table["sim_time"].parse().expect("Integer Parsing");

    TestCaseParams {
        log,
        log_id,
        max_concurrent_send,
        max_size_block,
        segment_length,
        sim_time,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_test() {
        let params =
            "log=false|log_id=12|max_concurrent_send=2|max_size_block=false|segment_length=1|sim_time=600";

        let values = TestCaseParams {
            log: false,
            log_id: 12,
            max_concurrent_send: 2,
            max_size_block: false,
            segment_length: 1,
            sim_time: 600,
        };

        assert_eq!(values, custom_instance_params(&params))
    }
}
