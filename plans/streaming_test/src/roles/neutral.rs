use crate::{
    ipfs, utils::data_network_ip, Result, BOOTSTRAP_STATE, INIT_STATE, NET_STATE, REDIS_TOPIC,
    STOP_STATE,
};

use futures_util::stream::StreamExt;

use libp2p::{multiaddr::Protocol, Multiaddr, PeerId};

use rand::{Rng, SeedableRng};
use rand_xoshiro::Xoshiro256StarStar;
use testground::{
    client::Client,
    network_conf::{
        FilterAction, LinkShape, NetworkConfiguration, RoutingPolicyType, DEAFULT_DATA_NETWORK,
    },
    RunParameters,
};

#[allow(unused)]
pub async fn neutral(sim_id: u64, testground: Client, runenv: RunParameters) -> Result<()> {
    let local_multi_addr = {
        let local_ip = data_network_ip(runenv.test_sidecar, runenv.test_subnet);

        let mut addr: Multiaddr = local_ip.into();

        addr.push(Protocol::Tcp(4001));

        addr
    };

    let (ipfs, local_peer_id) = ipfs::compose_network(false, 2);

    ipfs.listen_on(local_multi_addr.clone()).await?;

    let config = NetworkConfiguration {
        network: DEAFULT_DATA_NETWORK.to_owned(),
        ipv4: None,
        ipv6: None,
        enable: true,
        default: LinkShape {
            latency: 0_000_000,    // nanoseconds
            jitter: 0_000_000,     // nanoseconds
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
        "Neutral Sim ID: {} Peer: {} Addr: {}",
        sim_id, local_peer_id, local_multi_addr
    ); */

    let mut stream = testground.subscribe(REDIS_TOPIC).await;

    testground
        .barrier(INIT_STATE, runenv.test_instance_count)
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

    let neutral_nodes = (runenv.test_instance_count / 4) - 1;

    //Wait for neutral nodes peer_id & multi_addr
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

        if peer_ids.len() >= neutral_nodes as usize {
            break;
        }
    }

    testground.signal_and_wait(NET_STATE, neutral_nodes).await?;

    // Connect to a random peer
    let mut rng = Xoshiro256StarStar::seed_from_u64(sim_id);
    let rand_i = rng.gen_range(0..peer_ids.len());

    /* println!(
        "Neutral node {} connecting to {}, {}",
        local_peer_id, peer_ids[rand_i], addresses[rand_i]
    ); */

    ipfs.dht_add_addr(peer_ids[rand_i], addresses[rand_i].clone())
        .await;

    if let Err(e) = ipfs.dht_bootstrap().await {
        eprintln!("Bootstrap Error: {:?}", e);
    }

    testground
        .signal_and_wait(BOOTSTRAP_STATE, runenv.test_instance_count)
        .await?;

    // Wait for all node to stop.
    testground
        .signal_and_wait(STOP_STATE, runenv.test_instance_count)
        .await?;

    Ok(())
}
