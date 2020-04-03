// Copyright (c) 2018 Fabian Schuiki

//! A throttle that reduces the number of futures that run in parallel.

use std::sync::Mutex;

use futures::task::{self, Task};
use futures::{Async, Future, IntoFuture, Poll};

/// A throttling pool for futures.
///
/// Ensures that only a limited number of futures can execute at the same time.
/// This is useful for throttling network connections or disk I/O.
pub struct FutureThrottle {
    size: usize,
    alloc: Mutex<(usize, Vec<Task>)>,
}

impl FutureThrottle {
    /// Create a new throttling pool.
    ///
    /// Use `spawn()` to spawn an arbitrary number of futures onto the pool. At
    /// most `size` futures will be polled in parallel.
    pub fn new(size: usize) -> FutureThrottle {
        FutureThrottle {
            size: size,
            alloc: Mutex::new((0, Vec::new())),
        }
    }

    /// Spawn a future onto the pool.
    ///
    /// The returned future can be polled as usual, but it may return
    /// `Async::NotReady` not because the future `f` was not ready, but because
    /// maximum number of futures in the pool are already executing.
    pub fn spawn<'a, F>(&'a self, f: F) -> ThrottledFuture<'a, <F as IntoFuture>::Future>
    where
        F: IntoFuture,
    {
        ThrottledFuture {
            pool: self,
            state: State::Fresh,
            inner: f.into_future(),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum State {
    Fresh,
    Allocated,
    Done,
}

/// The result of `FutureThrottle::spawn()`.
pub struct ThrottledFuture<'pool, F: Future> {
    pool: &'pool FutureThrottle,
    state: State,
    inner: F,
}

impl<'pool, F: Future> Future for ThrottledFuture<'pool, F> {
    type Item = F::Item;
    type Error = F::Error;

    fn poll(&mut self) -> Poll<F::Item, F::Error> {
        // Allocate a slot in the pool.
        if self.state == State::Fresh {
            let mut alloc = self.pool.alloc.lock().unwrap();
            if alloc.0 < self.pool.size {
                alloc.0 += 1;
                self.state = State::Allocated;
            } else {
                let task = task::current();
                alloc.1.push(task);
                return Ok(Async::NotReady);
            }
        }

        // Poll the inner future.
        if self.state == State::Allocated {
            let result = match self.inner.poll() {
                Ok(Async::Ready(v)) => Ok(v),
                Ok(Async::NotReady) => return Ok(Async::NotReady),
                Err(e) => Err(e),
            };
            let task = {
                let mut alloc = self.pool.alloc.lock().unwrap();
                alloc.0 -= 1;
                alloc.1.pop()
            };
            if let Some(task) = task {
                task.notify();
            }
            self.state = State::Done;
            return match result {
                Ok(v) => Ok(Async::Ready(v)),
                Err(e) => Err(e),
            };
        }

        // Catch repolling of the finished future and panic.
        panic!("pooled future polled after Async::Ready was returned");
    }
}

// extern crate futures;
// extern crate tokio_timer;
//
// use tokio_timer::Timer;
// use std::time::Duration;
//
// fn main() {
//     let pool = FutureThrottle::new(4);
//     let timer = &Timer::default();
//     let sleeps: Vec<_> = (0..10).map(|i| pool.spawn(
//         futures::lazy(move ||{
//             println!("starting {}", i);
//             Ok(())
//         })
//         .and_then(move |_| timer.sleep(Duration::from_secs(1)))
//         .and_then(move |_|{
//             println!("stopping {}", i);
//             Ok(())
//         })
//     )).collect();
//
//     println!("start");
//     // sleeps.pop().unwrap().wait();
//     futures::future::join_all(sleeps).wait().unwrap();
//     println!("done");
// }
