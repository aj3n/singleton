#![cfg(test)]
use std::time::Duration;

use local_set::{spawn_local, LocalSet};

#[tokio::test]
async fn foreign_waker() {
	let (tx, mut rx) = tokio::sync::mpsc::channel(100);
	let j = std::thread::spawn(move || {
		for _ in 0..10 {
			for i in 0..100 {
				tx.try_send(i).unwrap();
			}
			std::thread::sleep(Duration::from_millis(10));
		}
	});

	let mut local_set = LocalSet::default();
	local_set
		.run_until(async move {
			let j = spawn_local(async move { while rx.recv().await.is_some() {} });
			j.await
		})
		.await;

	j.join().unwrap();
}
