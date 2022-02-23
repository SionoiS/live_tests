use std::{
    collections::{HashSet, VecDeque},
    time::Duration,
};

use chrono::Utc;
use influxdb::{Timestamp, WriteQuery};

use libp2p::gossipsub::GossipsubMessage;

use testground::client::Client;
use tokio::{
    sync::mpsc::{UnboundedReceiver, UnboundedSender},
    time,
};
use tokio_stream::StreamExt;

use crate::{
    ipfs::IpfsClient,
    utils::{StreamerMessage, TestCaseParams},
    GOSSIPSUB_TOPIC,
};

pub struct VideoStream {
    ipfs: IpfsClient,
    testground: Client,
    block_sender: UnboundedSender<(u64, u64)>,
    params: TestCaseParams,
    sim_id: u64,

    //gossip
    last_gossip_time: u64,
    gossips: VecDeque<StreamerMessage>,

    /// Segment indices
    fetching: HashSet<u64>,

    // buffer
    last_segment_time: u64,
    timestamps: VecDeque<u64>,
    segments: VecDeque<u64>,

    current_segment: u64,
    stopped: u64,
    buffering: u64,
    buffer_size: u64,
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
            params,
            sim_id,

            last_gossip_time: 0,
            gossips: Default::default(),

            fetching: Default::default(),

            last_segment_time: 0,
            timestamps: Default::default(),
            segments: Default::default(),

            current_segment: 0,
            stopped: 0,
            buffering: 0,

            buffer_size: 30,
        }
    }

    pub fn add_gossip(&mut self, gossip: GossipsubMessage) {
        let receive_time = Utc::now().timestamp_millis();

        let msg: StreamerMessage =
            serde_json::from_slice(&gossip.data).expect("Message Deserialization");

        let latency = receive_time as u64 - msg.timestamp;
        let jitter = if self.last_gossip_time == 0 {
            0
        } else {
            (receive_time - self.last_gossip_time as i64)
                - (self.params.segment_length * 1000) as i64
        };

        self.last_gossip_time = receive_time as u64;

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

        self.gossips.push_back(msg);
    }

    pub fn add_segment(&mut self, segment_index: u64, timestamp: u64) {
        self.fetching.remove(&segment_index);

        let receive_time = Utc::now().timestamp_millis();

        let latency = receive_time as u64 - timestamp;
        let jitter = if self.last_segment_time == 0 {
            0
        } else {
            (receive_time - self.last_segment_time as i64)
                - (self.params.segment_length * 1000) as i64
        };

        self.last_segment_time = receive_time as u64;

        let query = WriteQuery::new(Timestamp::Milliseconds(timestamp as u128), "blocks")
            .add_field("latency", latency)
            .add_field("jitter", jitter)
            .add_tag("sim_id", self.sim_id)
            .add_tag("segment_number", segment_index);

        tokio::spawn({
            let testground = self.testground.clone();

            async move {
                if let Err(e) = testground.record_metric(query).await {
                    eprintln!("Metric Error: {:?}", e);
                }
            }
        });

        self.segments.push_back(segment_index);
        self.timestamps.push_back(timestamp);

        //sort
        let mut i = 1;
        while i < self.segments.len() {
            let mut j = i;

            while j > 0 && self.segments[j - 1] > self.segments[j] {
                self.segments.swap(j, j - 1);
                self.timestamps.swap(j, j - 1);

                j -= 1;
            }

            i += 1;
        }
    }

    pub fn advance_stream(&mut self) -> WriteQuery {
        let now_time = Utc::now();

        let mut query = WriteQuery::new(now_time.into(), "video")
            .add_field("gossips", self.gossips.len() as u64)
            .add_field("fetch", self.fetching.len() as u64)
            .add_field("buffer", self.segments.len() as u64)
            .add_tag("sim_id", self.sim_id);

        if self.buffering > 0 {
            self.buffering -= 1;
            self.stopped += 1;

            query = query.add_field("stop", self.stopped);
            return query;
        }

        if self.segments.is_empty() {
            self.stopped += 1;
            self.buffering = 2;

            query = query.add_field("stop", self.stopped);
            return query;
        }

        let mut segment_idx = self.segments.pop_front().unwrap();
        let mut timestamp = self.timestamps.pop_front().unwrap();

        let mut skipped_segments = 0;

        // If buffer head is too far
        if self.segments.back().is_some()
            && segment_idx < (*self.segments.back().unwrap()) - self.buffer_size
        {
            // Skip to mid buffer
            while self.segments.back().is_some()
                && segment_idx < *self.segments.back().unwrap() - (self.buffer_size / 2)
            {
                segment_idx = self.segments.pop_front().unwrap();
                timestamp = self.timestamps.pop_front().unwrap();

                skipped_segments += 1;
            }
        }

        let latency = now_time.timestamp_millis() as u64 - timestamp;
        self.stopped = 0;
        self.current_segment = segment_idx;

        query = query
            .add_field("latency", latency)
            .add_field("stop", self.stopped)
            .add_field("skip", skipped_segments);

        //clean up
        let indices: Vec<u64> = self
            .fetching
            .iter()
            .filter_map(|index| {
                if *index < self.current_segment {
                    Some(*index)
                } else {
                    None
                }
            })
            .collect();

        for index in indices {
            self.fetching.remove(&index);
        }

        query
    }

    pub fn _buffer_space(&self) -> usize {
        (self.buffer_size as usize).saturating_sub(self.segments.len() + self.fetching.len())
    }

    pub async fn run(&mut self, mut block_receiver: UnboundedReceiver<(u64, u64)>) {
        let sleep = time::sleep(Duration::from_secs(self.params.sim_time as u64));
        tokio::pin!(sleep);

        let mut interval = time::interval(Duration::from_secs(self.params.segment_length as u64));

        let mut stream = self
            .ipfs
            .subscribe(GOSSIPSUB_TOPIC)
            .await
            .expect("GossipSub Subcribe");

        loop {
            tokio::select! {
                biased;

                _ = &mut sleep => break,

                Some((block_count, timestamp)) = block_receiver.recv() => self.add_segment(block_count, timestamp),

                _ = interval.tick() => self.play(),

                Some(msg) = stream.next() => self.add_gossip(msg),
            }
        }
    }

    pub fn play(&mut self) {
        //for _ in 0..self.buffer_space() {}

        if let Some(msg) = self.gossips.pop_front() {
            let StreamerMessage {
                count,
                cids,
                timestamp,
            } = msg;

            self.fetching.insert(count);

            tokio::spawn({
                let ipfs = self.ipfs.clone();
                let sender = self.block_sender.clone();

                async move {
                    let handles: Vec<_> = cids.into_iter().map(|cid| ipfs.get_block(cid)).collect();

                    let _results = futures_util::future::join_all(handles).await;

                    let _ = sender.send((count, timestamp));
                }
            });
        }

        let query = self.advance_stream();

        tokio::spawn({
            let testground = self.testground.clone();

            async move {
                if let Err(e) = testground.record_metric(query).await {
                    eprintln!("Metric Error: {:?}", e);
                }
            }
        });
    }
}
