use std::{
    future::Future,
    marker::PhantomData,
    thread::{self, ThreadId},
};

use async_executor::Task;
use derive_more::{Deref, DerefMut};

type ExecutorInner<'a> = async_executor::Executor<'a>;

/// An executor that runs tasks on whichever threads tick it.
#[derive(Deref, DerefMut, Default, Debug)]
pub struct Executor<'a>(ExecutorInner<'a>);

impl<'a> Executor<'a> {
    /// Creates an executor with no tasks.
    pub const fn new() -> Self {
        Self(ExecutorInner::new())
    }
}

/// An executor whose tasks run only on the thread that created it.
///
/// Any thread may spawn onto it; only the owning thread gets a
/// [`ticker`](ThreadExecutor::ticker) to run them.
#[derive(Debug)]
pub struct ThreadExecutor<'task> {
    executor: Executor<'task>,
    thread_id: ThreadId,
}

impl<'task> ThreadExecutor<'task> {
    /// Creates an executor owned by the calling thread.
    pub fn new() -> Self {
        Self {
            executor: Executor::new(),
            thread_id: thread::current().id(),
        }
    }

    /// Returns a ticker that runs this executor's tasks, or `None` when called from a thread other
    /// than the owner.
    pub fn ticker<'ticker>(&'ticker self) -> Option<ThreadExecutorTicker<'task, 'ticker>> {
        if thread::current().id() == self.thread_id {
            return Some(ThreadExecutorTicker {
                executor: self,
                _marker: PhantomData,
            });
        }
        None
    }

    /// Whether the calling thread is the one this executor runs its tasks on.
    pub fn is_current_thread(&self) -> bool {
        thread::current().id() == self.thread_id
    }

    /// Spawns `future`, which runs the next time the owning thread ticks this executor.
    pub fn spawn<T: Send + 'task>(
        &self,
        future: impl Future<Output = T> + Send + 'task,
    ) -> Task<T> {
        self.executor.spawn(future)
    }
}

/// A handle that runs a [`ThreadExecutor`]'s tasks on its owning thread; it is neither `Send` nor
/// `Sync`.
#[derive(Debug)]
pub struct ThreadExecutorTicker<'task, 'ticker> {
    executor: &'ticker ThreadExecutor<'task>,
    // make type not send or sync
    _marker: PhantomData<*const ()>,
}

impl<'task, 'ticker> ThreadExecutorTicker<'task, 'ticker> {
    /// Runs one task that is ready, waiting until one is.
    pub async fn tick(&self) {
        self.executor.executor.tick().await;
    }
}

impl<'task> Default for ThreadExecutor<'task> {
    fn default() -> Self {
        Self::new()
    }
}
