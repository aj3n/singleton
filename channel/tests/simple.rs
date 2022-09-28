use channel::*;

#[test]
fn try_send_tests() {
	let (tx, rx) = bounded(100);
	for i in 0..100 {
		tx.try_send(i).unwrap();
		assert_eq!(rx.try_recv().unwrap(), i);
	}
	tx.try_send(10).unwrap();
	tx.try_send(12).unwrap();
	tx.try_send(11).unwrap();
	drop(rx);
	assert_eq!(tx.try_send(10), Err(TrySendError::Disconnected));
	drop(tx);
}

#[tokio::test]
async fn test_drop_notify() {
	tokio::task::LocalSet::new()
		.run_until(async {
			let (tx, rx) = bounded::<()>(100);
			//let sleep_millis = 100;
			//let now = std::time::Instant::now();
			let j = tokio::task::spawn_local(async move {
				rx.recv().await.unwrap_err();
				//assert_eq!(sleep_millis/ 10, now.elapsed().as_millis() as u64/ 10);
			});

			//tokio::time::sleep(std::time::Duration::from_millis(sleep_millis)).await;
			drop(tx);
			j.await.unwrap();
		})
		.await;
}
