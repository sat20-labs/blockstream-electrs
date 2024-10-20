
use std::sync::mpsc::Receiver;
use std::thread;

use crate::chain::{Block, BlockHash};
use crate::daemon::Daemon;
use crate::errors::*;
use crate::util::{spawn_thread, HeaderEntry, SyncChannel};

pub fn start_fetcher(
    daemon: &Daemon,
    new_headers: Vec<HeaderEntry>,
) -> Result<Fetcher<Vec<BlockEntry>>> {
    satsnet_fetcher(daemon, new_headers)
}

#[derive(Clone)]
pub struct BlockEntry {
    pub block: Block,
    pub entry: HeaderEntry,
    pub size: u32,
}

pub struct Fetcher<T> {
    receiver: Receiver<T>,
    thread: thread::JoinHandle<()>,
}

impl<T> Fetcher<T> {
    fn from(receiver: Receiver<T>, thread: thread::JoinHandle<()>) -> Self {
        Fetcher { receiver, thread }
    }

    pub fn map<F>(self, mut func: F)
    where
        F: FnMut(T) -> (),
    {
        for item in self.receiver {
            func(item);
        }
        self.thread.join().expect("fetcher thread panicked")
    }
}

fn satsnet_fetcher(
    daemon: &Daemon,
    new_headers: Vec<HeaderEntry>,
) -> Result<Fetcher<Vec<BlockEntry>>> {
    if let Some(tip) = new_headers.last() {
        debug!("{:?} ({} left to index)", tip, new_headers.len());
    };
    let daemon = daemon.reconnect()?;
    let chan = SyncChannel::new(1);
    let sender = chan.sender();
    Ok(Fetcher::from(
        chan.into_receiver(),
        spawn_thread("bitcoind_fetcher", move || {
            for entries in new_headers.chunks(100) {
                let blockhashes: Vec<BlockHash> = entries.iter().map(|e| *e.hash()).collect();
                let blocks = daemon
                    .getblocks(&blockhashes)
                    .expect("failed to get blocks from satsnet");
                assert_eq!(blocks.len(), entries.len());
                let block_entries: Vec<BlockEntry> = blocks
                    .into_iter()
                    .zip(entries)
                    .map(|(block, entry)| BlockEntry {
                        entry: entry.clone(), // TODO: remove this clone()
                        size: block.total_size() as u32,
                        block,
                    })
                    .collect();
                assert_eq!(block_entries.len(), entries.len());
                sender
                    .send(block_entries)
                    .expect("failed to send fetched blocks");
                log::debug!("last fetch {:?}", entries.last());
            }
        }),
    ))
}