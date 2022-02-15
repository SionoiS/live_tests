mod background;
mod block_exchange;
mod client;
mod composition;

use background::IpfsBackgroundService;
use composition::*;

pub use client::IpfsClient;

use libp2p::{
    gossipsub::{
        subscription_filter::AllowAllSubscriptionFilter, Gossipsub, GossipsubConfig,
        IdentityTransform, MessageAuthenticity,
    },
    identity,
    kad::{store::MemoryStore, Kademlia},
    ping::{Ping, PingConfig},
    swarm::SwarmBuilder,
    PeerId,
};

use ipfs_bitswap::Bitswap;

/// Returns an Ipfs Client, a background service handle and the local peer id.
pub fn compose_network(log: bool, max_concurrent_send: usize) -> (IpfsClient, PeerId) {
    let local_key = identity::Keypair::generate_ed25519();
    let local_peer_id = PeerId::from(local_key.clone().public());

    let transport =
        libp2p::tokio_development_transport(local_key.clone()).expect("Tokio Transport");

    let ping = Ping::new(PingConfig::new().with_keep_alive(true));

    let kademlia = Kademlia::new(local_peer_id, MemoryStore::new(local_peer_id));
    let gossipsub = Gossipsub::<IdentityTransform, AllowAllSubscriptionFilter>::new(
        MessageAuthenticity::Signed(local_key),
        GossipsubConfig::default(),
    )
    .expect("Gossipsub");

    let bitswap = Bitswap::default();

    let behaviour = ComposedBehaviour {
        ping,
        kademlia,
        gossipsub,
        bitswap,
    };

    let swarm = SwarmBuilder::new(transport, behaviour, local_peer_id)
        .executor(Box::new(|f| {
            tokio::spawn(f);
        }))
        .build();

    let (ipfs_client, cmd_rx) = IpfsClient::new();
    let ipfs_service = IpfsBackgroundService::new(cmd_rx, swarm, log, max_concurrent_send);

    tokio::spawn(ipfs_service.run());

    (ipfs_client, local_peer_id)
}
