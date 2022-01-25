mod composition;
mod ipfs;
mod synchronization;
mod utils;

use std::{path::PathBuf, time::Duration};

use ipfs::IpfsClient;
use structopt::StructOpt;

use synchronization::*;
use tokio::time;
use utils::*;

use pnet::ipnetwork::IpNetwork;

use futures_util::stream::StreamExt;

use libp2p::{multiaddr::Protocol, Multiaddr, PeerId};

#[derive(StructOpt)]
#[allow(dead_code)]
struct Arguments {
    //PATH: /usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
    #[structopt(env)]
    hostname: Option<String>, // HOSTNAME: e6f4cc8fc147
    #[structopt(env)]
    test_temp_path: Option<String>, // TEST_TEMP_PATH: /temp
    #[structopt(env)]
    test_plan: String, // TEST_PLAN: streaming_test
    #[structopt(env)]
    test_branch: Option<String>, // TEST_BRANCH:
    #[structopt(env)]
    test_instance_count: usize, // TEST_INSTANCE_COUNT: 1
    #[structopt(env)]
    test_instance_role: Option<String>, // TEST_INSTANCE_ROLE:
    #[structopt(env)]
    test_outputs_path: Option<PathBuf>, // TEST_OUTPUTS_PATH: /outputs
    #[structopt(env)]
    test_run: String, // TEST_RUN: c7fjstge5te621cen4i0
    #[structopt(long, env)]
    test_sidecar: bool, // TEST_SIDECAR: true
    #[structopt(env)]
    test_group_instance_count: Option<usize>, // TEST_GROUP_INSTANCE_COUNT: 1
    #[structopt(env)]
    test_group_id: String, // TEST_GROUP_ID: single
    #[structopt(env)]
    test_instance_params: Option<String>, // TEST_INSTANCE_PARAMS:
    #[structopt(env)]
    test_repo: Option<String>, //TEST_REPO:
    #[structopt(env)]
    test_tag: Option<String>, // TEST_TAG:
    #[structopt(env)]
    test_disable_metrics: Option<bool>, // TEST_DISABLE_METRICS: false
    #[structopt(env)]
    test_case: String, // TEST_CASE: quickstart
    #[structopt(env)]
    test_subnet: IpNetwork, // TEST_SUBNET: 16.0.0.0/16
    #[structopt(env)]
    test_capture_profiles: Option<String>, // TEST_CAPTURE_PROFILES:
    #[structopt(env)]
    test_start_time: Option<String>, // TEST_START_TIME: 2022-01-12T15:48:07-05:00
    #[structopt(env)]
    influxdb_url: Option<String>, // INFLUXDB_URL: http://testground-influxdb:8086
    #[structopt(env)]
    redis_host: String, // REDIS_HOST: testground-redis
                        // HOME: /
}

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const REDIS_TOPIC: &str = "addr_ex";
const INIT_STATE: &str = "init";
const NET_STATE: &str = "net";

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    let args = Arguments::from_args();

    let sync_client = SyncClient::new(&args.redis_host).await?;

    let local_ip = data_network_ip(args.test_sidecar, args.test_subnet);

    let mut local_multi_addr: Multiaddr = local_ip.into();
    local_multi_addr.push(Protocol::Tcp(4001));

    let (ipfs, handle, local_peer_id) = utils::compose_network();

    if let Err(e) = ipfs.listen_on(local_multi_addr.clone()).await {
        eprintln!("{:?}", e);
    }

    let sim_id = sync_client.signal_entry(INIT_STATE).await?;

    let mut stream = sync_client.subscribe(REDIS_TOPIC).await?;

    // Barrier waiting for every container to initialize and subscribe.
    sync_client
        .barrier(INIT_STATE, args.test_instance_count)
        .await?;

    let msg = format!(
        "{} {}",
        local_peer_id.to_base58(),
        local_multi_addr.to_string()
    );

    // Send PeerId & MultiAddress to all other nodes.
    sync_client.publish(REDIS_TOPIC, msg).await?;

    let mut peer_ids: Vec<PeerId> = Vec::with_capacity(args.test_instance_count);
    let mut addresses: Vec<Multiaddr> = Vec::with_capacity(args.test_instance_count);

    sync_client.signal_entry(NET_STATE).await?;

    loop {
        tokio::select! {
            biased;
            // Pool futures in order. We want all the msg then check if barrier is down.
            Some(msg) = stream.next() => {
                let payload: String = msg.get_payload().unwrap();

                let parts: Vec<&str> = payload.split(' ').collect();

                let peer_id: PeerId = parts[0].parse().unwrap();
                let multi_addr: Multiaddr = parts[1].parse().unwrap();

                peer_ids.push(peer_id);
                addresses.push(multi_addr);
            },
            _ = sync_client.barrier(NET_STATE, args.test_instance_count) => break,
        }
    }

    // Add all other nodes to DHT.
    for (peer, addr) in peer_ids.into_iter().zip(addresses.into_iter()) {
        if peer == local_peer_id {
            continue;
        }

        ipfs.dht_add_addr(peer, addr).await;
    }

    // Wait for all node to add other nodes.
    if let Err(e) = sync_client
        .signal_and_wait("addrs_added", args.test_instance_count)
        .await
    {
        eprintln!("{}", e);
    }

    if let Err(e) = ipfs.dht_bootstrap().await {
        eprintln!("{:?}", e);
    }

    if let Err(e) = sync_client
        .signal_and_wait("bootstrapped", args.test_instance_count)
        .await
    {
        eprintln!("{}", e);
    }

    if sim_id == 1 {
        publisher(&ipfs).await;
    } else {
        subscribers(&ipfs).await;
    }

    if let Err(e) = sync_client
        .signal_and_wait("stopping", args.test_instance_count)
        .await
    {
        eprintln!("{}", e);
    }

    handle.abort();

    Ok(())
}

async fn subscribers(ipfs: &IpfsClient) {
    let sleep = time::sleep(Duration::from_secs(120));
    tokio::pin!(sleep);

    let mut stream = match ipfs.subscribe("live").await {
        Ok(stream) => stream,
        Err(e) => {
            eprintln!("{:?}", e);
            return;
        }
    };

    loop {
        tokio::select! {
            Some(msg) = stream.next() => {
                println!("{:?}", msg);
            },
            _ = &mut sleep => break,
        }
    }
}

async fn publisher(ipfs: &IpfsClient) {
    let sleep = time::sleep(Duration::from_secs(120));
    tokio::pin!(sleep);

    loop {
        tokio::select! {
            _ = async {
                loop {
                    time::sleep(Duration::from_secs(2)).await;

                    if let Err(e) = ipfs.publish("live", b"Hello!".to_vec()).await {
                        eprintln!("{:?}", e);
                    }
                }
            } => {},
            _ = &mut sleep => break,
        }
    }
}
