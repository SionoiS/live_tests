#![allow(unused)]

use std::collections::{HashSet, VecDeque};

use cid::Cid;

use ipfs_bitswap::Block;

use libp2p::PeerId;

#[derive(Default, Debug)]
pub struct BlockExchange {
    peers: Vec<PeerId>,

    /// peers idx mapped to cids idx.
    peer_indices: Vec<usize>,
    /// cids idx mapped to peers idx
    cid_indices: Vec<usize>,

    /// First Cids with data then starting at index == datums.len(), Cids without data
    cids: Vec<Cid>,
    datums: Vec<Box<[u8]>>,
}

impl BlockExchange {
    pub fn insertion_sort_wants(&mut self) {
        let mut i = 1;

        while (i < self.peer_indices.len()) {
            let mut j = i;

            while (j > 0 && self.peer_indices[j - 1] > self.peer_indices[j]) {
                self.peer_indices.swap(j, j - 1);
                self.cid_indices.swap(j, j - 1);

                j -= 1;
            }

            i += 1;
        }
    }

    /// Iterate the want list unordered.
    pub fn iter_wanted_blocks(&self) -> impl Iterator<Item = (PeerId, Block)> + '_ {
        let closure = |(peer_index, cid_index): (&usize, &usize)| {
            if *cid_index >= self.datums.len() {
                //Dont have block
                None
            } else {
                let peer = self.peers[*peer_index];
                let block = Block::new(
                    self.datums[*cid_index].clone(),
                    self.cids[*cid_index].clone(),
                );

                Some((peer, block))
            }
        };

        self.peer_indices
            .iter()
            .zip(self.cid_indices.iter())
            .filter_map(closure)
    }

    pub fn iter_who_want_block(&self, cid: Cid) -> impl Iterator<Item = PeerId> + '_ {
        let closure = move |(i, idx): (usize, &usize)| {
            if self.cids[*idx] == cid {
                let peer_idx = self.peer_indices[i];

                Some(self.peers[peer_idx])
            } else {
                None
            }
        };

        self.cid_indices.iter().enumerate().filter_map(closure)
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
                //self.peer_indices.swap_remove(i);
                //self.cid_indices.swap_remove(i);
                // Perserving ordering give best algo for streaming?
                self.peer_indices.remove(i);
                self.cid_indices.remove(i);
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

        //Reverse loop swap remove
        let mut i = self.peer_indices.len() - 1;
        loop {
            if peer_idx == self.peer_indices[i] && *cid == self.cids[self.cid_indices[i]] {
                //self.peer_indices.swap_remove(i);
                //self.cid_indices.swap_remove(i);
                // Perserving ordering give best algo for streaming?
                self.peer_indices.remove(i);
                self.cid_indices.remove(i);
            }

            if i == 0 {
                break;
            }

            i -= 1;
        }
    }

    pub fn get_block(&self, cid: &Cid) -> Option<Block> {
        for (id, data) in self.cids.iter().zip(self.datums.iter()) {
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
            if i < self.datums.len() {
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

        self.datums.push(data);

        true
    }
}

#[cfg(test)]
mod tests {
    use std::hash::Hash;

    use super::*;

    use libp2p::identity;
    use rand::{Rng, SeedableRng};
    use rand_xoshiro::Xoshiro256StarStar;

    use crate::utils::{random_block, STANDARD_BLOCK_SIZE};

    fn get_random_peer_id() -> PeerId {
        let local_key = identity::Keypair::generate_ed25519();

        PeerId::from(local_key.clone().public())
    }

    #[test]
    fn empty_remove() {
        let mut rng = Xoshiro256StarStar::seed_from_u64(5634653465365u64);

        let block = random_block(&mut rng, STANDARD_BLOCK_SIZE);
        let peer = get_random_peer_id();

        let mut exchange = BlockExchange::default();

        exchange.add_want(peer, block.cid().clone());

        exchange.remove_wants(block.cid());

        exchange.remove_want(&peer, block.cid());
    }

    #[test]
    fn block_roundtrip() {
        let mut rng = Xoshiro256StarStar::seed_from_u64(5634653465365u64);

        let block_one = random_block(&mut rng, STANDARD_BLOCK_SIZE);

        let mut exchange = BlockExchange::default();

        exchange.add_block(block_one.clone());

        let block_two = exchange.get_block(block_one.cid()).unwrap();

        assert_eq!(block_one, block_two)
    }

    #[test]
    fn want_roundtrip() {
        let mut rng = Xoshiro256StarStar::seed_from_u64(5634653465365u64);

        let block = random_block(&mut rng, STANDARD_BLOCK_SIZE);
        let peer = get_random_peer_id();
        let cid = block.cid().clone();

        let mut exchange = BlockExchange::default();

        exchange.add_want(peer, cid.clone());

        exchange.remove_want(&peer, &cid);

        let peers: Vec<PeerId> = exchange.iter_who_want_block(cid).collect();

        assert!(peers.is_empty())
    }

    #[test]
    fn want_exchange() {
        let mut rng = Xoshiro256StarStar::seed_from_u64(5634653465365u64);

        let blocks = (0..5)
            .map(|_| random_block(&mut rng, STANDARD_BLOCK_SIZE))
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

        let set: HashSet<PeerId> = exchange
            .iter_who_want_block(blocks[4].cid().clone())
            .collect();

        assert_eq!(set, peers.clone().into_iter().collect::<HashSet<PeerId>>());

        exchange.remove_wants(blocks[0].cid());

        let set: HashSet<PeerId> = exchange
            .iter_who_want_block(blocks[0].cid().clone())
            .collect();

        assert!(set.is_empty());

        exchange.add_want(peers[0], blocks[0].cid().clone());

        let set: HashSet<PeerId> = exchange
            .iter_who_want_block(blocks[0].cid().clone())
            .collect();

        assert!(set.len() == 1)
    }

    #[test]
    fn exchange_fuzz() {
        let mut rng = Xoshiro256StarStar::seed_from_u64(5634653465365u64);

        let max = 10;

        let blocks = (0..max)
            .map(|_| random_block(&mut rng, STANDARD_BLOCK_SIZE))
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
                exchange.add_want(*peer, *blocks[rand_i].cid());
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
                exchange.add_want(*peer, *blocks[rand_i].cid());
            }
        }

        for block in blocks.iter() {
            exchange.remove_wants(block.cid());
        }
    }
}
