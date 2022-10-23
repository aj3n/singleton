use slab::Slab;
use std::{
	cell::RefCell,
	collections::{VecDeque},
	sync::atomic::{AtomicU32, Ordering},
};

pub(crate) struct LocalCell {
	thread_id: u32,
	store_id: usize,
}

#[derive(Clone)]
pub(crate) struct LocalCellRef {
	thread_id: u32,
	store_id: usize,
}

impl From<&LocalCell> for LocalCellRef {
	fn from(cell: &LocalCell) -> Self {
		Self {
			thread_id: cell.thread_id,
			store_id: cell.store_id,
		}
	}
}

static MONO_ID: AtomicU32 = AtomicU32::new(0);

thread_local! {
	static THRD_ID: u32 = MONO_ID.fetch_add(1, Ordering::Relaxed);
	static GLOBAL_STORE: RefCell<Slab<VecDeque<usize>>> = RefCell::new(Slab::new());
}

impl LocalCell {
	pub(crate) fn new() -> Self {
		let store_id = GLOBAL_STORE.with(|store| store.borrow_mut().insert(Default::default()));
		Self {
			store_id,
			thread_id: THRD_ID.with(|thread_id| *thread_id),
		}
	}

	pub(crate) fn fetch(&mut self) -> Option<usize> {
		GLOBAL_STORE.with(|store| {
			let queue = &mut store.borrow_mut()[self.store_id];
			queue.pop_front()
		})
	}
}

impl LocalCellRef {
	pub(crate) fn notify(&self, task_id: usize) -> Result<(), ()> {
		if THRD_ID.with(|&id| self.thread_id != id) {
			Err(())
		} else {
			GLOBAL_STORE.with(|store| {
				store
					.borrow_mut()
					.get_mut(self.store_id)
					.map(|queue| queue.push_back(task_id))
					.ok_or(())
			})
		}
	}
}
impl Drop for LocalCell {
	fn drop(&mut self) {
		GLOBAL_STORE.with(|store| {
			let mut store = store.borrow_mut();
			store.remove(self.store_id);
			store.shrink_to_fit();
		})
	}
}
