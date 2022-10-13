#![cfg(test)]
use futures::future::ready;
use local_set::{spawn_local, LocalSet};
#[test]
fn spawn_test() {
	let mut local_set = LocalSet::default();
	futures::executor::block_on(local_set.run_until(async {
		let mut handles: Vec<_> = (0..10).map(|i| spawn_local(ready(i))).collect();

		handles.push(spawn_local(async { spawn_local(ready(20)).await }));

		for j in handles {
			dbg!(j.await);
		}
	}));
}
