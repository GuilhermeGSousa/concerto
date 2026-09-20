use std::{
    future::Future,
    marker::PhantomData,
    thread::{self, ThreadId},
};

use async_executor::Task;
use derive_more::{Deref, DerefMut};

type ExecutorInner<'a> = async_executor::Executor<'a>;
// type LocalExecutorInner<'a> = async_executor::LocalExecutor<'a>;

#[derive(Deref, DerefMut, Default, Debug)]
pub struct Executor<'a>(ExecutorInner<'a>);

impl<'a> Executor<'a> {
    pub const fn new() -> Self {
        Self(ExecutorInner::new())
    }
}

#[derive(Debug)]
pub struct ThreadExecutor<'task> {
    executor: Executor<'task>,
    thread_id: ThreadId,
}

impl<'task> ThreadExecutor<'task> {
    pub fn new() -> Self {
        Self {
            executor: Executor::new(),
            thread_id: thread::current().id(),
        }
    }

    pub fn ticker<'ticker>(&'ticker self) -> Option<ThreadExecutorTicker<'task, 'ticker>> {
        if thread::current().id() == self.thread_id {
            return Some(ThreadExecutorTicker {
                executor: self,
                _marker: PhantomData,
            });
        }
        None
    }

    pub fn spawn<T: Send + 'task>(
        &self,
        future: impl Future<Output = T> + Send + 'task,
    ) -> Task<T> {
        self.executor.spawn(future)
    }
}

#[derive(Debug)]
pub struct ThreadExecutorTicker<'task, 'ticker> {
    executor: &'ticker ThreadExecutor<'task>,
    // make type not send or sync
    _marker: PhantomData<*const ()>,
}

impl<'task, 'ticker> ThreadExecutorTicker<'task, 'ticker> {
    pub async fn tick(&self) {
        self.executor.executor.tick().await;
    }
}

impl<'task> Default for ThreadExecutor<'task> {
    fn default() -> Self {
        Self::new()
    }
}
