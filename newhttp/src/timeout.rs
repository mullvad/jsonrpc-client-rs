use futures::{Future, Poll};
use std::time::Duration;
use tokio_timer::Timeout;

pub use tokio_timer::timeout::Error;

/// Wraps a `Future` and optionally, give it a time limit to complete within.
/// This is a quite thin wrapper around `tokio_timer::Timeout` with the difference
/// that the timeout deadline is optional. Allowing static dispatch over a collection
/// of futures where only some of them might be time limited.
#[derive(Debug)]
pub enum OptionalTimeout<F: Future> {
    Unlimited(F),
    Limited(Timeout<F>),
}

impl<F: Future> OptionalTimeout<F> {
    /// Creates a new `OptionalTimeout` future.
    ///
    /// The duration parameter may be `None` to indicate there is no time limit. This means that
    /// `future` will run as normal.
    pub fn new(future: F, timeout: Option<Duration>) -> Self {
        match timeout {
            Some(timeout) => OptionalTimeout::Limited(Timeout::new(future, timeout)),
            None => OptionalTimeout::Unlimited(future),
        }
    }
}

impl<F: Future> Future for OptionalTimeout<F> {
    type Item = F::Item;
    type Error = Error<F::Error>;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        match self {
            OptionalTimeout::Unlimited(future) => future.poll().map_err(Error::inner),
            OptionalTimeout::Limited(future) => future.poll(),
        }
    }
}
