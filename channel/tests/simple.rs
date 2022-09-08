use channel::*;
use std::{cell::RefCell, collections::BTreeSet, rc::Rc};

#[tokio::test]
async fn it_works() {
	let len = 100;
	test_send(len, len).await;
	// XXX: terrible performance
	test_send(1, len).await;
}

async fn test_send(cap: usize, len: usize) {
	tokio::task::LocalSet::new()
		.run_until(async {
			//let now = std::time::Instant::now();
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
			//println!("time cost: {}", now.elapsed().as_millis());

			// flume
			//let now = std::time::Instant::now();
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
			//println!("flume time cost: {}", now.elapsed().as_millis());
		})
		.await;
}

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

#[test]
fn test_leak() {
	let (tx, rx) = bounded(100);
	let trash_bin = Rc::new(RefCell::new(BTreeSet::new()));

	for i in 0..10 {
		tx.try_send(Foo(i, trash_bin.clone())).unwrap();
	}

	let black_hole = Rc::new(RefCell::new(BTreeSet::new()));
	for _ in 0..5 {
		rx.try_recv().unwrap().1 = black_hole.clone();
	}

	drop(tx);
	drop(rx);

	assert_eq!(&*trash_bin.borrow_mut(), &(5..10).collect());

	struct Foo(i32, Rc<RefCell<BTreeSet<i32>>>);
	impl Drop for Foo {
		fn drop(&mut self) { self.1.borrow_mut().insert(self.0); }
	}
}
