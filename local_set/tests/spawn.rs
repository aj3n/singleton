#![cfg(test)]
use core::future::ready;
use std::time::Instant;

use futures::task::LocalSpawnExt;
use local_set::{spawn_local, LocalSet};

#[test]
fn spawn_test() {
	futures::executor::block_on(async {
		let mut local_set = LocalSet::default();
		local_set
			.run_until(async {
				let mut handles: Vec<_> = (0..10).map(|i| spawn_local(ready(i))).collect();

				handles.push(spawn_local(async { spawn_local(ready(20)).await }));

				for j in handles {
					dbg!(j.await);
				}
			})
			.await;
	});
}

#[test]
fn heavy_spawn() {
	futures::executor::block_on(async {
		let task_cnt = 100_000;

		{
			let mut local_set = LocalSet::default();
			let now = Instant::now();
			local_set.run_until(singleton_spawn_recure(task_cnt)).await;
			println!("singleton: {}", now.elapsed().as_millis());
		}

		/*{
			let local_set = tokio::task::LocalSet::default();
			let now = Instant::now();
			local_set.run_until(tokio_spawn_recure(task_cnt)).await;
			println!("tokio: {}", now.elapsed().as_millis());
		}*/

		/*{
			let mut pool = futures::executor::LocalPool::new();
			let now = Instant::now();
			let spawner = pool.spawner();
			SPAWNER.set(&spawner, || {
				pool.run_until(futures_spawn_recure(task_cnt));
			});
			println!("futures: {}", now.elapsed().as_millis());
		}*/
	});
}

#[test]
fn heavy_spawn_miri() {
	futures::executor::block_on(LocalSet::default().run_until(singleton_spawn_recure(100)));
	{
		let mut pool = futures::executor::LocalPool::new();
		let spawner = pool.spawner();
		SPAWNER.set(&spawner, || {
			pool.run_until(futures_spawn_recure(100));
		});
	}
}

scoped_tls::scoped_thread_local!(static SPAWNER: futures::executor::LocalSpawner);
async fn futures_spawn_recure(cnt: i32) {
	if cnt > 0 {
		SPAWNER
			.with(|spawner| {
				spawner
					.spawn_local_with_handle(async move { futures_spawn_recure(cnt - 1).await })
					.unwrap()
			})
			.await
	}
}

/*async fn tokio_spawn_recure(cnt: i32) {
	if cnt > 0 {
		let j = tokio::task::spawn_local(async move { tokio_spawn_recure(cnt - 1).await });
		j.await.unwrap()
	}
}*/

async fn singleton_spawn_recure(cnt: i32) {
	if cnt > 0 {
		let j = local_set::spawn_local(async move { singleton_spawn_recure(cnt - 1).await });
		j.await
	}
}
