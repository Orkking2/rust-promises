# Promisery

[![Crates.io](https://img.shields.io/crates/v/promisery.svg)](https://crates.io/crates/promisery)
[![Docs.rs](https://docs.rs/promisery/badge.svg)](https://docs.rs/promisery)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](./LICENSE)

A JavaScript-inspired, ergonomic, and composable Promise type for Rust, supporting background work, chaining, and error handling with `Result`.

---

## Features
- ECMAScript 5-style promises with Rust's type safety
- Chaining with `.then`, `.map`, and `.map_err`
- Combinators: `Promise::all`, `Promise::race`
- Panic-safe: panics in promise tasks are detected and reported
- Fully documented with tested examples

## Installation
Add to your `Cargo.toml`:
```toml
promisery = "2.0"
```

## Example
```rust
use promisery::Promise;

let p = Promise::<_, ()>::new(|| Ok(2))
    .then(|res| res.map(|v| v * 10))
    .then(|res| res.map(|v| v + 5));
assert_eq!(p.wait(), Ok(Ok(25)));
```

## API Highlights
- **Chaining:**
  - `.then` for full control over result and error
  - `.map` for value transformation
  - `.map_err` for error transformation
- **Combinators:**
  - `Promise::all` waits for all promises (returns `Result<Result<Vec<T>, E>, PromisePanic>`)
  - `Promise::race` resolves/rejects with the first to finish (returns `Result<Result<T, E>, PromisePanic>`)
- **Panic Handling:**
  - Panics in promise tasks are detected and reported via `PromisePanic`
  - `.wait()` never panics, returns a double Result

## Why Promisery?
- Lightweight, no async runtime required
- Familiar API for those coming from JavaScript
- Integrates with Rust's `Result`-based error handling
- Does not spawn a new thread for each action that produces a new promise (see the documentation for each method for more details)

## License
MIT OR Apache-2.0

---

See [docs.rs/promisery](https://docs.rs/promisery) for full documentation and more examples.
