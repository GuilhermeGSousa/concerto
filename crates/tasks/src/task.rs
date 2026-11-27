use derive_more::Deref;
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

/// A spawned future whose output is read by awaiting it.
///
/// Dropping a `Task` cancels the future.
#[derive(Deref)]
pub struct Task<T>(async_executor::Task<T>);

impl<T> Future for Task<T> {
    type Output = T;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<T> {
        Pin::new(&mut self.0).poll(cx)
    }
}

impl<T> Task<T> {
    /// Wraps an [`async_executor::Task`].
    pub fn new(task: async_executor::Task<T>) -> Self {
        Task(task)
    }

    /// Returns whether the future has finished, without waiting for it.
    pub fn is_finished(&self) -> bool {
        self.0.is_finished()
    }
}
