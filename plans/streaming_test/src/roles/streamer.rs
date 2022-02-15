use std::time::Duration;

use chrono::Utc;
use rand::{RngCore, SeedableRng};
use rand_xoshiro::Xoshiro256StarStar;
use tokio_stream::StreamExt;

use crate::{
    ipfs::{self, IpfsClient},
    utils::*,
    Result, BOOTSTRAP_STATE, GOSSIPSUB_TOPIC, INIT_STATE, NET_STATE, REDIS_TOPIC, STOP_STATE,
};

use tokio::time;

use libp2p::{multiaddr::Protocol, Multiaddr, PeerId};

use testground::{
    client::Client,
    network_conf::{
        FilterAction, LinkShape, NetworkConfiguration, RoutingPolicyType, DEAFULT_DATA_NETWORK,
    },
    RunParameters,
};

pub async fn streamer(sim_id: usize, testground: Client, runenv: RunParameters) -> Result<()> {
    let local_multi_addr = {
        let local_ip = data_network_ip(runenv.test_sidecar, runenv.test_subnet);

        let mut addr: Multiaddr = local_ip.into();

        addr.push(Protocol::Tcp(4001));

        addr
    };

    let test_case_params = custom_instance_params(&runenv.test_instance_params);
    let log = test_case_params.log && sim_id == test_case_params.log_id;

    let (ipfs, local_peer_id) = ipfs::compose_network(log, test_case_params.max_concurrent_send);

    ipfs.listen_on(local_multi_addr.clone()).await?;

    let config = NetworkConfiguration {
        network: DEAFULT_DATA_NETWORK.to_owned(),
        ipv4: None,
        ipv6: None,
        enable: true,
        default: LinkShape {
            latency: 50_000_000,   // nanoseconds
            jitter: 1_000_000,     // nanoseconds
            bandwidth: 10_000_000, // bits per seconds
            filter: FilterAction::Accept,
            loss: 0.0,
            corrupt: 0.0,
            corrupt_corr: 0.0,
            reorder: 0.0,
            reorder_corr: 0.0,
            duplicate: 0.0,
            duplicate_corr: 0.0,
        },
        rules: None,
        callback_state: "network-shaping".to_owned(),
        callback_target: None,
        routing_policy: RoutingPolicyType::AllowAll,
    };

    testground.configure_network(config).await?;

    /* println!(
        "Streamer Sim ID: {} Peer: {} Addr: {}",
        sim_id, local_peer_id, local_multi_addr
    ); */

    let mut stream = testground.subscribe(REDIS_TOPIC).await;

    testground
        .signal_and_wait(INIT_STATE, runenv.test_instance_count)
        .await?;

    let msg = format!(
        "{} {}",
        local_peer_id.to_base58(),
        local_multi_addr.to_string()
    );

    // Send PeerId & MultiAddress to all other nodes.
    testground.publish(REDIS_TOPIC, msg).await?;

    let mut peer_ids: Vec<PeerId> = Vec::with_capacity(runenv.test_instance_count as usize);
    let mut addresses: Vec<Multiaddr> = Vec::with_capacity(runenv.test_instance_count as usize);

    //let neutral_nodes = (runenv.test_instance_count / 4) - 1;

    testground.signal(NET_STATE).await?;

    while let Some(msg) = stream.next().await {
        let payload = match msg {
            Ok(p) => p,
            Err(e) => {
                eprintln!("Redis PubSub Error: {:?}", e);
                continue;
            }
        };

        let parts: Vec<&str> = payload.split(' ').collect();

        let peer_id: PeerId = parts[0].parse().unwrap();
        let multi_addr: Multiaddr = parts[1].parse().unwrap();

        if peer_id == local_peer_id {
            continue;
        }

        peer_ids.push(peer_id);
        addresses.push(multi_addr);

        if peer_ids.len() >= (runenv.test_instance_count - 1) as usize {
            break;
        }
    }

    testground
        .barrier(NET_STATE, runenv.test_instance_count)
        .await?;

    // Connect to one neutral node
    let mut rng = Xoshiro256StarStar::seed_from_u64(sim_id as u64);

    for (_peer, addr) in peer_ids.into_iter().zip(addresses.into_iter()) {
        //let rand_i = rng.gen_range(0..peer_ids.len());

        /* println!(
            "Streamer node {} connecting to: {}, {}",
            local_peer_id, peer, addr
        ); */

        //ipfs.dht_add_addr(peer, addr).await;
        ipfs.dial(addr);
    }

    /* if let Err(e) = ipfs.dht_bootstrap().await {
        eprintln!("Bootstrap Error: {:?}", e);
    } */

    testground
        .signal_and_wait(BOOTSTRAP_STATE, runenv.test_instance_count)
        .await?;

    let sleep = time::sleep(Duration::from_secs(test_case_params.sim_time as u64));
    tokio::pin!(sleep);

    let mut count = 0;

    loop {
        tokio::select! {
            biased;
            _ = &mut sleep => break,

            _ = generate_video(&ipfs, &mut rng, &mut count, test_case_params.segment_length, test_case_params.max_size_block) => {},
        }
    }

    // Wait for all node to stop.
    testground
        .signal_and_wait(STOP_STATE, runenv.test_instance_count)
        .await?;

    Ok(())
}

async fn generate_video(
    ipfs: &IpfsClient,
    rng: &mut impl RngCore,
    counter: &mut u64,
    segment_length: usize,
    max_size_block: bool,
) {
    loop {
        // 1s of 1080p60 video would be ~3 standard blocks
        // 1s of 720p30 would be ~1 standard block

        let count = *counter;
        *counter += 1;

        let cids = generate_blocks(ipfs, rng, 3, max_size_block);

        // Add 1s now because the msg will be sent 1s later.
        let timestamp = Utc::now().timestamp_millis() + (segment_length as i64 * 1000);

        let msg = StreamerMessage {
            count,
            timestamp,
            cids,
        };

        let msg = serde_json::to_vec(&msg).expect("Message Serialization");

        time::sleep(Duration::from_secs(segment_length as u64)).await;

        if let Err(e) = ipfs.publish(GOSSIPSUB_TOPIC, msg).await {
            eprintln!("GossipSub Error: {:?}", e);
        }
    }
}
