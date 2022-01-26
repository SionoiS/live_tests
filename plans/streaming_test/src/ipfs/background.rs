use std::{collections::HashMap, io};

use cid::Cid;
use ipfs_bitswap::{BitswapEvent, Block};

use libp2p::{
    core::{connection::ListenerId, either::EitherError},
    gossipsub::{
        error::GossipsubHandlerError, GossipsubEvent, GossipsubMessage, IdentTopic, TopicHash,
    },
    kad::{BootstrapError, BootstrapOk, KademliaEvent, QueryId, QueryResult},
    swarm::{ProtocolsHandlerUpgrErr, SwarmEvent},
    Multiaddr, PeerId, Swarm, TransportError,
};

use tokio::sync::{mpsc, oneshot};
use tokio_stream::{wrappers::ReceiverStream, StreamExt};

use super::{
    block_exchange::BlockExchange,
    client::Command,
    composition::{ComposedBehaviour, ComposedEvent},
};

pub struct IpfsBackgroundService {
    cmd_rx: ReceiverStream<Command>,
    swarm: Swarm<ComposedBehaviour>,

    pending_subscribe: HashMap<TopicHash, mpsc::Sender<GossipsubMessage>>,
    routing_updates: HashMap<PeerId, oneshot::Sender<()>>,
    pending_bootstap: HashMap<QueryId, oneshot::Sender<Result<BootstrapOk, BootstrapError>>>,
    pending_listen:
        HashMap<ListenerId, oneshot::Sender<Result<Multiaddr, TransportError<io::Error>>>>,
    block_exchange: BlockExchange,

    pending_block: HashMap<Cid, oneshot::Sender<Block>>,
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
            pending_block: Default::default(),
        }
    }

    pub async fn run(&mut self) {
        loop {
            tokio::select! {
                event = self.swarm.next() => self.event(event).await,
                cmd = self.cmd_rx.next() => match cmd {
                    Some(cmd) =>self.command(cmd).await,
                    None => return,
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
                self.swarm
                    .behaviour_mut()
                    .bitswap
                    .want_block(cid.clone(), 0);

                self.pending_block.insert(cid, sender);
            }
            Command::BlockAdd { block } => {
                self.block_exchange.add_block(block);
            }
        }
    }

    async fn event(
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
            SwarmEvent::Behaviour(ComposedEvent::Kademlia(event)) => self.kademlia(event).await,
            SwarmEvent::Behaviour(ComposedEvent::Gossipsub(event)) => self.gossipsub(event).await,
            SwarmEvent::Behaviour(ComposedEvent::Bitswap(event)) => self.bitswap(event).await,
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

                sender.send(message).await.expect("Receiver not dropped")
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
        match event {
            BitswapEvent::ReceivedBlock(_, block) => {
                let cid = block.cid();

                if let Some(sender) = self.pending_block.remove(cid) {
                    sender.send(block.clone()).expect("Receiver not dropped");
                }

                let set = self.block_exchange.who_want_block(cid);

                for peer in set {
                    self.swarm
                        .behaviour_mut()
                        .bitswap
                        .send_block(peer, block.clone());
                }

                self.block_exchange.remove_wants(cid);

                self.block_exchange.add_block(block);
            }
            BitswapEvent::ReceivedWant(peer_id, cid, _priority) => {
                let block = match self.block_exchange.get_block(&cid) {
                    Some(b) => b,
                    None => {
                        self.block_exchange.add_want(peer_id, cid);
                        return;
                    }
                };

                self.swarm
                    .behaviour_mut()
                    .bitswap
                    .send_block(peer_id, block);
            }
            BitswapEvent::ReceivedCancel(peer_id, cid) => {
                self.block_exchange.remove_want(&peer_id, &cid);
            }
        }
    }
}
