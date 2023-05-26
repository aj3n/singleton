use core::sync::atomic::AtomicU32;
use std::thread_local;

static ID: AtomicU32 = AtomicU32::new(0);

thread_local! {
	static THREAD_ID: u32 = ID.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
}

pub fn get() -> u32 { THREAD_ID.with(|n| *n) }
