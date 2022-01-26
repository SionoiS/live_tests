use std::collections::HashSet;

use cid::Cid;

use ipfs_bitswap::Block;

use libp2p::PeerId;

#[derive(Default, Debug)]
pub struct BlockExchange {
    peers: Vec<PeerId>,

    peer_indices: Vec<usize>, // peer idx mapped to cid idx
    cid_indices: Vec<usize>,  // cid idx mapped to peer idx

    cids: Vec<Cid>, // First Cids with data then starting at index == data_store.len(), Cids without data
    data_store: Vec<Box<[u8]>>,
}

impl BlockExchange {
    pub fn who_want_block(&self, cid: &Cid) -> HashSet<PeerId> {
        let cid_idx = match self.cids.iter().position(|item| item == cid) {
            Some(i) => i,
            None => return HashSet::default(),
        };

        self.cid_indices
            .iter()
            .enumerate()
            .filter_map(move |(i, idx)| {
                if *idx == cid_idx {
                    let peer_idx = self.peer_indices[i];

                    Some(self.peers[peer_idx])
                } else {
                    None
                }
            })
            .collect()
    }

    pub fn add_want(&mut self, peer_id: PeerId, cid: Cid) {
        let mut peer_index = self.peers.iter().position(|item| *item == peer_id);
        let mut cid_index = self.cids.iter().position(|item| *item == cid);

        if peer_index.is_none() {
            peer_index = Some(self.peers.len());
            self.peers.push(peer_id);
        }

        if cid_index.is_none() {
            cid_index = Some(self.cids.len());
            self.cids.push(cid);
        }

        self.peer_indices.push(peer_index.unwrap());
        self.cid_indices.push(cid_index.unwrap());
    }

    pub fn remove_wants(&mut self, cid: &Cid) {
        if self.cid_indices.is_empty() {
            return;
        }

        let cid_index = match self.cids.iter().position(|item| item == cid) {
            Some(i) => i,
            None => return,
        };

        let mut i = self.cid_indices.len() - 1;
        loop {
            if self.cid_indices[i] == cid_index {
                self.peer_indices.swap_remove(i);
                self.cid_indices.swap_remove(i);
            }

            if i == 0 {
                break;
            }

            i -= 1;
        }
    }

    pub fn remove_want(&mut self, peer_id: &PeerId, cid: &Cid) {
        if self.peer_indices.is_empty() {
            return;
        }

        let peer_idx = match self.peers.iter().position(|item| item == peer_id) {
            Some(i) => i,
            None => return,
        };

        let mut i = self.peer_indices.len() - 1;
        loop {
            if peer_idx == self.peer_indices[i] && *cid == self.cids[self.cid_indices[i]] {
                self.peer_indices.swap_remove(i);
                self.cid_indices.swap_remove(i);
            }

            if i == 0 {
                break;
            }

            i -= 1;
        }
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

        let Block { cid, data } = block;

        for (i, id) in self.cids.iter().enumerate() {
            // Case: cid not found
            if *id != cid {
                continue;
            }

            // Case: cid & data are present
            if i < self.data_store.len() {
                return false;
            }

            // Case: cid is present but not data
            index = Some(i);
        }

        if let Some(i) = index {
            let last_index = self.cids.len() - 1;
            self.cids.swap(last_index, i);

            for cid_index in self.cid_indices.iter_mut() {
                if *cid_index == i {
                    *cid_index = last_index;
                }

                if *cid_index == last_index {
                    *cid_index = i;
                }
            }
        } else {
            self.cids.push(cid);
        }

        self.data_store.push(data);

        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use libp2p::identity;
    use rand::{Rng, SeedableRng};
    use rand_xoshiro::Xoshiro256StarStar;

    use crate::utils::get_random_block;

    fn get_random_peer_id() -> PeerId {
        let local_key = identity::Keypair::generate_ed25519();

        PeerId::from(local_key.clone().public())
    }

    #[test]
    fn empty_remove() {
        let mut rng = Xoshiro256StarStar::seed_from_u64(5634653465365u64);

        let block = get_random_block(&mut rng);
        let peer = get_random_peer_id();

        let mut exchange = BlockExchange::default();

        exchange.add_want(peer, block.cid().clone());

        exchange.remove_wants(block.cid());

        exchange.remove_want(&peer, block.cid());
    }

    #[test]
    fn block_roundtrip() {
        let mut rng = Xoshiro256StarStar::seed_from_u64(5634653465365u64);

        let block_one = get_random_block(&mut rng);

        let mut exchange = BlockExchange::default();

        exchange.add_block(block_one.clone());

        let block_two = exchange.get_block(block_one.cid()).unwrap();

        assert_eq!(block_one, block_two)
    }

    #[test]
    fn want_roundtrip() {
        let mut rng = Xoshiro256StarStar::seed_from_u64(5634653465365u64);

        let block = get_random_block(&mut rng);
        let peer = get_random_peer_id();
        let cid = block.cid().clone();

        let mut exchange = BlockExchange::default();

        exchange.add_want(peer, cid.clone());

        exchange.remove_want(&peer, &cid);

        let peers = exchange.who_want_block(&cid);

        assert!(peers.is_empty())
    }

    #[test]
    fn want_exchange() {
        let mut rng = Xoshiro256StarStar::seed_from_u64(5634653465365u64);

        let blocks = (0..5)
            .map(|_| get_random_block(&mut rng))
            .collect::<Vec<Block>>();

        let peers = (0..5)
            .map(|_| get_random_peer_id())
            .collect::<Vec<PeerId>>();

        let mut exchange = BlockExchange::default();

        // first peer want all blocks -> last peer want the last block
        for (i, peer) in peers.iter().enumerate() {
            for (j, block) in blocks.iter().enumerate() {
                if i > j {
                    continue;
                }

                exchange.add_want(*peer, block.cid().clone());
            }
        }

        let set = exchange.who_want_block(blocks[4].cid());

        assert_eq!(set, peers.clone().into_iter().collect::<HashSet<PeerId>>());

        exchange.remove_wants(blocks[0].cid());

        let set = exchange.who_want_block(blocks[0].cid());

        assert!(set.is_empty());

        exchange.add_want(peers[0], blocks[0].cid().clone());

        let set = exchange.who_want_block(blocks[0].cid());

        assert!(set.len() == 1)
    }

    #[test]
    fn exchange_fuzz() {
        let mut rng = Xoshiro256StarStar::seed_from_u64(5634653465365u64);

        let max = 100;

        let blocks = (0..max)
            .map(|_| get_random_block(&mut rng))
            .collect::<Vec<Block>>();

        let peers = (0..max)
            .map(|_| get_random_peer_id())
            .collect::<Vec<PeerId>>();

        let mut exchange = BlockExchange::default();

        for peer in peers.iter() {
            let rand: usize = rng.gen_range(1..max);
            // Random # of wants
            for _ in 0..rand {
                let rand_i: usize = rng.gen_range(0..max);
                // For random blocks
                exchange.add_want(*peer, blocks[rand_i].cid().clone());
            }
        }

        for block in blocks.iter() {
            exchange.add_block(block.clone());
        }

        for peer in peers.iter() {
            let rand: usize = rng.gen_range(1..max);
            // Random # of wants
            for _ in 0..rand {
                let rand_i: usize = rng.gen_range(0..max);
                // For random blocks
                exchange.remove_want(peer, blocks[rand_i].cid());
            }
        }

        for peer in peers.iter() {
            let rand: usize = rng.gen_range(1..max);
            // Random # of wants
            for _ in 0..rand {
                let rand_i: usize = rng.gen_range(0..max);
                // For random blocks
                exchange.add_want(*peer, blocks[rand_i].cid().clone());
            }
        }

        for block in blocks.iter() {
            exchange.remove_wants(block.cid());
        }
    }
}
