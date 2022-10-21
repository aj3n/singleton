# local_set

A `tokio::task::LocalSet`-like data structure with runtime-independency;

## FAQ

### When should I use this crate instead of tokio::task::LocalSet

- you're looking for a pure single-thread `LocalSet` without capability of executing foreign tasks.
- you're doing runtime-independent work.

### When should I use this crate instead of futures::stream::{FuturesUnordered, FuturesOrdered}

- your futures' outputs are various.
- you're spawning new futures inside `futures::stream::{FuturesUnordered, FuturesOrdered}`
- you're dealing with pure single threaded situation.

## When shouldn't I use this crate

- If you're using this crate inside `tokio`, thanks to `tokio`'s budget mechanism, you may encounter multiple false `Poll::Pending`. So make sure you wrap `LocalSet` with `tokio::task::unconstraint`(with the risk of starving), or simply use `tokio::task::LocalSet` instead.
- There are some waking notification loss, which may cause the un-waked `futrue`s dead forever.

## Will `LocalSet` eagerly poll spawned task?

- No, LocalSet will and only will poll the futures with waking notification.
