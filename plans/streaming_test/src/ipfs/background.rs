use std::{
    collections::{HashMap, HashSet},
    io::{self, Error},
};

use cid::Cid;
use ipfs_bitswap::{BitswapEvent, Block};

use libp2p::{
    core::{connection::ListenerId, either::EitherError},
    gossipsub::{
        error::GossipsubHandlerError, GossipsubEvent, GossipsubMessage, IdentTopic, TopicHash,
    },
    kad::{BootstrapError, BootstrapOk, KademliaEvent, QueryId, QueryResult},
    ping::Failure,
    swarm::{ProtocolsHandlerUpgrErr, SwarmEvent},
    Multiaddr, PeerId, Swarm, TransportError,
};

use tokio::sync::{mpsc, oneshot};
use tokio_stream::{wrappers::UnboundedReceiverStream, StreamExt};

use super::{
    block_exchange::BlockExchange,
    client::Command,
    composition::{ComposedBehaviour, ComposedEvent},
};

pub struct IpfsBackgroundService {
    cmd_rx: UnboundedReceiverStream<Command>,

    swarm: Swarm<ComposedBehaviour>,

    block_exchange: BlockExchange,

    pending_subscribe: HashMap<TopicHash, mpsc::UnboundedSender<GossipsubMessage>>,
    routing_updates: HashMap<PeerId, oneshot::Sender<()>>,
    pending_bootstap: HashMap<QueryId, oneshot::Sender<Result<BootstrapOk, BootstrapError>>>,
    pending_listen:
        HashMap<ListenerId, oneshot::Sender<Result<Multiaddr, TransportError<io::Error>>>>,
    pending_get_block: HashMap<Cid, oneshot::Sender<Result<Block, Error>>>,

    pending_send_block: HashSet<(PeerId, Cid)>,
    max_concurrent_send: usize,

    log: bool,
}

impl IpfsBackgroundService {
    pub fn new(
        cmd_rx: mpsc::UnboundedReceiver<Command>,
        swarm: Swarm<ComposedBehaviour>,
        log: bool,
        max_concurrent_send: usize,
    ) -> Self {
        Self {
            cmd_rx: UnboundedReceiverStream::new(cmd_rx),
            swarm,

            pending_subscribe: Default::default(),
            routing_updates: Default::default(),
            pending_bootstap: Default::default(),
            pending_listen: Default::default(),
            block_exchange: Default::default(),
            pending_get_block: Default::default(),
            pending_send_block: Default::default(),
            max_concurrent_send,

            log,
        }
    }

    pub async fn run(mut self) {
        loop {
            tokio::select! {
                event = self.swarm.next() => self.event(event).await,
                cmd = self.cmd_rx.next() => match cmd {
                    Some(cmd) =>self.command(cmd).await,
                    None => {
                        eprintln!("IPFS Client Dropped");
                        return;
                    },
                }
            }
        }
    }

    async fn command(&mut self, cmd: Command) {
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
                self.routing_updates.insert(peer_id, sender);

                self.swarm
                    .behaviour_mut()
                    .kademlia
                    .add_address(&peer_id, multi_addr);
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
            Command::BlockGet { cid, sender } => {
                if let Some(sender) = self.pending_get_block.insert(cid, sender) {
                    sender
                        .send(Err(Error::new(
                            io::ErrorKind::AlreadyExists,
                            "Already Getting Block",
                        )))
                        .expect("Receiver not dropped");
                }

                self.swarm.behaviour_mut().bitswap.want_block(cid, 0);
            }
            Command::BlockAdd { block } => {
                self.block_exchange.add_block(block);
            }
            Command::Dial { multi_addr } => {
                use libp2p::swarm::dial_opts::DialOpts;

                let _ = self.swarm.dial(DialOpts::from(multi_addr));
            }
        }
    }

    async fn event(
        &mut self,
        event: Option<
            SwarmEvent<
                ComposedEvent,
                EitherError<
                    EitherError<
                        EitherError<ProtocolsHandlerUpgrErr<Error>, GossipsubHandlerError>,
                        Error,
                    >,
                    Failure,
                >,
            >,
        >,
    ) {
        let event = event.expect("Infinite Swarm Stream");

        match event {
            SwarmEvent::Behaviour(ComposedEvent::Kademlia(event)) => self.kademlia(event).await,
            SwarmEvent::Behaviour(ComposedEvent::Gossipsub(event)) => self.gossipsub(event).await,
            SwarmEvent::Behaviour(ComposedEvent::Bitswap(event)) => self.bitswap(event).await,
            SwarmEvent::Behaviour(ComposedEvent::Ping(_event)) => {}
            SwarmEvent::ConnectionEstablished {
                peer_id: _,
                endpoint: _,
                num_established: _,
                concurrent_dial_errors: _,
            } => {}
            SwarmEvent::ConnectionClosed {
                peer_id: _,
                endpoint: _,
                num_established: _,
                cause: _,
            } => {}
            SwarmEvent::IncomingConnection {
                local_addr: _,
                send_back_addr: _,
            } => {}
            SwarmEvent::IncomingConnectionError {
                local_addr: _,
                send_back_addr: _,
                error: _,
            } => {}
            SwarmEvent::OutgoingConnectionError {
                peer_id: _,
                error: _,
            } => {}
            SwarmEvent::BannedPeer {
                peer_id: _,
                endpoint: _,
            } => {}
            SwarmEvent::NewListenAddr {
                listener_id,
                address,
            } => {
                if let Some(sender) = self.pending_listen.remove(&listener_id) {
                    sender.send(Ok(address)).expect("Receiver not dropped");
                }
            }
            SwarmEvent::ExpiredListenAddr {
                listener_id: _,
                address: _,
            } => {}
            SwarmEvent::ListenerClosed {
                listener_id: _,
                addresses: _,
                reason: _,
            } => {}
            SwarmEvent::ListenerError {
                listener_id: _,
                error: _,
            } => {}
            SwarmEvent::Dialing(_) => {}
        }
    }

    async fn gossipsub(&mut self, event: GossipsubEvent) {
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

                sender.send(message).expect("Receiver not dropped")
            }
            GossipsubEvent::Subscribed {
                peer_id: _,
                topic: _,
            } => {}
            GossipsubEvent::Unsubscribed {
                peer_id: _,
                topic: _,
            } => {}
            GossipsubEvent::GossipsubNotSupported { peer_id: _ } => {}
        }
    }

    async fn kademlia(&mut self, event: KademliaEvent) {
        match event {
            KademliaEvent::InboundRequest { request: _ } => {}
            KademliaEvent::OutboundQueryCompleted {
                id,
                result,
                stats: _,
            } => {
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
                is_new_peer: _,
                addresses: _,
                bucket_range: _,
                old_peer: _,
            } => {
                let sender = match self.routing_updates.remove(&peer) {
                    Some(sender) => sender,
                    None => return,
                };

                sender.send(()).expect("Receiver not dropped")
            }
            KademliaEvent::UnroutablePeer { peer: _ } => {}
            KademliaEvent::RoutablePeer {
                peer: _,
                address: _,
            } => {}
            KademliaEvent::PendingRoutablePeer {
                peer: _,
                address: _,
            } => {}
        }
    }

    async fn bitswap(&mut self, event: BitswapEvent) {
        // First come first served. Sending only couple blocks at a time.

        match event {
            BitswapEvent::ReceivedBlock(peer_id, block) => {
                self.block_exchange.add_block(block.clone());

                if self.log {
                    println!("Received Block -> Peer: {} Cid: {}", peer_id, block.cid());
                }

                if let Some(sender) = self.pending_get_block.remove(block.cid()) {
                    sender
                        .send(Ok(block.clone()))
                        .expect("Receiver not dropped");
                }

                if self.pending_send_block.len() >= self.max_concurrent_send {
                    // if already sending too many blocks
                    if self.log {
                        println!("Too Many Pending Send");
                    }

                    return;
                }

                for (peer, block) in self.block_exchange.iter_wanted_blocks() {
                    if self.log {
                        println!("Who Want Block -> Peer: {} Cid: {}", peer, block.cid());
                    }

                    if !self.pending_send_block.insert((peer, *block.cid())) {
                        //if already sending this peer this block
                        if self.log {
                            println!(
                                "Already Sending Block -> Peer: {} Cid: {}",
                                peer,
                                block.cid()
                            );
                        }

                        continue;
                    }

                    if self.log {
                        println!("Sending Block -> Peer: {} Cid: {}", peer, block.cid());
                    }

                    self.swarm.behaviour_mut().bitswap.send_block(peer, block);

                    break;
                }
            }
            BitswapEvent::ReceivedWant(peer_id, cid, _priority) => {
                self.block_exchange.add_want(peer_id, cid);

                if self.log {
                    println!("Received Want -> Peer: {} Cid: {}", peer_id, cid);
                }

                if self.pending_send_block.len() >= self.max_concurrent_send {
                    // if already sending too many blocks
                    if self.log {
                        println!("Too Many Pending Send");
                    }

                    return;
                }

                for (peer, block) in self.block_exchange.iter_wanted_blocks() {
                    if self.log {
                        println!("Who Want Block -> Peer: {} Cid: {}", peer, block.cid());
                    }

                    if !self.pending_send_block.insert((peer, *block.cid())) {
                        //if already sending this peer this block
                        if self.log {
                            println!(
                                "Already Sending Block -> Peer: {} Cid: {}",
                                peer,
                                block.cid()
                            );
                        }

                        continue;
                    }

                    if self.log {
                        println!("Sending Block -> Peer: {} Cid: {}", peer, block.cid());
                    }

                    self.swarm.behaviour_mut().bitswap.send_block(peer, block);

                    break;
                }
            }
            BitswapEvent::ReceivedCancel(peer_id, cid) => {
                self.block_exchange.remove_want(&peer_id, &cid);

                if self.log {
                    println!("Received Cancel -> Peer: {} Cid: {}", peer_id, cid);
                }

                if !self.pending_send_block.remove(&(peer_id, cid)) {
                    // if was not sending this peer this block
                    if self.log {
                        println!("Was Not Sending Block -> Peer: {} Cid: {}", peer_id, cid);
                    }

                    return;
                }

                for (peer, block) in self.block_exchange.iter_wanted_blocks() {
                    if self.log {
                        println!("Who Want Block -> Peer: {} Cid: {}", peer, block.cid());
                    }

                    if !self.pending_send_block.insert((peer, *block.cid())) {
                        //if already sending this peer this block
                        if self.log {
                            println!(
                                "Already Sending Block -> Peer: {} Cid: {}",
                                peer,
                                block.cid()
                            );
                        }

                        continue;
                    }

                    if self.log {
                        println!("Sending Block -> Peer: {} Cid: {}", peer, block.cid());
                    }

                    self.swarm.behaviour_mut().bitswap.send_block(peer, block);

                    break;
                }
            }
        }
    }
}
