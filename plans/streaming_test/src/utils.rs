use std::net::{IpAddr, Ipv4Addr};

use ipfs_bitswap::Bitswap;
use libp2p::{
    gossipsub::{
        subscription_filter::AllowAllSubscriptionFilter, Gossipsub, GossipsubConfig,
        IdentityTransform, MessageAuthenticity,
    },
    identity,
    kad::{store::MemoryStore, Kademlia},
    swarm::SwarmBuilder,
    PeerId,
};
use pnet::{datalink, ipnetwork::IpNetwork};

use tokio::task::JoinHandle;

use crate::{
    composition::ComposedBehaviour,
    ipfs::{IpfsBackgroundService, IpfsClient},
};

/// Returns an Ipfs Client, a background service handle and the local peer id.
pub fn compose_network() -> (IpfsClient, JoinHandle<()>, PeerId) {
    let local_key = identity::Keypair::generate_ed25519();
    let local_peer_id = PeerId::from(local_key.clone().public());

    let transport =
        libp2p::tokio_development_transport(local_key.clone()).expect("Tokio Transport");

    let kademlia = Kademlia::new(local_peer_id, MemoryStore::new(local_peer_id));
    let gossipsub = Gossipsub::<IdentityTransform, AllowAllSubscriptionFilter>::new(
        MessageAuthenticity::Signed(local_key),
        GossipsubConfig::default(),
    )
    .expect("Gossipsub");

    let bitswap = Bitswap::default();

    let behaviour = ComposedBehaviour {
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
    let mut ipfs_service = IpfsBackgroundService::new(cmd_rx, swarm);

    let handle = tokio::spawn(async move {
        ipfs_service.run().await;
    });

    (ipfs_client, handle, local_peer_id)
}

/// Get the IP address of this container on the data network
pub fn data_network_ip(sidecar: bool, subnet: IpNetwork) -> IpAddr {
    if !sidecar {
        return IpAddr::V4(Ipv4Addr::LOCALHOST);
    }

    let all_interfaces = datalink::interfaces();

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
