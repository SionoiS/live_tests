#![allow(unused)]

use core::num;
use std::{
    collections::{HashMap, VecDeque},
    net::{IpAddr, Ipv4Addr},
    time::Duration,
};

use chrono::{TimeZone, Utc};
use influxdb::{Timestamp, WriteQuery};
use ipnetwork::IpNetwork;

use ipfs_bitswap::Block;

use libp2p::gossipsub::GossipsubMessage;
use rand::RngCore;

use cid::Cid;

use multihash::{Code, MultihashDigest};

use rand_xoshiro::rand_core::block;
use serde::{Deserialize, Serialize};
use serde_with::{serde_as, DisplayFromStr};
use testground::client::Client;
use tokio::{
    sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
    time,
};
use tokio_stream::{wrappers::UnboundedReceiverStream, StreamExt};

use crate::{ipfs::IpfsClient, GOSSIPSUB_TOPIC};

pub struct VideoStream {
    ipfs: IpfsClient,
    testground: Client,
    block_sender: UnboundedSender<(u64, u64)>,

    gossips_timestamp: VecDeque<u64>,
    gossips: VecDeque<StreamerMessage>,

    timestamps: VecDeque<u64>,
    forward_buf: VecDeque<u64>,
    current_segment: u64,

    params: TestCaseParams,
    sim_id: u64,
}

impl VideoStream {
    pub fn new(
        ipfs: IpfsClient,
        testground: Client,
        params: TestCaseParams,
        sim_id: u64,
        block_sender: UnboundedSender<(u64, u64)>,
    ) -> Self {
        Self {
            ipfs,
            testground,
            block_sender,

            gossips_timestamp: Default::default(),
            gossips: Default::default(),
            timestamps: Default::default(),
            forward_buf: Default::default(),
            current_segment: 0,
            params,
            sim_id,
        }
    }

    pub fn add_gossip(&mut self, gossip: GossipsubMessage) {
        let receive_time = Utc::now().timestamp_millis();

        let msg: StreamerMessage =
            serde_json::from_slice(&gossip.data).expect("Message Deserialization");

        let latency = receive_time as u64 - msg.timestamp;
        let jitter = if let Some(last_receive) = self.gossips_timestamp.back() {
            (receive_time - *last_receive as i64) - (self.params.segment_length * 1000) as i64
        } else {
            0
        };

        let query = WriteQuery::new(Timestamp::Milliseconds(msg.timestamp as u128), "gossips")
            .add_field("latency", latency)
            .add_field("jitter", jitter)
            .add_tag("sim_id", self.sim_id)
            .add_tag("segment_number", msg.count as u64);

        tokio::spawn({
            let testground = self.testground.clone();

            async move {
                if let Err(e) = testground.record_metric(query).await {
                    eprintln!("Metric Error: {:?}", e);
                }
            }
        });

        self.gossips_timestamp.push_back(receive_time as u64);
        self.gossips.push_back(msg);
    }

    pub fn add_segment(&mut self, block_count: u64, timestamp: u64) {
        let receive_time = Utc::now().timestamp_millis();

        let latency = receive_time as u64 - timestamp;
        let jitter = if let Some(last_receive) = self.forward_buf.back() {
            (receive_time - *last_receive as i64) - (self.params.segment_length * 1000) as i64
        } else {
            0
        };

        let query = WriteQuery::new(Timestamp::Milliseconds(timestamp as u128), "blocks")
            .add_field("latency", latency)
            .add_field("jitter", jitter)
            .add_tag("sim_id", self.sim_id)
            .add_tag("segment_number", block_count);

        tokio::spawn({
            let testground = self.testground.clone();

            async move {
                if let Err(e) = testground.record_metric(query).await {
                    eprintln!("Metric Error: {:?}", e);
                }
            }
        });

        if self.forward_buf.back().is_none() || *self.forward_buf.back().unwrap() == block_count - 1
        {
            self.forward_buf.push_back(block_count);
            self.timestamps.push_back(timestamp);
            return;
        }

        let last_count: u64 = *self.forward_buf.back().unwrap();

        if let Some(index) = self
            .forward_buf
            .len()
            .checked_sub((last_count - block_count) as usize)
        {
            self.forward_buf.insert(index, block_count);
            self.timestamps.insert(index, timestamp);
        }

        //Block arrived too late, discard
    }

    pub fn advance_stream(&mut self) -> WriteQuery {
        let now_time = Utc::now();
        let mut query = WriteQuery::new(now_time.into(), "video");

        if self.forward_buf.front().is_none() {
            query = query
                .add_field("buffering", true)
                .add_tag("sim_id", self.sim_id);

            return query;
        }

        let timestamp = *self.timestamps.front().unwrap();
        let new_segment = *self.forward_buf.front().unwrap();

        let skip = new_segment - (self.current_segment + 1);

        if skip > 0 {
            if !self.is_buffer_full() {
                query = query
                    .add_field("buffering", true)
                    .add_tag("sim_id", self.sim_id);

                return query;
            }

            query = query.add_field("skip", skip);
        } else {
            let latency = now_time.timestamp_millis() as u64 - timestamp;

            query = query.add_field("latency", latency);
        }

        query = query
            .add_tag("sim_id", self.sim_id)
            .add_tag("segment_number", self.current_segment);

        self.current_segment = new_segment;

        self.timestamps.pop_front();
        self.forward_buf.pop_front();

        query
    }

    pub fn is_buffer_full(&self) -> bool {
        self.forward_buf.len() >= 30 / self.params.segment_length
    }

    pub async fn run(&mut self, mut block_receiver: UnboundedReceiver<(u64, u64)>) {
        let mut stream = self
            .ipfs
            .subscribe(GOSSIPSUB_TOPIC)
            .await
            .expect("GossipSub Subcribe");

        let sleep = time::sleep(std::time::Duration::from_secs(self.params.sim_time as u64));

        tokio::pin!(sleep);

        loop {
            tokio::select! {
                biased;

                _ = &mut sleep => break,

                Some(msg) = stream.next() => self.add_gossip(msg),

                Some((block_count, timestamp)) = block_receiver.recv() => self.add_segment(block_count, timestamp),

                _ = self.play() => {}
            }
        }
    }

    pub async fn play(&mut self) {
        loop {
            time::sleep(Duration::from_secs(self.params.segment_length as u64)).await;

            let query = self.advance_stream();

            tokio::spawn({
                let testground = self.testground.clone();

                async move {
                    if let Err(e) = testground.record_metric(query).await {
                        eprintln!("Metric Error: {:?}", e);
                    }
                }
            });

            if self.is_buffer_full() {
                continue;
            }

            let (msg, timestamp) =
                match (self.gossips.pop_front(), self.gossips_timestamp.pop_front()) {
                    (Some(msg), Some(timestamp)) => (msg, timestamp),
                    _ => continue,
                };

            let StreamerMessage {
                count,
                cids,
                timestamp,
            } = msg;

            tokio::spawn({
                let ipfs = self.ipfs.clone();
                let testground = self.testground.clone();
                let sender = self.block_sender.clone();

                async move {
                    let handles: Vec<_> = cids.into_iter().map(|cid| ipfs.get_block(cid)).collect();

                    let _results = futures_util::future::join_all(handles).await;

                    let _ = sender.send((count, timestamp));
                }
            });
        }
    }
}

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
    use rand_xoshiro::{Xoshiro256StarStar, Xoshiro512StarStar};

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
