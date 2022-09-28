use channel::*;

#[tokio::test]
async fn it_works() {
	let len = 1_000_000;
	//test_send(len, len).await;
	// XXX: terrible performance
	test_send(1, len).await;
}

async fn test_channel(cap: usize, len: usize) {
	let (tx, rx) = bounded(cap);
	let j = tokio::task::spawn_local(async move {
		let mut it = 0..;
		while let Ok(v) = rx.recv().await {
			assert_eq!(v, it.next().unwrap());
		}
	});
	for i in 0..len {
		tx.send(i).await.unwrap();
	}
	drop(tx);
	j.await.unwrap();
}

async fn test_flume(cap: usize, len: usize) {
	let (tx, rx) = flume::bounded(cap);
	let j = tokio::spawn(async move {
		let mut it = 0..;
		while let Ok(v) = rx.recv_async().await {
			assert_eq!(v, it.next().unwrap());
		}
	});
	for i in 0..len {
		tx.send_async(i).await.unwrap();
	}
	drop(tx);
	j.await.unwrap();
}

async fn test_send(cap: usize, len: usize) {
	tokio::task::LocalSet::new()
		.run_until(async {
			let now = std::time::Instant::now();
			test_channel(cap, len).await;
			println!("channel time cost: {}", now.elapsed().as_millis());

			// flume
			//let now = std::time::Instant::now();
			//test_flume(cap, len).await;
			//println!("flume time cost: {}", now.elapsed().as_millis());
		})
		.await;
}
