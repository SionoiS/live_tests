use std::io;

use cid::Cid;
use futures_util::Stream;

use ipfs_bitswap::Block;
use libp2p::{
    gossipsub::{
        error::{PublishError, SubscriptionError},
        GossipsubMessage, MessageId,
    },
    kad::{BootstrapError, BootstrapOk},
    Multiaddr, PeerId, TransportError,
};

use tokio::sync::{mpsc, oneshot};
use tokio_stream::wrappers::UnboundedReceiverStream;

#[derive(Debug)]
pub enum Command {
    ListenOn {
        multi_addr: Multiaddr,
        sender: oneshot::Sender<Result<Multiaddr, TransportError<io::Error>>>,
    },

    DhtAdd {
        peer_id: PeerId,
        multi_addr: Multiaddr,
        sender: oneshot::Sender<()>,
    },

    Bootstrap {
        sender: oneshot::Sender<Result<BootstrapOk, BootstrapError>>,
    },

    Publish {
        topic: String,
        data: Vec<u8>,
        sender: oneshot::Sender<Result<MessageId, PublishError>>,
    },

    Subscribe {
        topic: String,
        sender: oneshot::Sender<Result<bool, SubscriptionError>>,
        stream: mpsc::UnboundedSender<GossipsubMessage>,
    },

    BlockGet {
        cid: Cid,
        sender: oneshot::Sender<Block>,
    },

    BlockAdd {
        block: Block,
    },

    Dial {
        multi_addr: Multiaddr,
    },
}

#[derive(Clone)]
pub struct IpfsClient {
    cmd_tx: mpsc::UnboundedSender<Command>,
}

impl IpfsClient {
    pub fn new() -> (Self, mpsc::UnboundedReceiver<Command>) {
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();

        (Self { cmd_tx }, cmd_rx)
    }

    pub fn dial(&self, multi_addr: Multiaddr) {
        let cmd = Command::Dial { multi_addr };

        self.cmd_tx.send(cmd).expect("Receiver not dropped");
    }

    pub async fn listen_on(
        &self,
        multi_addr: Multiaddr,
    ) -> Result<Multiaddr, TransportError<io::Error>> {
        let (sender, receiver) = oneshot::channel();

        let cmd = Command::ListenOn { multi_addr, sender };

        self.cmd_tx.send(cmd).expect("Receiver not dropped");

        receiver.await.expect("Sender not dropped")
    }

    #[allow(unused)]
    pub async fn dht_add_addr(&self, peer_id: PeerId, multi_addr: Multiaddr) {
        let (sender, receiver) = oneshot::channel();

        let cmd = Command::DhtAdd {
            peer_id,
            multi_addr,
            sender,
        };

        self.cmd_tx.send(cmd).expect("Receiver not dropped");

        receiver.await.expect("Sender not dropped")
    }

    #[allow(unused)]
    pub async fn dht_bootstrap(&self) -> Result<BootstrapOk, BootstrapError> {
        let (sender, receiver) = oneshot::channel();

        let cmd = Command::Bootstrap { sender };

        self.cmd_tx.send(cmd).expect("Receiver not dropped");

        receiver.await.expect("Sender not dropped")
    }

    pub async fn publish(&self, topic: &str, data: Vec<u8>) -> Result<MessageId, PublishError> {
        let (sender, receiver) = oneshot::channel();

        let topic = topic.to_owned();
        let cmd = Command::Publish {
            topic,
            data,
            sender,
        };

        self.cmd_tx.send(cmd).expect("Receiver not dropped");

        receiver.await.expect("Sender not dropped")
    }

    /// Subscribe to a topic.
    ///
    /// Only the first subscribe on a topic will yield messages.
    pub async fn subscribe(
        &self,
        topic: &str,
    ) -> Result<impl Stream<Item = GossipsubMessage>, SubscriptionError> {
        let (stream, mut receiver) = mpsc::unbounded_channel();
        let (sender, err_rx) = oneshot::channel();

        let topic = topic.to_owned();
        let cmd = Command::Subscribe {
            topic,
            sender,
            stream,
        };

        self.cmd_tx.send(cmd).expect("Receiver not dropped");

        match err_rx.await.expect("Sender not dropped") {
            Ok(status) => {
                if !status {
                    receiver.close();
                }
            }
            Err(e) => return Err(e),
        }

        Ok(UnboundedReceiverStream::new(receiver))
    }

    pub async fn get_block(&self, cid: Cid) -> Block {
        let (sender, receiver) = oneshot::channel();

        let cmd = Command::BlockGet { cid, sender };

        self.cmd_tx.send(cmd).expect("Receiver not dropped");

        receiver.await.expect("Sender not dropped")
    }

    pub async fn add_block(&self, block: Block) {
        let cmd = Command::BlockAdd { block };

        self.cmd_tx.send(cmd).expect("Receiver not dropped");
    }
}
