use crate::{ipfs::*, Arguments, Result, INIT_STATE, REDIS_TOPIC, STOP_STATE};

use futures_util::stream::StreamExt;

use tokio::task::JoinHandle;

use libp2p::{Multiaddr, PeerId};

use testground::client::Client;

pub async fn neutral(
    sim_id: u64,
    ipfs: IpfsClient,
    sync_client: Client,
    handle: JoinHandle<()>,
    args: Arguments,
    local_peer_id: PeerId,
    local_multi_addr: Multiaddr,
) -> Result<()> {
    println!(
        "Neutral Sim ID: {} Peer: {} Addr: {}",
        sim_id, local_peer_id, local_multi_addr
    );

    let mut stream = sync_client.subscribe(REDIS_TOPIC).await;

    // Barrier waiting for every container to initialize and subscribe.
    sync_client
        .wait_for_barrier(INIT_STATE, args.test_instance_count)
        .await?;

    //Wait for the streamer node peer_id & multi_addr
    if let Some(msg) = stream.next().await {
        let payload = match msg {
            Ok(p) => p,
            Err(e) => {
                eprintln!("Redis PubSub Error: {:?}", e);
                return Ok(());
            }
        };

        let parts: Vec<&str> = payload.split(' ').collect();

        let peer: PeerId = parts[0].parse().unwrap();
        let addr: Multiaddr = parts[1].parse().unwrap();

        println!(
            "Neutral node {} connecting to {}, {}",
            local_peer_id, peer, addr
        );

        ipfs.dht_add_addr(peer, addr).await;
    }

    if let Err(e) = ipfs.dht_bootstrap().await {
        eprintln!("{:?}", e);
    }

    let msg = format!(
        "{} {}",
        local_peer_id.to_base58(),
        local_multi_addr.to_string()
    );

    // Send PeerId & MultiAddress to all other nodes.
    sync_client.publish(REDIS_TOPIC, msg).await?;

    // Wait for all node to stop.
    sync_client
        .signal_and_wait(STOP_STATE, args.test_instance_count)
        .await?;

    handle.abort();

    Ok(())
}
