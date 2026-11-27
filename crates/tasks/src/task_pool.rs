use async_channel::Sender;
use async_executor::FallibleTask;
use concurrent_queue::ConcurrentQueue;
use futures_lite::future::FutureExt;
use std::{
    future::Future,
    marker::PhantomData,
    mem,
    num::NonZero,
    panic::{catch_unwind, resume_unwind, AssertUnwindSafe},
    sync::Arc,
    thread::JoinHandle,
};

use crate::thread_executor::{Executor, ThreadExecutor, ThreadExecutorTicker};

use super::Task;

fn available_parallelism() -> usize {
    std::thread::available_parallelism()
        .map(NonZero::<usize>::get)
        .unwrap_or(1)
}

/// A pool of worker threads that run spawned futures.
///
/// Dropping the pool stops and joins its threads.
pub struct TaskPool {
    executor: Arc<Executor<'static>>,
    threads: Vec<JoinHandle<()>>,
    shutdown_send: Sender<()>,
}

impl TaskPool {
    thread_local! {
        static LOCAL_EXECUTOR: async_executor::LocalExecutor<'static> = const { async_executor::LocalExecutor::new() };
        static THREAD_EXECUTOR: Arc<ThreadExecutor<'static>> = Arc::new(ThreadExecutor::new());
    }

    /// Creates a pool with one worker per available core, named `task-pool-{i}`.
    pub fn new() -> Self {
        Self::with_name("task-pool")
    }

    /// Returns the calling thread's [`ThreadExecutor`], which runs tasks spawned onto it only on
    /// this thread.
    pub fn get_thread_executor() -> Arc<ThreadExecutor<'static>> {
        Self::THREAD_EXECUTOR.with(Clone::clone)
    }

    /// Like [`new`](TaskPool::new), but worker threads are named `{name}-{i}`
    /// so they are identifiable in profilers and debuggers.
    pub fn with_name(name: &str) -> Self {
        Self::with_threads(name, available_parallelism())
    }

    /// A pool of exactly `threads` workers named `{name}-{i}`.
    ///
    /// # Panics
    ///
    /// Panics if the operating system fails to spawn a worker thread.
    pub fn with_threads(name: &str, threads: usize) -> Self {
        let executor = Arc::new(Executor::new());

        let (shutdown_send, shutdown_rcv) = async_channel::unbounded::<()>();

        let threads = (0..threads)
            .map(|i| {
                let executor = Arc::clone(&executor);
                let shutdown_rcv = shutdown_rcv.clone();
                let thread_name = format!("{name}-{i}");
                std::thread::Builder::new()
                    .name(thread_name.clone())
                    .spawn(move || {
                        profiling::register_thread!(&thread_name);
                        Self::LOCAL_EXECUTOR.with(|local_executor| loop {
                            let res = std::panic::catch_unwind(|| {
                                let tick_forever = async move {
                                    loop {
                                        local_executor.tick().await;
                                    }
                                };
                                pollster::block_on(
                                    executor.run(tick_forever.or(shutdown_rcv.recv())),
                                )
                            });
                            if let Ok(val) = res {
                                val.unwrap_err();
                                break;
                            }
                        })
                    })
                    .expect("Failed to spawn thread")
            })
            .collect();

        TaskPool {
            executor,
            threads,
            shutdown_send,
        }
    }

    /// Spawns `future` onto the worker threads and returns its [`Task`].
    pub fn spawn<T>(&self, future: impl Future<Output = T> + Send + 'static) -> Task<T>
    where
        T: Send + 'static,
    {
        Task::new(self.executor.spawn(future))
    }

    /// Spawns `future` onto the calling thread's local executor, so it never moves between threads.
    /// Only the pool's worker threads run their local executors, so a future spawned from any other
    /// thread never runs.
    pub fn spawn_local<T>(&self, future: impl Future<Output = T> + 'static) -> Task<T>
    where
        T: Send + 'static,
    {
        Task::new(Self::LOCAL_EXECUTOR.with(|local_executor| local_executor.spawn(future)))
    }

    /// Runs `f` and every task it spawns, returning the tasks' outputs.
    ///
    /// # Panics
    ///
    /// Panics if `f` or a spawned task panicked, raising it again only after every spawned
    /// task has finished.
    pub fn scope<'env, F, T>(&self, f: F) -> Vec<T>
    where
        F: for<'scope> FnOnce(&'scope ScopedTaskPool<'scope, 'env, T>),
        T: Send + 'static,
    {
        Self::THREAD_EXECUTOR
            .with(|scope_executor| self.scope_inner(scope_executor, scope_executor, f))
    }

    /// Like [`scope`](TaskPool::scope), but tasks spawned with
    /// [`ScopedTaskPool::spawn_on_external`] run on `external_executor` when it is given.
    ///
    /// The ECS executor passes the main thread's executor here so that systems which must run on
    /// the main thread do.
    ///
    /// # Panics
    ///
    /// Panics if `f` or a spawned task panicked, raising it again only after every spawned task has
    /// finished.
    pub fn scope_with_executor<'env, F, T>(
        &self,
        external_executor: Option<&ThreadExecutor>,
        f: F,
    ) -> Vec<T>
    where
        F: for<'scope> FnOnce(&'scope ScopedTaskPool<'scope, 'env, T>),
        T: Send + 'static,
    {
        Self::THREAD_EXECUTOR.with(|scope_executor| {
            // If an `external_executor` is passed, use that. Otherwise, get the executor stored
            // in the `THREAD_EXECUTOR` thread local.
            if let Some(external_executor) = external_executor {
                self.scope_inner(external_executor, scope_executor, f)
            } else {
                self.scope_inner(scope_executor, scope_executor, f)
            }
        })
    }

    fn scope_inner<'env, F, T>(
        &self,
        external_executor: &ThreadExecutor,
        scope_executor: &ThreadExecutor,
        f: F,
    ) -> Vec<T>
    where
        F: for<'scope> FnOnce(&'scope ScopedTaskPool<'scope, 'env, T>),
        T: Send + 'static,
    {
        let executor: &Executor = &self.executor;
        // SAFETY: the scope blocks until every task spawned in it has finished, so no borrow outlives this call.
        let executor: &'env Executor = unsafe { mem::transmute(executor) };

        // SAFETY: the scope blocks until every task spawned in it has finished, so no borrow outlives this call.
        let external_executor: &'env ThreadExecutor<'env> =
            unsafe { mem::transmute(external_executor) };
        // SAFETY: the scope blocks until every task spawned in it has finished, so no borrow outlives this call.
        let scope_executor: &'env ThreadExecutor<'env> = unsafe { mem::transmute(scope_executor) };

        let spawned_tasks: ConcurrentQueue<
            FallibleTask<Result<T, Box<dyn core::any::Any + Send>>>,
        > = ConcurrentQueue::unbounded();
        // SAFETY: the scope blocks until every task spawned in it has finished, so no borrow outlives this call.
        let spawned_tasks: &'env ConcurrentQueue<
            FallibleTask<Result<T, Box<dyn core::any::Any + Send>>>,
        > = unsafe { mem::transmute(&spawned_tasks) };

        let scope = ScopedTaskPool::new(executor, external_executor, scope_executor, spawned_tasks);
        // SAFETY: the scope blocks until every task spawned in it has finished, so no borrow outlives this call.
        let scope: &'env ScopedTaskPool<'_, 'env, T> = unsafe { mem::transmute(&scope) };
        let closure_panic = catch_unwind(AssertUnwindSafe(|| f(scope))).err();

        let (results, task_panic) = if spawned_tasks.is_empty() {
            (Vec::new(), None)
        } else {
            pollster::block_on(async move {
                let get_results = async {
                    let mut results = Vec::with_capacity(spawned_tasks.len());
                    let mut panic = None;
                    while let Ok(task) = spawned_tasks.pop() {
                        match task.await {
                            Some(Ok(val)) => results.push(val),
                            Some(Err(error_payload)) => {
                                panic.get_or_insert(error_payload);
                            }
                            None => {}
                        }
                    }

                    (results, panic)
                };

                let scope_ticker = scope_executor.ticker().unwrap();
                let external_ticker = if std::ptr::eq(external_executor, scope_executor) {
                    None
                } else {
                    external_executor.ticker()
                };

                Self::execute_scope(scope_ticker, external_ticker, get_results).await
            })
        };

        if let Some(payload) = closure_panic.or(task_panic) {
            resume_unwind(payload);
        }

        results
    }

    async fn execute_scope<'scope, 'ticker, R>(
        scope_ticker: ThreadExecutorTicker<'scope, 'ticker>,
        external_ticker: Option<ThreadExecutorTicker<'scope, 'ticker>>,
        get_results: impl Future<Output = R>,
    ) -> R {
        let execute_forever = async {
            let tick_forever = async {
                loop {
                    scope_ticker.tick().await;
                }
            };

            if let Some(external_ticker) = &external_ticker {
                let tick_external_forever = async {
                    loop {
                        external_ticker.tick().await;
                    }
                };
                tick_forever.or(tick_external_forever).await
            } else {
                tick_forever.await
            }
        };
        get_results.or(execute_forever).await
    }
}

impl Drop for TaskPool {
    fn drop(&mut self) {
        let _ = self.shutdown_send.close();

        for handle in self.threads.drain(..) {
            handle.join().expect("Failed to join thread");
        }
    }
}

impl Default for TaskPool {
    fn default() -> Self {
        Self::new()
    }
}

/// A scope opened by [`TaskPool::scope`] that spawns tasks which may borrow from the caller.
///
/// The scope does not return until every task spawned in it has finished.
pub struct ScopedTaskPool<'scope, 'env: 'scope, T: Send> {
    executor: &'scope Executor<'scope>,
    external_executor: &'scope ThreadExecutor<'scope>,
    scope_executor: &'scope ThreadExecutor<'scope>,
    spawned_tasks: &'scope ConcurrentQueue<FallibleTask<Result<T, Box<dyn core::any::Any + Send>>>>,
    // Note: These are necessary to make both env + scope invariant
    // A simple &'a T/&'a mut T phantom type would make 'a covariant
    // This makes sure the lifetime remains invariant (invariant + covariant = invariant)
    env_marker: PhantomData<&'env mut &'env ()>,
    scope_marker: PhantomData<&'scope mut &'scope ()>,
}

impl<'scope, 'env, T: Send> ScopedTaskPool<'scope, 'env, T> {
    /// Creates a scope over the given executors and task queue; [`TaskPool::scope`] is the way to
    /// open one.
    pub fn new(
        executor: &'scope Executor<'scope>,
        external_executor: &'scope ThreadExecutor<'scope>,
        scope_executor: &'scope ThreadExecutor<'scope>,
        spawned_tasks: &'scope ConcurrentQueue<
            FallibleTask<Result<T, Box<dyn core::any::Any + Send>>>,
        >,
    ) -> Self {
        Self {
            executor,
            external_executor,
            scope_executor,
            spawned_tasks,
            env_marker: PhantomData,
            scope_marker: PhantomData,
        }
    }

    /// Spawns `f` onto the pool's executor; the scope returns its output with the others.
    ///
    /// # Panics
    ///
    /// Panics if the scope's task queue is closed, which this crate never does.
    pub fn spawn<F: Future<Output = T> + Send + 'scope>(&self, f: F) {
        let task = self
            .executor
            .spawn(AssertUnwindSafe(f).catch_unwind())
            .fallible();

        self.spawned_tasks.push(task).unwrap();
    }

    /// Spawns `f` onto the external executor the scope was opened with, or the scope's thread
    /// when it was opened without one.
    ///
    /// # Panics
    ///
    /// Panics if the scope's task queue is closed, which this crate never does.
    pub fn spawn_on_external<F: Future<Output = T> + 'scope + Send>(&self, f: F) {
        let task = self
            .external_executor
            .spawn(AssertUnwindSafe(f).catch_unwind())
            .fallible();

        self.spawned_tasks.push(task).unwrap();
    }

    /// Spawns onto the thread that opened this scope and is waiting on it.
    ///
    /// # Panics
    ///
    /// Panics if the scope's task queue is closed, which this crate never does.
    pub fn spawn_on_scope<F: Future<Output = T> + 'scope + Send>(&self, f: F) {
        let task = self
            .scope_executor
            .spawn(AssertUnwindSafe(f).catch_unwind())
            .fallible();

        self.spawned_tasks.push(task).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    #[test]
    fn spawned_tasks_all_run_and_return_their_results() {
        let pool = TaskPool::new();
        let counter = Arc::new(AtomicUsize::new(0));
        let tasks: Vec<_> = (0..16u32)
            .map(|index| {
                let counter = Arc::clone(&counter);
                pool.spawn(async move {
                    counter.fetch_add(1, Ordering::Relaxed);
                    index * 2
                })
            })
            .collect();

        let results: Vec<u32> = tasks.into_iter().map(pollster::block_on).collect();

        assert_eq!(results, (0..16).map(|index| index * 2).collect::<Vec<_>>());
        assert_eq!(counter.load(Ordering::Relaxed), 16);
    }

    #[test]
    fn a_scope_whose_closure_panics_unwinds_with_that_panic_only_after_its_tasks_finish() {
        let pool = TaskPool::with_threads("panic-test", 2);
        for _ in 0..10 {
            let finished = std::sync::atomic::AtomicBool::new(false);
            let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| {
                pool.scope(|scope| {
                    scope.spawn(async { panic!("the task broke") });
                    scope.spawn(async {
                        std::thread::sleep(std::time::Duration::from_millis(100));
                        finished.store(true, Ordering::SeqCst);
                    });
                    panic!("the closure broke");
                })
            }));

            let payload = outcome.expect_err("the panic is raised again");
            assert!(finished.load(Ordering::SeqCst));
            assert_eq!(payload.downcast_ref::<&str>(), Some(&"the closure broke"));
        }
    }
}
