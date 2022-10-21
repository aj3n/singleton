#![cfg(test)]
use std::{
	cell::Cell,
	future::{ready, Future},
	pin::Pin,
	task::{self, Context, Poll, RawWaker, RawWakerVTable},
	time::Instant,
};

use local_set::{spawn_local, LocalSet};
#[tokio::test]
async fn spawn_test() {
	let mut local_set = LocalSet::default();
	local_set
		.run_until(async {
			let mut handles: Vec<_> = (0..10).map(|i| spawn_local(ready(i))).collect();

			handles.push(spawn_local(async { spawn_local(ready(20)).await }));

			for j in handles {
				dbg!(j.await);
			}
		})
		.await
}

fn noop_waker() -> task::Waker { unsafe { task::Waker::from_raw(noop_waker_raw()) } }
fn noop_waker_raw() -> RawWaker { RawWaker::new(&() as _, noop_vtable()) }

fn noop_vtable() -> &'static RawWakerVTable {
	&RawWakerVTable::new(|_| noop_waker_raw(), drop, drop, drop)
}

thread_local! { static C: Cell<i32> = Cell::new(0);}
#[tokio::test]
async fn heavy_spawn() {
	let task_cnt = 100_000;

	{
		let mut local_set = LocalSet::default();
		C.with(|c| c.set(0));
		let now = Instant::now();
		local_set.run_until(singleton_spawn_recure(task_cnt)).await;
		println!("singleton: {}", now.elapsed().as_millis());
	}

	{
		let local_set = tokio::task::LocalSet::default();
		C.with(|c| c.set(0));
		let now = Instant::now();
		local_set.run_until(tokio_spawn_recure(task_cnt)).await;
		println!("tokio: {}", now.elapsed().as_millis());
	}
}

async fn tokio_spawn_recure(cnt: i32) {
	let cur_cnt = C.with(|c| c.get());
	if cur_cnt < cnt {
		C.with(|c| c.set(cur_cnt + 1));

		let j = tokio::task::spawn_local(async move { tokio_spawn_recure(cnt).await });
		j.await.unwrap()
	}
}

async fn singleton_spawn_recure(cnt: i32) {
	let cur_cnt = C.with(|c| c.get());
	if cur_cnt < cnt {
		C.with(|c| c.set(cur_cnt + 1));

		let j = local_set::spawn_local(async move { singleton_spawn_recure(cnt).await });
		j.await
	}
}
