use crate::{
    blockcfg::{Block, HeaderHash},
    start_up::NodeStorage,
};
use chain_storage::{
    error::Error as StorageError,
    store::{for_path_to_nth_ancestor, BlockInfo},
};
use std::ops::Deref as _;
use tokio::prelude::*;
use tokio::sync::lock::Lock;

#[derive(Clone)]
pub struct Storage {
    inner: Lock<NodeStorage>,
}

pub struct BlockStream {
    lock: Lock<NodeStorage>,
    to_depth: u64,
    cur_depth: u64,
    pending_infos: Vec<BlockInfo<HeaderHash>>,
}

impl Storage {
    pub fn new(storage: NodeStorage) -> Self {
        Storage {
            inner: Lock::new(storage),
        }
    }

    pub fn get_tag(
        &self,
        tag: String,
    ) -> impl Future<Item = Option<HeaderHash>, Error = StorageError> {
        let mut inner = self.inner.clone();

        future::poll_fn(move || Ok(inner.poll_lock())).and_then(move |guard| {
            match guard.get_tag(&tag) {
                Err(error) => future::err(error),
                Ok(res) => future::ok(res),
            }
        })
    }

    pub fn put_tag(
        &mut self,
        tag: String,
        header_hash: HeaderHash,
    ) -> impl Future<Item = (), Error = StorageError> {
        let mut inner = self.inner.clone();

        future::poll_fn(move || Ok(inner.poll_lock())).and_then(move |mut guard| {
            match guard.put_tag(&tag, &header_hash) {
                Err(error) => future::err(error),
                Ok(res) => future::ok(res),
            }
        })
    }

    pub fn get(
        &self,
        header_hash: HeaderHash,
    ) -> impl Future<Item = Option<Block>, Error = StorageError> {
        let mut inner = self.inner.clone();

        future::poll_fn(move || Ok(inner.poll_lock())).and_then(move |guard| {
            match guard.get_block(&header_hash) {
                Err(StorageError::BlockNotFound) => future::ok(None),
                Err(error) => future::err(error),
                Ok((block, _block_info)) => future::ok(Some(block)),
            }
        })
    }

    pub fn block_exists(
        &self,
        header_hash: HeaderHash,
    ) -> impl Future<Item = bool, Error = StorageError> {
        let mut inner = self.inner.clone();

        future::poll_fn(move || Ok(inner.poll_lock())).and_then(move |guard| {
            match guard.block_exists(&header_hash) {
                Err(StorageError::BlockNotFound) => future::ok(false),
                Err(error) => future::err(error),
                Ok(existence) => future::ok(existence),
            }
        })
    }

    pub fn put_block(&mut self, block: Block) -> impl Future<Item = (), Error = StorageError> {
        let mut inner = self.inner.clone();

        future::poll_fn(move || Ok(inner.poll_lock())).and_then(move |mut guard| {
            match guard.put_block(&block) {
                Err(StorageError::BlockNotFound) => unreachable!(),
                Err(error) => future::err(error),
                Ok(()) => future::ok(()),
            }
        })
    }

    pub fn stream_from_to(
        &self,
        from: HeaderHash,
        to: HeaderHash,
    ) -> impl Future<Item = Option<BlockStream>, Error = StorageError> {
        let mut inner = self.inner.clone();
        let inner_2 = self.inner.clone();

        future::poll_fn(move || Ok(inner.poll_lock())).and_then(move |store| {
            match store.is_ancestor(&from, &to) {
                Err(error) => future::err(error),
                Ok(None) => future::ok(None),
                Ok(Some(distance)) => match store.get_block_info(&to) {
                    Err(error) => future::err(error),
                    Ok(to_info) => future::ok(Some(BlockStream {
                        lock: inner_2,
                        to_depth: to_info.depth,
                        cur_depth: to_info.depth - distance,
                        pending_infos: vec![to_info],
                    })),
                },
            }
        })
    }
}

impl Stream for BlockStream {
    type Item = BlockInfo<HeaderHash>;
    type Error = StorageError;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        if self.cur_depth >= self.to_depth {
            return Ok(Async::Ready(None));
        }

        let guard = try_ready!(Ok(self.lock.poll_lock()));

        self.cur_depth += 1;

        let block_info = self.pending_infos.pop().unwrap();

        if block_info.depth == self.cur_depth {
            // We've seen this block on a previous ancestor traversal.
            Ok(Async::Ready(Some(block_info)))
        } else {
            // We don't have this block yet, so search back from
            // the furthest block that we do have.
            assert!(self.cur_depth < block_info.depth);
            let depth = block_info.depth;
            let parent = block_info.parent_id();
            self.pending_infos.push(block_info);
            let block_info = for_path_to_nth_ancestor(
                guard.deref().deref(),
                &parent,
                depth - self.cur_depth - 1,
                |new_info| {
                    self.pending_infos.push(new_info.clone());
                },
            )?;

            Ok(Async::Ready(Some(block_info)))
        }
    }
}
