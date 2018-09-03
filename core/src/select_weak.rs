// Shamelessly stolen from paritytech/jsonrpc/ipc crate
use futures::stream::{Fuse, Stream};
use futures::{Async, Poll};

pub trait SelectWithWeakExt: Stream {
    fn select_with_weak<S>(self, other: S) -> SelectWithWeak<Self, S>
    where
        S: Stream<Item = Self::Item, Error = Self::Error>,
        Self: Sized;
}

impl<T> SelectWithWeakExt for T
where
    T: Stream,
{
    fn select_with_weak<S>(self, other: S) -> SelectWithWeak<Self, S>
    where
        S: Stream<Item = Self::Item, Error = Self::Error>,
        Self: Sized,
    {
        new(self, other)
    }
}

/// An adapter for merging the output of two streams.
///
/// The merged stream produces items from either of the underlying streams as
/// they become available, and the streams are polled in a round-robin fashion.
/// Errors, however, are not merged: you get at most one error at a time.
///
/// Finishes when strong stream finishes
#[derive(Debug)]
#[must_use = "streams do nothing unless polled"]
pub struct SelectWithWeak<S1, S2> {
    strong: Fuse<S1>,
    weak: Fuse<S2>,
    use_strong: bool,
    weak_done: bool,
}

fn new<S1, S2>(stream1: S1, stream2: S2) -> SelectWithWeak<S1, S2>
where
    S1: Stream,
    S2: Stream<Item = S1::Item, Error = S1::Error>,
{
    SelectWithWeak {
        strong: stream1.fuse(),
        weak: stream2.fuse(),
        use_strong: false,
        weak_done: false,
    }
}

impl<S1, S2> SelectWithWeak<S1, S2>
where
    S1: Stream,
    S2: Stream<Item = S1::Item, Error = S1::Error>,
{
    fn check_weak(&mut self) -> Poll<Option<S1::Item>, S1::Error> {
        if !self.weak_done {
            let result = match self.weak.poll()? {
                Async::Ready(None) => {
                    self.weak_done = true;
                    Async::NotReady
                }
                others => others,
            };
            return Ok(result);
        }
        Ok(Async::NotReady)
    }
}

impl<S1, S2> Stream for SelectWithWeak<S1, S2>
where
    S1: Stream,
    S2: Stream<Item = S1::Item, Error = S1::Error>,
{
    type Item = S1::Item;
    type Error = S1::Error;

    fn poll(&mut self) -> Poll<Option<S1::Item>, S1::Error> {
        if !self.use_strong {
            self.use_strong = true;
            match self.check_weak() {
                Ok(Async::NotReady) => self.strong.poll(),
                others => others,
            }
        } else {
            self.use_strong = false;
            match self.strong.poll() {
                Ok(Async::NotReady) => self.check_weak(),
                others => others,
            }
        }
    }
}
