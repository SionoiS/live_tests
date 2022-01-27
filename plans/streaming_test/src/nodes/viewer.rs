use std::time::Duration;

use crate::{
    ipfs::*, synchronization::*, Arguments, Result, BOOTSTRAP_STATE, GOSSIPSUB_TOPIC, INIT_STATE,
    REDIS_TOPIC, SIM_TIME, STOP_STATE,
};

use cid::Cid;

use futures_util::stream::StreamExt;

use rand::{Rng, SeedableRng};
use rand_xoshiro::Xoshiro256StarStar;
use tokio::{task::JoinHandle, time};

use libp2p::{Multiaddr, PeerId};

pub async fn viewer(
    ipfs: IpfsClient,
    sync_client: SyncClient,
    handle: JoinHandle<()>,
    args: Arguments,
    local_peer_id: PeerId,
    _local_multi_addr: Multiaddr,
) -> Result<()> {
    let mut stream = sync_client.subscribe(REDIS_TOPIC).await?;

    // Barrier waiting for every container to initialize and subscribe.
    sync_client
        .barrier(INIT_STATE, args.test_instance_count)
        .await?;

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

    let mut rng = Xoshiro256StarStar::seed_from_u64(78234890243789u64);

    let rand_i = rng.gen_range(1..peer_ids.len());

    println!(
        "Viewer node {} connecting to: {}, {}",
        local_peer_id, peer_ids[rand_i], addresses[rand_i]
    );

    ipfs.dht_add_addr(peer_ids[rand_i], addresses[rand_i].clone())
        .await;

    if let Err(e) = ipfs.dht_bootstrap().await {
        eprintln!("{:?}", e);
    }

    subscribers(&ipfs).await;

    // Wait for all node to stop.
    sync_client
        .signal_and_wait(STOP_STATE, args.test_instance_count)
        .await?;

    handle.abort();

    Ok(())
}

async fn subscribers(ipfs: &IpfsClient) {
    let sleep = time::sleep(Duration::from_secs(SIM_TIME));
    tokio::pin!(sleep);

    let mut stream = match ipfs.subscribe(GOSSIPSUB_TOPIC).await {
        Ok(stream) => stream,
        Err(e) => {
            eprintln!("{:?}", e);
            return;
        }
    };

    loop {
        tokio::select! {
            Some(msg) = stream.next() => {
                let cid = Cid::try_from(msg.data).unwrap();

                let block = ipfs.get_block(cid).await;

                println!("Received Block -> {}", block.cid());
            },
            _ = &mut sleep => break,
        }
    }
}
