#![doc(html_root_url = "https://docs.rs/promise/1.0.0")]
//!
//! # Promises for Rust
//!
//! This crate provides a JavaScript-inspired, ergonomic, and composable Promise type for Rust, supporting background work,
//! chaining, and error handling with `Result`.
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
//! assert_eq!(p.wait(), Ok(Ok(25)));
//! ```
//!
//! ## Error Handling
//! All errors are handled via `Result<T, E>`. Panics in promise tasks are reported via [`PromisePanic`] (see [`Promise::wait`]).
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
    time::{Duration, Instant},
};

use oneshot::RecvTimeoutError;

#[derive(Debug, PartialEq, Clone, Copy)]
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
/// assert_eq!(p.wait(), Err(PromisePanic));
/// ```
pub struct PromisePanic;

/// Error type returned when waiting on a promise with a timeout.
///
/// This enum indicates whether the wait operation timed out or the promise panicked.
pub enum WaitTimeoutError<T: Send + 'static, E: Send + 'static> {
    /// Indicates the promise panicked during execution.
    Panic(PromisePanic),
    /// Indicates the wait operation timed out and returns the original promise.
    Timeout(Promise<T, E>),
}

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
/// To use the result of a fulfilled promise, attach another promise to it (e.g., via [`then`](Self::then)).
///
/// # Error Handling
/// Unlike JavaScript, success or failure is indicated by a `Result<T, E>`, allowing ergonomic use of Rust's error handling idioms.
///
/// # Panics
/// If a function passed to a promise, through any of the methods which allow such actions (e.g. [`new`](Self::new), [`then`](Self::then))
/// panics, then the promise enters a special state as described by [`poll`](Self::poll). Such promises will not continue to evaluate any functions.
/// When [`wait`](Self::wait)ing on a promise, you need to account for the possibility that the promise has panicked by manually `.unwrap()`ing.
///
/// If you can guarantee that your [`Promise`] will not panic, you can use [`wait_nopanic`](Self::wait_nopanic).
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
    /// Returns an error, specifically a [`PromisePanic`], if this promise has panicked.
    ///
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
    /// This method will never panic. If the promise's background task panicked, returns `Err(PromisePanic)`.
    /// Otherwise, returns `Ok(Ok(value))` or `Ok(Err(error))` as appropriate.
    ///
    /// # Example
    /// ```
    /// # use promisery::{Promise, PromisePanic};
    /// let p = Promise::<_, ()>::new(|| Ok(1));
    /// assert_eq!(p.wait(), Ok(Ok(1)));
    /// let p2 = Promise::<(), _>::new(|| Err("fail"));
    /// assert_eq!(p2.wait(), Ok(Err("fail")));
    /// let p3 = Promise::<(), ()>::new(|| panic!());
    /// assert_eq!(p3.wait(), Err(PromisePanic));
    /// ```
    pub fn wait(self) -> Result<Result<T, E>, PromisePanic> {
        self.rx.recv().map_err(|_| PromisePanic)
    }

    /// Blocks until a `Result` is ready and returns it, panicking if the promise's background task panicked.
    ///
    /// This method will panic if the background task panicked, otherwise it returns the result directly.
    ///
    /// # Panics
    /// Panics if the background thread panicked.
    ///
    /// # Example
    /// ```
    /// # use promisery::Promise;
    /// let p = Promise::<_, ()>::new(|| Ok(1));
    /// assert_eq!(p.wait_nopanic(), Ok(1));
    /// ```
    pub fn wait_nopanic(self) -> Result<T, E> {
        self.rx.recv().unwrap()
    }

    /// Blocks until a `Result` is ready or the specified timeout elapses.
    ///
    /// Returns `Ok(Ok(value))` or `Ok(Err(error))` if the promise resolves before the timeout,
    /// `Err(WaitTimeoutError::Timeout(self))` if the timeout is reached,
    /// or `Err(WaitTimeoutError::Panic(PromisePanic))` if the background task panicked.
    ///
    /// # Example
    /// ```
    /// # use promisery::{Promise, WaitTimeoutError};
    /// # use std::time::Duration;
    /// let p = Promise::<_, ()>::new(|| {
    ///     std::thread::sleep(Duration::from_millis(100));
    ///     Ok(42)
    /// });
    /// match p.wait_timeout(Duration::from_millis(10)) {
    ///     Err(WaitTimeoutError::Timeout(_)) => (),
    ///     _ => panic!("Expected timeout"),
    /// }
    /// ```
    pub fn wait_timeout(self, timeout: Duration) -> Result<Result<T, E>, WaitTimeoutError<T, E>> {
        self.rx.recv_timeout(timeout).map_err(|err| match err {
            RecvTimeoutError::Disconnected => WaitTimeoutError::Panic(PromisePanic),
            RecvTimeoutError::Timeout => WaitTimeoutError::Timeout(self),
        })
    }

    /// Blocks until a `Result` is ready or the specified deadline is reached.
    ///
    /// Returns `Ok(Ok(value))` or `Ok(Err(error))` if the promise resolves before the deadline,
    /// `Err(WaitTimeoutError::Timeout(self))` if the deadline is reached,
    /// or `Err(WaitTimeoutError::Panic(PromisePanic))` if the background task panicked.
    ///
    /// # Example
    /// ```
    /// # use promisery::{Promise, WaitTimeoutError};
    /// # use std::time::{Duration, Instant};
    /// let p = Promise::<_, ()>::new(|| {
    ///     std::thread::sleep(Duration::from_millis(100));
    ///     Ok(())
    /// });
    /// let deadline = Instant::now() + Duration::from_millis(10);
    /// match p.wait_deadline(deadline) {
    ///     Err(WaitTimeoutError::Timeout(_)) => (),
    ///     _ => panic!("Expected timeout"),
    /// }
    /// ```
    pub fn wait_deadline(self, deadline: Instant) -> Result<Result<T, E>, WaitTimeoutError<T, E>> {
        self.rx.recv_deadline(deadline).map_err(|err| match err {
            RecvTimeoutError::Disconnected => WaitTimeoutError::Panic(PromisePanic),
            RecvTimeoutError::Timeout => WaitTimeoutError::Timeout(self),
        })
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
    /// assert_eq!(p.wait(), Ok(Ok(25)));
    /// ```
    ///
    /// ## Propagating errors:
    /// ```
    /// # use promisery::Promise;
    /// let p = Promise::<i32, _>::new(|| Err("fail"))
    ///     .then(|res| res.map(|v| v + 1));
    /// assert_eq!(p.wait(), Ok(Err("fail")));
    /// ```
    ///
    /// ## Changing error type:
    /// ```
    /// # use promisery::Promise;
    /// let p = Promise::<(), _>::new(|| Err("fail"))
    ///     .then(|res| res.map_err(|e| format!("error: {}", e)));
    /// assert_eq!(p.wait(), Ok(Err("error: fail".to_string())));
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
    /// assert_eq!(p.wait(), Ok(Ok(6)));
    /// ```
    ///
    /// ## Chaining with error propagation:
    /// ```
    /// # use promisery::Promise;
    /// let p = Promise::<_, _>::new(|| Ok(1))
    ///     .map(|v| Ok(v + 1))
    ///     .map(|v| if v > 1 { Err("too big") } else { Ok(v) });
    /// assert_eq!(p.wait(), Ok(Err("too big")));
    /// ```
    ///
    /// ## Mapping after an error (will not run):
    /// ```
    /// # use promisery::Promise;
    /// let p = Promise::<i32, _>::new(|| Err("fail")).map(|v| Ok(v + 1));
    /// assert_eq!(p.wait(), Ok(Err("fail")));
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
    /// assert_eq!(p.wait(), Ok(Err("Promise failed: fail".to_string())));
    /// ```
    ///
    /// ## Error mapping after a successful value (will not run):
    /// ```
    /// # use promisery::Promise;
    /// let p = Promise::<_, &str>::new(|| Ok(5)).map_err(|e| Err(format!("err: {}", e)));
    /// assert_eq!(p.wait(), Ok(Ok(5)));
    /// ```
    ///
    /// ## Chaining error mapping:
    /// ```
    /// # use promisery::Promise;
    /// let p = Promise::<(), &str>::new(|| Err("fail"))
    ///     .map_err(|e| Err(format!("1st: {}", e)))
    ///     .map_err(|e| Err(format!("2nd: {}", e)));
    /// assert_eq!(p.wait(), Ok(Err("2nd: 1st: fail".to_string())));
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
    /// assert_eq!(p.wait(), Ok(Ok(42)));
    /// ```
    ///
    /// ## With error:
    /// ```
    /// # use promisery::Promise;
    /// let p = Promise::<(), _>::new(|| Err("fail"));
    /// assert_eq!(p.wait(), Ok(Err("fail")));
    /// ```
    ///
    /// ## Chaining with map:
    /// ```
    /// # use promisery::Promise;
    /// let p = Promise::<_, ()>::new(|| Ok(10)).map(|v| Ok(v * 2));
    /// assert_eq!(p.wait(), Ok(Ok(20)));
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
        }))
        .unwrap();

        Promise { rx, jtx }
    }

    /// Returns a promise that resolves or rejects with the result of the first promise to complete from the given vector
    /// or `None` if the input vector is empty. If every promise passed to this method panics, the returned promise
    /// will also be in a panicked state. Without this propogation, the returned promise would busy-wait forever.
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
    /// assert_eq!(p.wait(), Ok(Ok(2)));
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
    /// assert_eq!(p.wait(), Ok(Err("fail")));
    /// ```
    ///
    /// ## Empty race:
    /// ```
    /// # use promisery::Promise;
    /// let opt_p = Promise::<(), ()>::race(vec![]);
    /// assert!(opt_p.is_none());
    /// ```
    ///
    /// ## Panicked race:
    /// ```
    /// # use promisery::{Promise, PromisePanic};
    /// # use std::thread::sleep;
    /// # use std::time::Duration;
    /// let p = Promise::race(vec![
    ///     Promise::<(), ()>::new(|| panic!()),
    ///     Promise::new(|| panic!()),
    /// ]).unwrap();
    /// assert_eq!(p.wait(), Err(PromisePanic));
    /// ```
    pub fn race(mut promises: Vec<Promise<T, E>>) -> Option<Promise<T, E>> {
        if promises.is_empty() {
            None
        } else {
            Some(Self::new(move || loop {
                if let Some(promise) = promises.extract_if(.., |p| p.fulfilled()).next() {
                    // Safety: p.fulfilled being true guarantees we do not panic.
                    break promise.wait().unwrap();
                }
                // Drop all panicked promises
                let _ = promises
                    .extract_if(.., |p| p.panicked())
                    .collect::<Vec<_>>();
                // If every promise has panicked, propogate the panic.
                if promises.is_empty() {
                    panic!();
                }
            }))
        }
    }

    /// Returns a promise that resolves when all input promises resolve, or rejects if any promise rejects.
    /// This method *will always* produce a new thread context which [`wait`](Self::wait)s on all the input
    /// promises, pruning any that panic.
    ///
    /// The output vector preserves the order of the input promises.
    /// If the input is empty, [`wait`](Self::wait) will produce an empty vector.
    ///
    /// # Panic safety:
    /// If an input promise enters a panicked state it will be skipped
    ///
    /// # Examples
    /// ## All succeed:
    /// ```
    /// # use promisery::Promise;
    /// let ps: Vec<_> = (0..3).map(|i| Promise::<_, ()>::new(move || Ok(i))).collect();
    /// let all = Promise::all(ps);
    /// assert_eq!(all.wait(), Ok(Ok(vec![0, 1, 2])));
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
    /// assert_eq!(all.wait(), Ok(Err("fail")));
    /// ```
    ///
    /// ## Empty input:
    /// ```
    /// # use promisery::Promise;
    /// let all = Promise::<(), ()>::all(vec![]);
    /// assert_eq!(all.wait(), Ok(Ok(vec![])));
    /// ```
    ///
    /// ## With a panic:
    /// ```
    /// # use promisery::Promise;
    /// let ps: Vec<_> = vec![
    ///     Promise::<_, ()>::new(|| Ok(1)),
    ///     Promise::new(|| panic!()),
    ///     Promise::new(|| Ok(3)),
    /// ];
    /// let all = Promise::all(ps);
    /// assert_eq!(all.wait(), Ok(Ok(vec![1, 3])));
    /// ```
    pub fn all(promises: Vec<Promise<T, E>>) -> Promise<Vec<T>, E> {
        Promise::new(move || {
            promises
                .into_iter()
                .filter_map(|p| p.wait().ok())
                .collect::<Result<Vec<_>, _>>()
        })
    }

    /// Creates a promise that immediately resolves to the given value.
    ///
    /// # Examples
    /// ```
    /// # use promisery::Promise;
    /// let p = Promise::<_, ()>::resolve(123);
    /// assert_eq!(p.wait(), Ok(Ok(123)));
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
    /// assert_eq!(p.wait(), Ok(Err("fail")));
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
    /// assert_eq!(p.wait(), Ok(Ok(1)));
    /// let p = Promise::<(), _>::from_result(Err("fail"));
    /// assert_eq!(p.wait(), Ok(Err("fail")));
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
