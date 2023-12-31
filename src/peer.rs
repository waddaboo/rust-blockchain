use std::panic;

use anyhow::Result;
use isahc::{ReadResponseExt, Request};

use crate::{
    model::{Block, Blockchain},
    util::{
        execution::{sleep_millis, Runnable},
        Context,
    },
};

pub struct Peer {
    peer_addresses: Vec<String>,
    blockchain: Blockchain,
    peer_sync_ms: u64,
}

impl Runnable for Peer {
    fn run(&self) -> Result<()> {
        self.start()
    }
}

impl Peer {
    pub fn new(context: &Context) -> Peer {
        Peer {
            peer_addresses: context.config.peers.clone(),
            blockchain: context.blockchain.clone(),
            peer_sync_ms: context.config.peer_sync_ms,
        }
    }

    fn get_last_block_index(&self) -> usize {
        self.blockchain.get_last_block().index as usize
    }

    fn get_new_blocks_from_peer(&self, address: &str) -> Vec<Block> {
        let last_index = self.blockchain.get_last_block().index as usize;

        let peer_blocks = self.get_blocks_from_peer(address);
        let peer_last_index = peer_blocks.last().unwrap().index as usize;

        if peer_last_index <= last_index {
            return Vec::<Block>::new();
        }

        let first_new = last_index + 1;
        let last_new = peer_last_index;
        let new_blocks_range = first_new..=last_new;

        peer_blocks.get(new_blocks_range).unwrap().to_vec()
    }

    fn add_new_blocks(&self, new_blocks: &[Block]) {
        for block in new_blocks.iter() {
            let result = self.blockchain.add_block(block.clone());

            if result.is_err() {
                error!("Could not add peer block {} to the blockchain", block.index);
                return;
            }

            info!("Added new peer block {} to the blockchain", block.index);
        }
    }

    fn try_receive_new_blocks(&self) {
        for address in self.peer_addresses.iter() {
            let result = panic::catch_unwind(|| {
                let new_blocks = self.get_new_blocks_from_peer(address);

                if !new_blocks.is_empty() {
                    self.add_new_blocks(&new_blocks);
                }
            });

            if result.is_err() {
                error!("Could not sync blocks from peer {}", address);
            }
        }
    }

    fn get_blocks_from_peer(&self, address: &str) -> Vec<Block> {
        let uri = format!("{}/blocks", address);
        let mut response = isahc::get(uri).unwrap();

        assert_eq!(response.status().as_u16(), 200);

        let raw_body = response.text().unwrap();

        serde_json::from_str(&raw_body).unwrap()
    }

    fn get_new_blocks_since(&self, start_index: usize) -> Vec<Block> {
        let last_block_index = self.get_last_block_index();
        let new_blocks_range = start_index + 1..=last_block_index;

        self.blockchain
            .get_all_blocks()
            .get(new_blocks_range)
            .unwrap()
            .to_vec()
    }

    fn send_block_to_peer(address: &str, block: &Block) {
        let uri = format!("{}/blocks", address);
        let body = serde_json::to_string(&block).unwrap();

        let request = Request::post(uri)
            .header("Content-Type", "application/json")
            .body(body)
            .unwrap();

        isahc::send(request).unwrap();
    }

    fn try_send_new_blocks(&self, last_send_block_index: usize) {
        let new_blocks = self.get_new_blocks_since(last_send_block_index);

        for block in new_blocks.iter() {
            for address in self.peer_addresses.iter() {
                let result = panic::catch_unwind(|| Peer::send_block_to_peer(address, block));

                if result.is_err() {
                    error!("Could not send block {} to peer {}", block.index, address);
                    return;
                }

                info!("Sended new block {} to peer {}", block.index, address);
            }
        }
    }

    pub fn start(&self) -> Result<()> {
        if self.peer_addresses.is_empty() {
            info!("No peers configured, exiting peer sync system");

            return Ok(());
        }

        info!(
            "Start peer system with peers: {}",
            self.peer_addresses.join(", ")
        );

        let mut last_sent_block_index = self.get_last_block_index();

        loop {
            self.try_receive_new_blocks();
            self.try_send_new_blocks(last_sent_block_index);
            last_sent_block_index = self.get_last_block_index();

            sleep_millis(self.peer_sync_ms);
        }
    }
}
