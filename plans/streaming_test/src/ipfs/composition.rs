use ipfs_bitswap::{Bitswap, BitswapEvent};
use libp2p::{
    gossipsub::{Gossipsub, GossipsubEvent},
    kad::{store::MemoryStore, Kademlia, KademliaEvent},
    ping::{Ping, PingEvent},
};

#[derive(libp2p::NetworkBehaviour)]
#[behaviour(out_event = "ComposedEvent")]
pub struct ComposedBehaviour {
    pub bitswap: Bitswap,

    pub gossipsub: Gossipsub,

    pub kademlia: Kademlia<MemoryStore>,

    pub ping: Ping,
}

#[derive(Debug)]
pub enum ComposedEvent {
    Ping(PingEvent),

    Kademlia(KademliaEvent),

    Gossipsub(GossipsubEvent),

    Bitswap(BitswapEvent),
}

impl From<PingEvent> for ComposedEvent {
    fn from(event: PingEvent) -> Self {
        ComposedEvent::Ping(event)
    }
}

impl From<KademliaEvent> for ComposedEvent {
    fn from(event: KademliaEvent) -> Self {
        ComposedEvent::Kademlia(event)
    }
}

impl From<GossipsubEvent> for ComposedEvent {
    fn from(event: GossipsubEvent) -> Self {
        ComposedEvent::Gossipsub(event)
    }
}

impl From<BitswapEvent> for ComposedEvent {
    fn from(event: BitswapEvent) -> Self {
        ComposedEvent::Bitswap(event)
    }
}
