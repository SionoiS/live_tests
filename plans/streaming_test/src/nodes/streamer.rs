use std::time::Duration;

use rand::{Rng, SeedableRng};
use rand_xoshiro::Xoshiro256StarStar;
use tokio_stream::StreamExt;

use crate::{
    ipfs::*, synchronization::*, utils::*, Arguments, Result, BOOTSTRAP_STATE, GOSSIPSUB_TOPIC,
    INIT_STATE, NET_STATE, REDIS_TOPIC, SIM_TIME, STOP_STATE,
};

use tokio::{task::JoinHandle, time};

use libp2p::{Multiaddr, PeerId};

pub async fn streamer(
    ipfs: IpfsClient,
    sync_client: SyncClient,
    handle: JoinHandle<()>,
    args: Arguments,
    local_peer_id: PeerId,
    local_multi_addr: Multiaddr,
) -> Result<()> {
    if let Some(params) = args.test_instance_params {
        println!("Params: {}", params);
    }

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

    sync_client.signal_entry(NET_STATE).await?;

    let mut peer_ids: Vec<PeerId> = Vec::with_capacity(args.test_instance_count);
    let mut addresses: Vec<Multiaddr> = Vec::with_capacity(args.test_instance_count);

    let neutral_nodes = (args.test_instance_count / 4) - 1;

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
            _ = sync_client.barrier(BOOTSTRAP_STATE, neutral_nodes) => break,
        }
    }

    let mut rng = Xoshiro256StarStar::seed_from_u64(5634653465365u64);

    let mut rand_i = rng.gen_range(1..peer_ids.len());

    // Don't connect to yourself
    while peer_ids[rand_i] == local_peer_id {
        rand_i = rng.gen_range(1..peer_ids.len());
    }

    println!(
        "Streamer node {} connecting to: {}, {}",
        local_peer_id, peer_ids[rand_i], addresses[rand_i]
    );

    ipfs.dht_add_addr(peer_ids[rand_i], addresses[rand_i].clone())
        .await;

    if let Err(e) = ipfs.dht_bootstrap().await {
        eprintln!("{:?}", e);
    }

    let sleep = time::sleep(Duration::from_secs(SIM_TIME));
    tokio::pin!(sleep);

    loop {
        tokio::select! {
            _ = async {
                loop {
                    let block = get_random_block(&mut rng);

                    let cid_bytes = block.cid().to_bytes();

                    time::sleep(Duration::from_secs(1)).await;

                    ipfs.add_block(block).await;

                    if let Err(e) = ipfs.publish(GOSSIPSUB_TOPIC, cid_bytes).await {
                        eprintln!("{:?}", e);
                    }
                }
            } => {},
            _ = &mut sleep => break,
        }
    }

    // Wait for all node to stop.
    sync_client
        .signal_and_wait(STOP_STATE, args.test_instance_count)
        .await?;

    handle.abort();

    Ok(())
}
