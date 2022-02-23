use crate::{
    ipfs::{self},
    utils::{custom_instance_params, data_network_ip},
    video_player::VideoStream,
    Result, BOOTSTRAP_STATE, INIT_STATE, NET_STATE, REDIS_TOPIC, STOP_STATE,
};

use futures_util::stream::StreamExt;

use libp2p::{multiaddr::Protocol, Multiaddr, PeerId};

use testground::{
    client::Client,
    network_conf::{
        FilterAction, LinkShape, NetworkConfiguration, RoutingPolicyType, DEAFULT_DATA_NETWORK,
    },
    RunParameters,
};
use tokio::sync::mpsc::unbounded_channel;

pub async fn viewer(sim_id: usize, testground: Client, runenv: RunParameters) -> Result<()> {
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
            latency: 50_000_000,                                  // nanoseconds
            jitter: 1_000_000,                                    // nanoseconds
            bandwidth: test_case_params.network_bandwidth as u64, // bits per seconds
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
        "Viewer Sim ID: {} Peer: {} Addr: {}",
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

    //let mut rng = Xoshiro256StarStar::seed_from_u64(sim_id);

    for (_peer, addr) in peer_ids.into_iter().zip(addresses.into_iter()) {
        //let rand_i = rng.gen_range(0..peer_ids.len());

        /* println!(
            "Viewer node {} connecting to {}, {}",
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

    let (block_sender, block_receiver) = unbounded_channel();

    let mut video_stream = VideoStream::new(
        ipfs,
        testground.clone(),
        test_case_params,
        sim_id as u64,
        block_sender,
    );

    video_stream.run(block_receiver).await;

    if let Err(e) = testground.record_success().await {
        eprintln!("Record Success Error: {:?}", e);
    }

    // Wait for all node to stop.
    testground
        .signal_and_wait(STOP_STATE, runenv.test_instance_count)
        .await?;

    Ok(())
}
