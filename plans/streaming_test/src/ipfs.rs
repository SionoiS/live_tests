#![allow(dead_code)]
#![allow(unused_variables)]

use std::{collections::HashMap, io};

use cid::Cid;

use futures_util::Stream;

use ipfs_bitswap::{BitswapEvent, Block};

use libp2p::{
    core::{connection::ListenerId, either::EitherError},
    gossipsub::{
        error::{GossipsubHandlerError, PublishError, SubscriptionError},
        GossipsubEvent, GossipsubMessage, IdentTopic, MessageId, TopicHash,
    },
    kad::{BootstrapError, BootstrapOk, KademliaEvent, QueryId, QueryResult},
    swarm::{ProtocolsHandlerUpgrErr, SwarmEvent},
    Multiaddr, PeerId, Swarm, TransportError,
};

use tokio::sync::{mpsc, oneshot};
use tokio_stream::{wrappers::ReceiverStream, StreamExt};

use crate::composition::{ComposedBehaviour, ComposedEvent};

#[derive(Debug)]
pub enum Command {
    Publish {
        topic: String,
        data: Vec<u8>,
        sender: oneshot::Sender<Result<MessageId, PublishError>>,
    },

    Subscribe {
        topic: String,
        sender: oneshot::Sender<Result<bool, SubscriptionError>>,
        stream: mpsc::Sender<GossipsubMessage>,
    },

    DhtAdd {
        peer_id: PeerId,
        multi_addr: Multiaddr,
        sender: oneshot::Sender<()>,
    },

    Bootstrap {
        sender: oneshot::Sender<Result<BootstrapOk, BootstrapError>>,
    },

    AddPeers {
        peer_id: PeerId,
    },

    ListenOn {
        multi_addr: Multiaddr,
        sender: oneshot::Sender<Result<Multiaddr, TransportError<io::Error>>>,
    },
}

#[derive(Clone)]
pub struct IpfsClient {
    cmd_tx: mpsc::Sender<Command>,
}

impl IpfsClient {
    pub fn new() -> (Self, mpsc::Receiver<Command>) {
        let (cmd_tx, cmd_rx) = mpsc::channel(1);

        (Self { cmd_tx }, cmd_rx)
    }

    pub async fn publish(&self, topic: &str, data: Vec<u8>) -> Result<MessageId, PublishError> {
        let (sender, receiver) = oneshot::channel();

        let topic = topic.to_owned();
        let cmd = Command::Publish {
            topic,
            data,
            sender,
        };

        self.cmd_tx.send(cmd).await.expect("Receiver not dropped");

        receiver.await.expect("Sender not dropped")
    }

    /// Subscribe to a topic.
    ///
    /// Only the first subscribe on a topic will yield messages.
    pub async fn subscribe(
        &self,
        topic: &str,
    ) -> Result<impl Stream<Item = GossipsubMessage>, SubscriptionError> {
        let (stream, mut receiver) = mpsc::channel(10);
        let (sender, err_rx) = oneshot::channel();

        let topic = topic.to_owned();
        let cmd = Command::Subscribe {
            topic,
            sender,
            stream,
        };

        self.cmd_tx.send(cmd).await.expect("Receiver not dropped");

        match err_rx.await.expect("Sender not dropped") {
            Ok(status) => {
                if !status {
                    receiver.close();
                }
            }
            Err(e) => return Err(e),
        }

        Ok(ReceiverStream::new(receiver))
    }

    pub async fn dht_add_addr(&self, peer_id: PeerId, multi_addr: Multiaddr) {
        let (sender, receiver) = oneshot::channel();

        let cmd = Command::DhtAdd {
            peer_id,
            multi_addr,
            sender,
        };

        self.cmd_tx.send(cmd).await.expect("Receiver not dropped");

        receiver.await.expect("Sender not dropped")
    }

    pub async fn dht_bootstrap(&self) -> Result<BootstrapOk, BootstrapError> {
        let (sender, receiver) = oneshot::channel();

        let cmd = Command::Bootstrap { sender };

        self.cmd_tx.send(cmd).await.expect("Receiver not dropped");

        receiver.await.expect("Sender not dropped")
    }

    pub async fn gossipsub_add_peer(&self, peer_id: PeerId) {
        let cmd = Command::AddPeers { peer_id };

        self.cmd_tx.send(cmd).await.expect("Receiver not dropped");
    }

    pub async fn listen_on(
        &self,
        multi_addr: Multiaddr,
    ) -> Result<Multiaddr, TransportError<io::Error>> {
        let (sender, receiver) = oneshot::channel();

        let cmd = Command::ListenOn { multi_addr, sender };

        self.cmd_tx.send(cmd).await.expect("Receiver not dropped");

        receiver.await.expect("Sender not dropped")
    }
}

pub struct IpfsBackgroundService {
    cmd_rx: ReceiverStream<Command>,
    swarm: Swarm<ComposedBehaviour>,

    pending_subscribe: HashMap<TopicHash, mpsc::Sender<GossipsubMessage>>,
    routing_updates: HashMap<PeerId, oneshot::Sender<()>>,
    pending_bootstap: HashMap<QueryId, oneshot::Sender<Result<BootstrapOk, BootstrapError>>>,
    pending_listen:
        HashMap<ListenerId, oneshot::Sender<Result<Multiaddr, TransportError<io::Error>>>>,
    block_exchange: BlockExchange,
}

impl IpfsBackgroundService {
    pub fn new(cmd_rx: mpsc::Receiver<Command>, swarm: Swarm<ComposedBehaviour>) -> Self {
        Self {
            cmd_rx: ReceiverStream::new(cmd_rx),
            swarm,

            pending_subscribe: Default::default(),
            routing_updates: Default::default(),
            pending_bootstap: Default::default(),
            pending_listen: Default::default(),
            block_exchange: Default::default(),
        }
    }

    pub async fn run(&mut self) {
        loop {
            tokio::select! {
                event = self.swarm.next() => self.handle_event(event).await,
                cmd = self.cmd_rx.next() => match cmd {
                    Some(cmd) =>self.handle_command(cmd).await,
                    None => return,
                }
            }
        }
    }

    async fn handle_command(&mut self, cmd: Command) {
        match cmd {
            Command::Publish {
                topic,
                data,
                sender,
            } => {
                let topic = IdentTopic::new(topic);

                let res = self.swarm.behaviour_mut().gossipsub.publish(topic, data);

                if sender.send(res).is_err() {
                    eprintln!("Receiver hung up");
                }
            }
            Command::Subscribe {
                topic,
                sender,
                stream,
            } => {
                let topic = IdentTopic::new(topic);

                let status = match self.swarm.behaviour_mut().gossipsub.subscribe(&topic) {
                    Ok(status) => status,
                    Err(e) => {
                        sender.send(Err(e)).expect("Receiver not dropped");
                        return;
                    }
                };

                sender.send(Ok(status)).expect("Receiver not dropped");

                if !status {
                    return;
                }

                self.pending_subscribe.insert(topic.hash(), stream);
            }
            Command::DhtAdd {
                peer_id,
                multi_addr,
                sender,
            } => {
                self.swarm
                    .behaviour_mut()
                    .kademlia
                    .add_address(&peer_id, multi_addr);

                self.routing_updates.insert(peer_id, sender);
            }
            Command::Bootstrap { sender } => {
                let id = match self.swarm.behaviour_mut().kademlia.bootstrap() {
                    Ok(id) => id,
                    Err(e) => {
                        eprintln!("{:?}", e);
                        return;
                    }
                };

                self.pending_bootstap.insert(id, sender);
            }
            Command::AddPeers { peer_id } => {
                self.swarm
                    .behaviour_mut()
                    .gossipsub
                    .add_explicit_peer(&peer_id);
            }
            Command::ListenOn { multi_addr, sender } => {
                let id = match self.swarm.listen_on(multi_addr) {
                    Ok(id) => id,
                    Err(e) => {
                        sender.send(Err(e)).expect("Receiver not dropped");
                        return;
                    }
                };

                self.pending_listen.insert(id, sender);
            }
        }
    }

    async fn handle_event(
        &mut self,
        event: Option<
            SwarmEvent<
                ComposedEvent,
                EitherError<
                    EitherError<std::io::Error, GossipsubHandlerError>,
                    ProtocolsHandlerUpgrErr<std::io::Error>,
                >,
            >,
        >,
    ) {
        let event = event.expect("Swarm stream is infinite");

        match event {
            SwarmEvent::Behaviour(ComposedEvent::Kademlia(event)) => {
                self.kademlia_event(event).await
            }
            SwarmEvent::Behaviour(ComposedEvent::Gossipsub(event)) => {
                self.gossipsub_event(event).await
            }
            SwarmEvent::Behaviour(ComposedEvent::Bitswap(event)) => self.bitswap_event(event).await,
            SwarmEvent::ConnectionEstablished {
                peer_id,
                endpoint,
                num_established,
                concurrent_dial_errors,
            } => {}
            SwarmEvent::ConnectionClosed {
                peer_id,
                endpoint,
                num_established,
                cause,
            } => {}
            SwarmEvent::IncomingConnection {
                local_addr,
                send_back_addr,
            } => {}
            SwarmEvent::IncomingConnectionError {
                local_addr,
                send_back_addr,
                error,
            } => {}
            SwarmEvent::OutgoingConnectionError { peer_id, error } => {}
            SwarmEvent::BannedPeer { peer_id, endpoint } => {}
            SwarmEvent::NewListenAddr {
                listener_id,
                address,
            } => {
                if let Some(sender) = self.pending_listen.remove(&listener_id) {
                    sender.send(Ok(address)).expect("Receiver not dropped");
                }
            }
            SwarmEvent::ExpiredListenAddr {
                listener_id,
                address,
            } => {}
            SwarmEvent::ListenerClosed {
                listener_id,
                addresses,
                reason,
            } => {}
            SwarmEvent::ListenerError { listener_id, error } => {}
            SwarmEvent::Dialing(_) => {}
        }
    }

    async fn gossipsub_event(&mut self, event: GossipsubEvent) {
        match event {
            GossipsubEvent::Message {
                propagation_source: _,
                message_id: _,
                message,
            } => {
                let sender = match self.pending_subscribe.get(&message.topic) {
                    Some(sender) => sender,
                    None => return,
                };

                if sender.send(message).await.is_err() {
                    eprintln!("Receiver dropped");
                }
            }
            GossipsubEvent::Subscribed { peer_id, topic } => {}
            GossipsubEvent::Unsubscribed { peer_id, topic } => {}
            GossipsubEvent::GossipsubNotSupported { peer_id } => {}
        }
    }

    async fn kademlia_event(&mut self, event: KademliaEvent) {
        match event {
            KademliaEvent::InboundRequest { request } => {}
            KademliaEvent::OutboundQueryCompleted { id, result, stats } => {
                if let QueryResult::Bootstrap(result) = result {
                    match result {
                        Ok(bootstrap_ok) => {
                            if bootstrap_ok.num_remaining == 0 {
                                match self.pending_bootstap.remove(&id) {
                                    Some(sender) => {
                                        sender.send(Ok(bootstrap_ok)).expect("Receiver not dropped")
                                    }
                                    None => return,
                                }
                            }
                        }
                        Err(e) => match self.pending_bootstap.remove(&id) {
                            Some(sender) => sender.send(Err(e)).expect("Receiver not dropped"),
                            None => return,
                        },
                    }
                }
            }
            KademliaEvent::RoutingUpdated {
                peer,
                is_new_peer,
                addresses: _,
                bucket_range,
                old_peer,
            } => {
                let sender = match self.routing_updates.remove(&peer) {
                    Some(sender) => sender,
                    None => return,
                };

                if sender.send(()).is_err() {
                    eprintln!("Receiver dropped");
                }
            }
            KademliaEvent::UnroutablePeer { peer } => {}
            KademliaEvent::RoutablePeer { peer, address } => {}
            KademliaEvent::PendingRoutablePeer { peer, address } => {}
        }
    }

    async fn bitswap_event(&mut self, event: BitswapEvent) {
        match event {
            BitswapEvent::ReceivedBlock(_, block) => {
                self.block_exchange.add_block(block);

                //TODO check if some peer want this block
            }
            BitswapEvent::ReceivedWant(peer_id, cid, priority) => {
                let block = match self.block_exchange.get_block(&cid) {
                    Some(b) => b,
                    None => {
                        //TODO store the want list.
                        return;
                    }
                };

                self.swarm
                    .behaviour_mut()
                    .bitswap
                    .send_block(peer_id, block);
            }
            BitswapEvent::ReceivedCancel(peer_id, cid) => {
                //TODO update the want list.
            }
        }
    }
}

#[derive(Default)]
pub struct BlockExchange {
    peers_want: Vec<PeerId>,

    peer_indices: Vec<usize>, // sync
    cid_indices: Vec<usize>,  // sync

    // First Cids with data then starting at data_store.len() Cids without data
    cids: Vec<Cid>,
    data_store: Vec<Box<[u8]>>,
}

impl BlockExchange {
    pub fn peers_want_block(&self, cid: Cid) -> &[PeerId] {
        //find index of cid in cids
        //find all the occurances in cid_indices
        //find all the peers in peer_indices

        todo!()
    }

    pub fn get_block(&self, cid: &Cid) -> Option<Block> {
        for (id, data) in self.cids.iter().zip(self.data_store.iter()) {
            if cid != id {
                continue;
            }

            return Some(Block::new(data.clone(), id.clone()));
        }

        None
    }

    pub fn add_block(&mut self, block: Block) -> bool {
        let mut index = None;

        for (i, cid) in self.cids.iter().enumerate() {
            if cid != block.cid() {
                continue;
            }

            if i < self.data_store.len() {
                return false;
            }

            // Case where cid is present but no data
            index = Some(i);
        }

        let Block { cid, data } = block;

        match index {
            Some(i) => {
                self.data_store[i] = data;
            }
            None => {
                self.cids.push(cid);
                self.data_store.push(data);
            }
        }

        true
    }
}
