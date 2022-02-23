use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr},
};

use ipnetwork::IpNetwork;

use ipfs_bitswap::Block;

use rand::RngCore;

use cid::Cid;

use multihash::{Code, MultihashDigest};

use serde::{Deserialize, Serialize};
use serde_with::{serde_as, DisplayFromStr};

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

    pub timestamp: u64,

    #[serde_as(as = "Vec<DisplayFromStr>")]
    pub cids: Vec<Cid>,
}

pub fn generate_blocks(
    rng: &mut impl RngCore,
    bitrate: usize,
    segment_length: usize,
    max_size: bool,
) -> Vec<Block> {
    let total_bytes = bitrate * segment_length / 8;

    let block_size = if max_size {
        MAX_BLOCK_SIZE
    } else {
        STANDARD_BLOCK_SIZE
    };

    let (mut num_blocks, remainder_bytes) = {
        let mut div = total_bytes / block_size;
        let rem = total_bytes % block_size;

        if rem > 0 {
            div += 1;
        }

        (div, rem)
    };

    let mut cids = Vec::with_capacity(num_blocks);

    loop {
        num_blocks -= 1;

        let block_size = if num_blocks == 0 && remainder_bytes > 0 {
            remainder_bytes
        } else {
            block_size
        };

        let block = random_block(rng, block_size);

        cids.push(block);

        if num_blocks == 0 {
            break;
        }
    }

    cids
}

pub const MAX_BLOCK_SIZE: usize = 524288;
pub const STANDARD_BLOCK_SIZE: usize = 262144;

/// Randomly generate a block.
pub fn random_block(rng: &mut impl RngCore, block_size: usize) -> Block {
    let mut data = vec![0; block_size];

    rng.fill_bytes(&mut data);

    let cid = Cid::new_v1(0x00, Code::Sha2_256.digest(&data));

    Block::new(data.into_boxed_slice(), cid)
}

#[derive(PartialEq, Debug)]
pub struct TestCaseParams {
    pub log: bool,
    pub log_id: usize,

    pub max_concurrent_send: usize,

    pub max_size_block: bool,

    pub segment_length: usize,

    pub sim_time: usize,

    /// ~2000000 bps ->  480p30
    ///
    /// ~3000000 bps -> 720p30
    ///
    /// ~4500000 bps -> 720p60
    ///
    /// ~6000000 bps -> 1080p60
    pub video_bitrate: usize,

    pub network_bandwidth: usize,
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
    let video_bitrate = table["video_bitrate"].parse().expect("Integer Parsing");
    let network_bandwidth = table["network_bandwidth"].parse().expect("Integer Parsing");

    TestCaseParams {
        log,
        log_id,
        max_concurrent_send,
        max_size_block,
        segment_length,
        sim_time,
        video_bitrate,
        network_bandwidth,
    }
}

#[cfg(test)]
mod tests {
    use rand::SeedableRng;
    use rand_xoshiro::Xoshiro256StarStar;

    use super::*;

    #[test]
    fn serde_test() {
        let params =
            "log=false|log_id=12|max_concurrent_send=4|max_size_block=false|segment_length=1|sim_time=600|video_bitrate=6000000|network_bandwidth=10000000";

        let values = TestCaseParams {
            log: false,
            log_id: 12,
            max_concurrent_send: 4,
            max_size_block: false,
            segment_length: 1,
            sim_time: 600,
            video_bitrate: 6_000_000,
            network_bandwidth: 10_000_000,
        };

        assert_eq!(values, custom_instance_params(&params))
    }

    #[test]
    fn block_test() {
        let mut rng = Xoshiro256StarStar::seed_from_u64(79082340324789u64);

        let bitrate = 6000000;
        let segment_length = 4;
        let max_size = false;

        let blocks = generate_blocks(&mut rng, bitrate, segment_length, max_size);

        for block in blocks.iter() {
            println!("CID -> {}", block.cid());
        }

        assert_eq!(blocks.len(), 12);
    }
}
