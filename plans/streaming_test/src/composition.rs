use ipfs_bitswap::{Bitswap, BitswapEvent};
use libp2p::{
    gossipsub::{Gossipsub, GossipsubEvent},
    kad::{store::MemoryStore, Kademlia, KademliaEvent},
};

#[derive(libp2p::NetworkBehaviour)]
#[behaviour(out_event = "ComposedEvent")]
pub struct ComposedBehaviour {
    pub kademlia: Kademlia<MemoryStore>,

    pub gossipsub: Gossipsub,

    pub bitswap: Bitswap,
}

#[derive(Debug)]
pub enum ComposedEvent {
    Kademlia(KademliaEvent),

    Gossipsub(GossipsubEvent),

    Bitswap(BitswapEvent),
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
