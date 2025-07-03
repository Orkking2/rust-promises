#![doc(html_root_url = "https://docs.rs/promise/1.0.0")]
//!
//! # Promises for Rust
//!
//! This crate provides a JavaScript-inspired, ergonomic, and composable Promise type for Rust, supporting background work, chaining, and error handling with `Result`.
//!
//! ## Features
//! - ECMAScript 5-style promises with Rust's type safety
//! - Chaining with `.then`, `.map`, and `.map_err`
//! - Combinators: `Promise::all`, `Promise::race`
//! - Panic-safe: panics in promise tasks are detected and reported
//! - Fully documented with tested examples
//!
//! ## Example
//! ```
//! use promisery::Promise;
//! let p = Promise::<_, ()>::new(|| Ok(2))
//!     .then(|res| res.map(|v| v * 10))
//!     .then(|res| res.map(|v| v + 5));
//! assert_eq!(p.wait(), Ok(25));
//! ```
//!
//! ## Error Handling
//! All errors are handled via `Result<T, E>`. Panics in promise tasks are reported via [`PromisePanic`] (see [`Promise::wait_nopanic`]).
//!
//! ## See Also
//! - [`Promise`] for the main type
//! - [`PromisePanic`] for panic detection
//! - [README.md](https://github.com/Orkking2/rust-promises#readme) for more details and usage
//!
//! ---
//!
//! Released under the MIT License.

#![warn(missing_docs)]

#[cfg(test)]
mod tests;

use std::{
    marker::Send,
    sync::mpsc::{self, Sender},
    thread,
};

#[derive(Debug, PartialEq)]
/// Error type returned when a promise panics during execution.
/// 
/// Only useful for the [`wait_nopanic`](Promise::wait_nopanic) method.
/// 
/// # Example
/// ```
/// # use promisery::{Promise, PromisePanic};
/// # use std::thread::sleep;
/// # use std::time::Duration;
/// let p = Promise::<(), ()>::new(|| panic!());
/// # sleep(Duration::from_millis(50));
/// assert_eq!(p.wait_nopanic(), Err(PromisePanic));
/// ```
pub struct PromisePanic;

/// A promise is a way of doing work in the background, similar to JavaScript promises.
///
/// Promises in this library have the same featureset as those in ECMAScript 5, but use Rust's `Result` type for error handling.
/// 
/// Similarly to the JS promise, this [`Promise`] implements an event queue to handle [`then`](Self::then). This means there is very little
/// overhead when chaining methods. Every [`Promise`] (except when using [`all`](Self::all)) must have its own thread, of course, so they can 
/// each evaluate their functions independently.
///
/// # States
/// Promises can be pending (work in progress), fulfilled (result ready), or panicked (background task panicked).
/// To use the result of a fulfilled promise, attach another promise to it (e.g., via [`then`]).
///
/// # Error Handling
/// Unlike JavaScript, success or failure is indicated by a `Result<T, E>`, allowing ergonomic use of Rust's error handling idioms.
///
/// # Panics
/// If a function passed to a promise, through any of the methods which allow such actions (e.g. [`new`](Self::new), [`then`](Self::then))
/// panics, then the promise enters a special state as described by [`poll`](Self::poll). Such promises will not continue to evaluate any functions.
/// If you are working with functions that can panic (beyond your control), use [`wait_nopanic`](Self::wait_nopanic) instead of [`wait`](Self::wait). 
///
/// **Thus**:
/// It remains the general recommendation to use `Result`-based error handling
/// rather than `unwrap()` in your functions, and this advice remains true for promise functions.
pub struct Promise<T: Send, E: Send> {
    rx: oneshot::Receiver<Result<T, E>>,
    jtx: Sender<Box<dyn FnOnce() + Send + 'static>>,
}

impl<T: Send + 'static, E: Send + 'static> Promise<T, E> {
    /// Returns `true` if the promise has a `Result` ready to be reaped, and `false` otherwise.
    /// Returns a [`PromisePanic`] if this promise has panicked.
    ///
    /// If this returns `true`, then [`wait`](Self::wait) is guaranteed to not block.
    /// 
    /// # Examples
    /// ## Successful promise
    /// ```
    /// # use promisery::Promise;
    /// # use std::thread::sleep;
    /// # use std::time::Duration;
    /// let p = Promise::<(), ()>::new(|| Ok(()));
    /// # sleep(Duration::from_millis(50));
    /// assert_eq!(p.poll(), Ok(true));
    /// ```
    /// 
    /// ## Promise still pending
    /// ```
    /// # use promisery::Promise;
    /// # use std::thread::sleep;
    /// # use std::time::Duration;
    /// let p = Promise::<(), ()>::new(|| { sleep(Duration::from_secs(1)); Ok(()) });
    /// assert_eq!(p.poll(), Ok(false));
    /// ```
    /// 
    /// ## Promise panicked
    /// ```
    /// # use promisery::{Promise, PromisePanic};
    /// # use std::thread::sleep;
    /// # use std::time::Duration;
    /// let p = Promise::<(), ()>::new(|| panic!());
    /// # sleep(Duration::from_millis(50));
    /// assert_eq!(p.poll(), Err(PromisePanic));
    /// ```
    pub fn poll(&self) -> Result<bool, PromisePanic> {
        if self.panicked() {
            Err(PromisePanic)
        } else {
            Ok(self.fulfilled())
        }
    }

    /// Returns `true` if there is a message ready to be reaped, `false` otherwise.
    /// If `true` is returned, [`wait`](Self::wait) is guaranteed to succeed without blocking.
    /// 
    /// # Examples
    /// ## Successful promise
    /// ```
    /// # use promisery::Promise;
    /// # use std::thread::sleep;
    /// # use std::time::Duration;
    /// let p = Promise::<(), ()>::new(|| Ok(()));
    /// # sleep(Duration::from_millis(50));
    /// assert!(p.fulfilled());
    /// ```
    /// 
    /// ## Unsuccessful promise
    /// ```
    /// # use promisery::Promise;
    /// # use std::thread::sleep;
    /// # use std::time::Duration;
    /// let p = Promise::<(), ()>::new(|| panic!());
    /// # sleep(Duration::from_millis(50));
    /// assert!(!p.fulfilled());
    /// ```
    pub fn fulfilled(&self) -> bool {
        self.rx.has_message()
    }

    /// Returns `true` if this promise panicked.
    /// 
    /// # Example
    /// ```
    /// # use promisery::Promise;
    /// # use std::thread::sleep;
    /// # use std::time::Duration;
    /// let p = Promise::<(), ()>::new(|| panic!());
    /// # sleep(Duration::from_millis(50));
    /// assert!(p.panicked());
    /// ```
    pub fn panicked(&self) -> bool {
        self.rx.is_closed()
    }

    /// Blocks until a `Result` is ready, then returns it.
    ///
    /// # Panics
    /// Panics if this [`Promise`] has panicked, which should not occur in normal use.
    /// If this behaviour is unacceptable, use [`wait_nopanic`](Self::wait_nopanic).
    /// 
    /// # Example
    /// ```
    /// # use promisery::Promise;
    /// let p = Promise::<_, ()>::new(|| Ok(1));
    /// assert_eq!(p.wait(), Ok(1));
    /// let p2 = Promise::<(), _>::new(|| Err("fail"));
    /// assert_eq!(p2.wait(), Err("fail"));
    /// ```
    pub fn wait(self) -> Result<T, E> {
        self.wait_nopanic().unwrap()
    }

    /// Fully identical to [`wait`](Self::wait), except it will never panic.
    /// 
    /// # Utility
    /// This can be useful when testing methods that can potentially panic.
    /// 
    /// # Example
    /// ```
    /// # use promisery::{Promise, PromisePanic};
    /// let p = Promise::<(), ()>::new(|| panic!());
    /// assert_eq!(p.wait_nopanic(), Err(PromisePanic));
    /// // This does make it a little more complicated to extract values.
    /// let p2 = Promise::<_, ()>::new(|| Ok(1));
    /// assert_eq!(p2.wait_nopanic(), Ok(Ok(1)));
    /// ```
    pub fn wait_nopanic(self) -> Result<Result<T, E>, PromisePanic> {
        self.rx.recv().map_err(|_| PromisePanic)
    }

    /// Chains a function to be called after this promise resolves, returning a new promise.
    ///
    /// The callback receives the `Result` of this promise and must return a new `Result`.
    /// This allows for both value and error transformation.
    /// 
    /// If any previous method given to this [`Promise`] has panicked, this method will do nothing.
    ///
    /// # Examples
    /// ## Chaining and transforming values:
    /// ```
    /// # use promisery::Promise;
    /// let p = Promise::<_, ()>::new(|| Ok(2))
    ///     .then(|res| res.map(|v| v * 10))
    ///     .then(|res| res.map(|v| v + 5));
    /// assert_eq!(p.wait(), Ok(25));
    /// ```
    ///
    /// ## Propagating errors:
    /// ```
    /// # use promisery::Promise;
    /// let p = Promise::<i32, _>::new(|| Err("fail"))
    ///     .then(|res| res.map(|v| v + 1));
    /// assert_eq!(p.wait(), Err("fail"));
    /// ```
    ///
    /// ## Changing error type:
    /// ```
    /// # use promisery::Promise;
    /// let p = Promise::<(), _>::new(|| Err("fail"))
    ///     .then(|res| res.map_err(|e| format!("error: {}", e)));
    /// assert_eq!(p.wait(), Err("error: fail".to_string()));
    /// ```
    pub fn then<T2: Send + 'static, E2: Send + 'static, F: Send + 'static>(
        self,
        callback: F,
    ) -> Promise<T2, E2>
    where
        F: FnOnce(Result<T, E>) -> Result<T2, E2>,
    {
        let (ntx, nrx) = oneshot::channel();
        let Promise { rx, jtx } = self;

        // Simply drop the function if this promise is in a paniced state.
        let _ = jtx.send(Box::new(move || {
            ntx.send(callback(rx.recv().unwrap())).unwrap()
        }));

        Promise { rx: nrx, jtx }
    }

    /// Maps the success value of this promise using the provided callback, returning a new promise.
    ///
    /// This is a convenience wrapper over [`then`](Self::then) for transforming only the `Ok` value.
    /// Errors are passed through unchanged.
    ///
    /// # Examples
    /// ## Basic mapping:
    /// ```
    /// # use promisery::Promise;
    /// let p = Promise::<_, ()>::new(|| Ok(3)).map(|v| Ok(v * 2));
    /// assert_eq!(p.wait(), Ok(6));
    /// ```
    ///
    /// ## Chaining with error propagation:
    /// ```
    /// # use promisery::Promise;
    /// let p = Promise::<_, _>::new(|| Ok(1))
    ///     .map(|v| Ok(v + 1))
    ///     .map(|v| if v > 1 { Err("too big") } else { Ok(v) });
    /// assert_eq!(p.wait(), Err("too big"));
    /// ```
    ///
    /// ## Mapping after an error (will not run):
    /// ```
    /// # use promisery::Promise;
    /// let p = Promise::<i32, _>::new(|| Err("fail")).map(|v| Ok(v + 1));
    /// assert_eq!(p.wait(), Err("fail"));
    /// ```
    pub fn map<T2: Send + 'static, F: Send + 'static>(self, callback: F) -> Promise<T2, E>
    where
        F: FnOnce(T) -> Result<T2, E>,
    {
        self.then(|res| match res {
            Ok(ok) => callback(ok),
            Err(err) => Err(err),
        })
    }

    /// Maps the error value of this promise using the provided callback, returning a new promise.
    ///
    /// This is a convenience wrapper over [`then`](Self::then) for transforming only the `Err` value.
    /// Success values are passed through unchanged.
    ///
    /// # Examples
    /// ## Basic error mapping:
    /// ```
    /// # use promisery::Promise;
    /// let p = Promise::<(), _>::new(|| Err("fail"))
    ///     .map_err(|e| Err(format!("Promise failed: {}", e)));
    /// assert_eq!(p.wait(), Err("Promise failed: fail".to_string()));
    /// ```
    ///
    /// ## Error mapping after a successful value (will not run):
    /// ```
    /// # use promisery::Promise;
    /// let p = Promise::<_, &str>::new(|| Ok(5)).map_err(|e| Err(format!("err: {}", e)));
    /// assert_eq!(p.wait(), Ok(5));
    /// ```
    ///
    /// ## Chaining error mapping:
    /// ```
    /// # use promisery::Promise;
    /// let p = Promise::<(), &str>::new(|| Err("fail"))
    ///     .map_err(|e| Err(format!("1st: {}", e)))
    ///     .map_err(|e| Err(format!("2nd: {}", e)));
    /// assert_eq!(p.wait(), Err("2nd: 1st: fail".to_string()));
    /// ```
    pub fn map_err<E2: Send + 'static, F: Send + 'static>(self, errback: F) -> Promise<T, E2>
    where
        F: FnOnce(E) -> Result<T, E2>,
    {
        self.then(|res| match res {
            Ok(ok) => Ok(ok),
            Err(err) => errback(err),
        })
    }

    /// Spawns a new promise that executes the given function in a background thread.
    ///
    /// The function should return a `Result<T, E>`. The promise will resolve to this result.
    ///
    /// # Examples
    /// ## Basic usage:
    /// ```
    /// # use promisery::Promise;
    /// let p = Promise::<_, ()>::new(|| Ok(42));
    /// assert_eq!(p.wait(), Ok(42));
    /// ```
    ///
    /// ## With error:
    /// ```
    /// # use promisery::Promise;
    /// let p = Promise::<(), _>::new(|| Err("fail"));
    /// assert_eq!(p.wait(), Err("fail"));
    /// ```
    ///
    /// ## Chaining with map:
    /// ```
    /// # use promisery::Promise;
    /// let p = Promise::<_, ()>::new(|| Ok(10)).map(|v| Ok(v * 2));
    /// assert_eq!(p.wait(), Ok(20));
    /// ```
    pub fn new<F: Send + 'static>(func: F) -> Promise<T, E>
    where
        F: FnOnce() -> Result<T, E>,
    {
        let (tx, rx) = oneshot::channel();
        let (jtx, jrx) = mpsc::channel::<Box<dyn FnOnce() + Send + 'static>>();

        thread::spawn(move || {
            jrx.into_iter().for_each(|f| f());
        });

        // Safety: it is not possible for this promise to panic before being sent a job.
        // Thusly the receiver *must* be alive and able to receive this message.
        jtx.send(Box::new(move || {
            tx.send(func()).unwrap();
        })).unwrap();

        Promise { rx, jtx }
    }

    /// Returns a promise that resolves or rejects with the result of the first promise to complete from the given vector.
    ///
    /// Returns `None` if the input vector is empty.
    ///
    /// # Examples
    /// ## Basic race:
    /// ```
    /// # use promisery::Promise;
    /// # use std::thread::sleep;
    /// # use std::time::Duration;
    /// let p = Promise::race(vec![
    ///     Promise::new(|| { sleep(Duration::from_millis(50)); Ok(1) }),
    ///     Promise::<_, ()>::new(|| Ok(2)),
    /// ]).unwrap();
    /// assert_eq!(p.wait(), Ok(2));
    /// ```
    ///
    /// ## Race with error:
    /// ```
    /// # use promisery::Promise;
    /// # use std::thread::sleep;
    /// # use std::time::Duration;
    /// let p = Promise::race(vec![
    ///     Promise::new(|| { sleep(Duration::from_millis(50)); Ok(()) }),
    ///     Promise::new(|| Err("fail")),
    /// ]).unwrap();
    /// assert_eq!(p.wait(), Err("fail"));
    /// ```
    /// 
    /// ## Empty race:
    /// ```
    /// # use promisery::Promise;
    /// assert!(Promise::<(), ()>::race(Vec::new()).is_none());
    /// ```
    pub fn race(mut promises: Vec<Promise<T, E>>) -> Option<Promise<T, E>> {
        if promises.is_empty() {
            None
        } else {
            Some(Self::new(move || loop {
                if let Some(promise) = promises.extract_if(.., |p| p.fulfilled()).next() {
                    break promise.wait();
                }
            }))
        }
    }

    /// Returns a promise that resolves when all input promises resolve, or rejects if any promise rejects.
    /// This method does not spawn a new promise, instead opting to hijack the last promise in the vector 
    /// and just [`then`](Self::then) all the logic required to reap the other promises.
    ///
    /// The output vector preserves the order of the input promises.
    /// If the input is empty, [`wait`](Self::wait) will produce an empty vector.
    ///
    /// # Examples
    /// ## All succeed:
    /// ```
    /// # use promisery::Promise;
    /// let ps: Vec<_> = (0..3).map(|i| Promise::<_, ()>::new(move || Ok(i))).collect();
    /// let all = Promise::all(ps);
    /// assert_eq!(all.wait(), Ok(vec![0, 1, 2]));
    /// ```
    ///
    /// ## With an error:
    /// ```
    /// # use promisery::Promise;
    /// let ps: Vec<_> = vec![
    ///     Promise::new(|| Ok(1)),
    ///     Promise::new(|| Err("fail")),
    ///     Promise::new(|| Ok(3)),
    /// ];
    /// let all = Promise::all(ps);
    /// assert_eq!(all.wait(), Err("fail"));
    /// ```
    ///
    /// ## Empty input:
    /// ```
    /// # use promisery::Promise;
    /// let all = Promise::<(), ()>::all(vec![]);
    /// assert_eq!(all.wait(), Ok(vec![]));
    /// ```
    pub fn all(mut promises: Vec<Promise<T, E>>) -> Promise<Vec<T>, E> {
        // This promise *must* resolve after the last promise (it must resovle after every promise)
        // in the vector so it is feasible to simply hijack that promise's job queue
        // instead of spawning a new promise and its associated context.
        if let Some(hijacked) = promises.pop() {
            hijacked.then(move |res| {
                promises
                    .into_iter()
                    .map(|p| p.wait())
                    .chain(std::iter::once(res))
                    .collect::<Result<Vec<_>, _>>()
            })
        } else {
            // Cannot hijack the promise since no promises exist
            Promise::new(|| Ok(Vec::new()))
        }
    }

    /// Creates a promise that immediately resolves to the given value.
    ///
    /// # Examples
    /// ```
    /// # use promisery::Promise;
    /// let p = Promise::<_, ()>::resolve(123);
    /// assert_eq!(p.wait(), Ok(123));
    /// ```
    pub fn resolve(val: T) -> Promise<T, E> {
        Self::from_result(Ok(val))
    }

    /// Creates a promise that immediately rejects with the given error.
    ///
    /// # Examples
    /// ```
    /// # use promisery::Promise;
    /// let p = Promise::<(), _>::reject("fail");
    /// assert_eq!(p.wait(), Err("fail"));
    /// ```
    pub fn reject(val: E) -> Promise<T, E> {
        Self::from_result(Err(val))
    }

    /// Creates a promise that immediately resolves to the given result.
    ///
    /// # Examples
    /// ```
    /// # use promisery::Promise;
    /// let p = Promise::<_, ()>::from_result(Ok(1));
    /// assert_eq!(p.wait(), Ok(1));
    /// let p = Promise::<(), _>::from_result(Err("fail"));
    /// assert_eq!(p.wait(), Err("fail"));
    /// ```
    pub fn from_result(result: Result<T, E>) -> Promise<T, E> {
        Promise::new(move || result)
    }
}

impl<T: Send + 'static, E: Send + 'static> From<Result<T, E>> for Promise<T, E> {
    fn from(result: Result<T, E>) -> Self {
        Promise::from_result(result)
    }
}