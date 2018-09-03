use futures::stream::{Fuse, Stream};
use futures::{Async, Poll};

pub trait SelectWithWeakExt: Stream + Sized {
    fn select_with_weak<S>(self, weak: S) -> SelectWithWeak<Self, S>
    where
        S: Stream<Item = Self::Item, Error = Self::Error>;
}

impl<T: Stream> SelectWithWeakExt for T {
    fn select_with_weak<S>(self, weak: S) -> SelectWithWeak<Self, S>
    where
        S: Stream<Item = Self::Item, Error = Self::Error>,
        Self: Sized,
    {
        SelectWithWeak {
            strong: self.fuse(),
            weak: weak.fuse(),
            use_strong: false,
            weak_done: false,
        }
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

impl<S1, S2> SelectWithWeak<S1, S2>
where
    S1: Stream,
    S2: Stream<Item = S1::Item, Error = S1::Error>,
{
    fn check_weak(&mut self) -> Poll<Option<S1::Item>, S1::Error> {
        if !self.weak_done {
            return Ok(Async::NotReady);
        };
        match self.weak.poll() {
            Ok(Async::Ready(None)) => {
                self.weak_done = true;
                Ok(Async::NotReady)
            }
            other => other,
        }
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
                other => other,
            }
        } else {
            self.use_strong = false;
            match self.strong.poll() {
                Ok(Async::NotReady) => self.check_weak(),
                other => other,
            }
        }
    }
}
