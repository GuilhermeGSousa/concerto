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

    pub fn new() -> Self {
        Self::with_name("task-pool")
    }

    pub fn get_thread_executor() -> Arc<ThreadExecutor<'static>> {
        Self::THREAD_EXECUTOR.with(Clone::clone)
    }

    /// Like [`new`](TaskPool::new), but worker threads are named `{name}-{i}`
    /// so they are identifiable in profilers and debuggers.
    pub fn with_name(name: &str) -> Self {
        Self::with_threads(name, available_parallelism())
    }

    /// A pool of exactly `threads` workers named `{name}-{i}`.
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

    pub fn spawn<T>(&self, future: impl Future<Output = T> + Send + 'static) -> Task<T>
    where
        T: Send + 'static,
    {
        Task::new(self.executor.spawn(future))
    }

    pub fn spawn_local<T>(&self, future: impl Future<Output = T> + 'static) -> Task<T>
    where
        T: Send + 'static,
    {
        Task::new(Self::LOCAL_EXECUTOR.with(|local_executor| local_executor.spawn(future)))
    }

    /// Runs `f` and every task it spawns; a panic in either is raised again only after every spawned task has finished.
    pub fn scope<'env, F, T>(&self, f: F) -> Vec<T>
    where
        F: for<'scope> FnOnce(&'scope ScopedTaskPool<'scope, 'env, T>),
        T: Send + 'static,
    {
        Self::THREAD_EXECUTOR
            .with(|scope_executor| self.scope_inner(scope_executor, scope_executor, f))
    }

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
        let executor: &'env Executor = unsafe { mem::transmute(executor) };

        let external_executor: &'env ThreadExecutor<'env> =
            unsafe { mem::transmute(external_executor) };
        let scope_executor: &'env ThreadExecutor<'env> = unsafe { mem::transmute(scope_executor) };

        let spawned_tasks: ConcurrentQueue<
            FallibleTask<Result<T, Box<dyn core::any::Any + Send>>>,
        > = ConcurrentQueue::unbounded();
        let spawned_tasks: &'env ConcurrentQueue<
            FallibleTask<Result<T, Box<dyn core::any::Any + Send>>>,
        > = unsafe { mem::transmute(&spawned_tasks) };

        let scope = ScopedTaskPool::new(executor, external_executor, scope_executor, spawned_tasks);
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
            loop {
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
                    tick_forever.or(tick_external_forever).await;
                } else {
                    tick_forever.await;
                }
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

    pub fn spawn<F: Future<Output = T> + Send + 'scope>(&self, f: F) {
        let task = self
            .executor
            .spawn(AssertUnwindSafe(f).catch_unwind())
            .fallible();

        self.spawned_tasks.push(task).unwrap();
    }

    pub fn spawn_on_external<F: Future<Output = T> + 'scope + Send>(&self, f: F) {
        let task = self
            .external_executor
            .spawn(AssertUnwindSafe(f).catch_unwind())
            .fallible();

        self.spawned_tasks.push(task).unwrap();
    }

    /// Spawns onto the thread that opened this scope and is waiting on it.
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
    fn spawn_returns_result() {
        let pool = TaskPool::new();
        let task = pool.spawn(async { 42u32 });
        let result = pollster::block_on(task);
        assert_eq!(result, 42);
    }

    #[test]
    fn spawn_multiple_tasks_concurrently() {
        let pool = TaskPool::new();
        let counter = Arc::new(AtomicUsize::new(0));

        let tasks: Vec<_> = (0..16)
            .map(|_| {
                let counter = Arc::clone(&counter);
                pool.spawn(async move {
                    counter.fetch_add(1, Ordering::Relaxed);
                })
            })
            .collect();

        for task in tasks {
            pollster::block_on(task);
        }

        assert_eq!(counter.load(Ordering::Relaxed), 16);
    }

    #[test]
    fn scope_collects_results() {
        let pool = TaskPool::new();
        let results = pool.scope(|scope| {
            for i in 0u32..8 {
                scope.spawn(async move { i * 2 });
            }
        });

        assert_eq!(results.len(), 8);
        let mut sorted = results;
        sorted.sort();
        assert_eq!(sorted, vec![0, 2, 4, 6, 8, 10, 12, 14]);
    }

    #[test]
    fn scope_with_no_tasks_returns_empty() {
        let pool = TaskPool::new();
        let results = pool.scope(|_scope: &ScopedTaskPool<u32>| {});
        assert!(results.is_empty());
    }

    #[test]
    fn scope_shared_state_mutation() {
        let pool = TaskPool::new();
        let counter = Arc::new(AtomicUsize::new(0));

        pool.scope(|scope| {
            for _ in 0..4 {
                let counter = Arc::clone(&counter);
                scope.spawn(async move {
                    counter.fetch_add(1, Ordering::Relaxed);
                });
            }
        });

        assert_eq!(counter.load(Ordering::Relaxed), 4);
    }

    #[test]
    fn drop_joins_threads() {
        // If Drop impl is broken this will hang or panic.
        let pool = TaskPool::new();
        drop(pool);
    }

    #[test]
    fn a_task_spawned_on_the_scope_runs_on_the_thread_that_opened_it() {
        let pool = TaskPool::with_threads("scope-test", 1);
        let opener = std::thread::current().id();

        let ran_on = pool.scope(|scope| {
            scope.spawn_on_scope(async { std::thread::current().id() });
        });

        assert_eq!(ran_on, vec![opener]);
    }

    #[test]
    fn a_scope_whose_task_panics_unwinds_only_after_its_other_tasks_finish() {
        let pool = TaskPool::with_threads("panic-test", 2);
        let finished: Vec<std::sync::atomic::AtomicBool> = (0..10)
            .map(|_| std::sync::atomic::AtomicBool::new(false))
            .collect();

        for flag in &finished {
            let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| {
                pool.scope(|scope| {
                    scope.spawn(async { panic!("the task broke") });
                    scope.spawn(async {
                        std::thread::sleep(std::time::Duration::from_millis(100));
                        flag.store(true, Ordering::SeqCst);
                    });
                    scope.spawn(async { panic!("a later task broke") });
                })
            }));

            let payload = outcome.expect_err("the panic is raised again");
            assert!(flag.load(Ordering::SeqCst));
            assert_eq!(payload.downcast_ref::<&str>(), Some(&"the task broke"));
        }
    }

    #[test]
    fn a_scope_whose_closure_panics_unwinds_only_after_its_tasks_finish() {
        let pool = TaskPool::with_threads("closure-panic-test", 2);
        let finished: Vec<std::sync::atomic::AtomicBool> = (0..10)
            .map(|_| std::sync::atomic::AtomicBool::new(false))
            .collect();

        for flag in &finished {
            let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| {
                pool.scope(|scope| {
                    scope.spawn(async { panic!("the task broke") });
                    scope.spawn(async {
                        std::thread::sleep(std::time::Duration::from_millis(100));
                        flag.store(true, Ordering::SeqCst);
                    });
                    panic!("the closure broke");
                })
            }));

            let payload = outcome.expect_err("the panic is raised again");
            assert!(flag.load(Ordering::SeqCst));
            assert_eq!(payload.downcast_ref::<&str>(), Some(&"the closure broke"));
        }
    }
}
