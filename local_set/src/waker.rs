use alloc::{collections::VecDeque, sync::Arc};
use core::{
	cell::RefCell,
	marker::PhantomData,
	mem::ManuallyDrop,
	ops::Deref,
	sync::atomic::{AtomicBool, Ordering},
	task::{self, RawWaker, RawWakerVTable},
};

use std::sync::{mpsc, Mutex};

use crate::shared_rc;

//use crate::local_cell;

pub(crate) struct Waker {
	pub(crate) waker: Arc<CachedWaker>,

	pub(crate) id: usize,
}

impl Waker {
	fn wake(&self) { self.waker.wake_by_id(self.id) }
}

pub(crate) fn waker_ref(w: &Arc<Waker>) -> WakerRef<'_> {
	let ptr = Arc::as_ptr(w) as *const ();

	let waker = unsafe { core::task::Waker::from_raw(RawWaker::new(ptr, waker_vtable())) };

	WakerRef {
		waker: ManuallyDrop::new(waker),
		_p: PhantomData,
	}
}
fn waker_vtable() -> &'static RawWakerVTable {
	&RawWakerVTable::new(
		clone_arc_raw,
		wake_arc_raw,
		wake_by_ref_arc_raw,
		drop_arc_raw,
	)
}

unsafe fn inc_ref_count<T>(data: *const ()) {
	// Retain Arc, but don't touch refcount by wrapping in ManuallyDrop
	let arc = ManuallyDrop::new(Arc::<T>::from_raw(data as *const T));

	// Now increase refcount, but don't drop new refcount either
	let _arc_clone: ManuallyDrop<_> = arc.clone();
}

unsafe fn clone_arc_raw(data: *const ()) -> RawWaker {
	inc_ref_count::<Waker>(data);
	RawWaker::new(data, waker_vtable())
}

unsafe fn wake_arc_raw(data: *const ()) {
	let arc: Arc<Waker> = Arc::from_raw(data as *const Waker);
	arc.wake()
}

// used by `waker_ref`
unsafe fn wake_by_ref_arc_raw(data: *const ()) {
	let arc = ManuallyDrop::new(Arc::<Waker>::from_raw(data as *const Waker));
	arc.wake()
}

unsafe fn drop_arc_raw(data: *const ()) { drop(Arc::<Waker>::from_raw(data.cast::<Waker>())) }

pub(crate) struct WakerRef<'a> {
	waker: ManuallyDrop<core::task::Waker>,
	_p: PhantomData<&'a ()>,
}

impl Deref for WakerRef<'_> {
	type Target = core::task::Waker;

	fn deref(&self) -> &core::task::Waker { &self.waker }
}

fn noop_waker() -> task::Waker { unsafe { task::Waker::from_raw(noop_waker_raw()) } }
fn noop_waker_raw() -> RawWaker { RawWaker::new(&() as _, noop_vtable()) }

fn noop_vtable() -> &'static RawWakerVTable {
	&RawWakerVTable::new(|_| noop_waker_raw(), do_nothing, do_nothing, do_nothing)
}

unsafe fn do_nothing(_: *const ()) {}

pub(crate) struct CachedWaker {
	local_waker: shared_rc::Weak<RefCell<VecDeque<usize>>>,
	foreign_waker: mpsc::Sender<usize>,

	waked: AtomicBool,
	// TODO: no Mutex
	waker: Mutex<task::Waker>,
}

impl CachedWaker {
	pub(crate) fn new(
		local_waker: shared_rc::Weak<RefCell<VecDeque<usize>>>,
		foreign_waker: mpsc::Sender<usize>,
	) -> Self {
		Self {
			local_waker,
			foreign_waker,
			waked: AtomicBool::new(false),
			waker: Mutex::new(noop_waker()),
		}
	}

	pub(crate) fn wake_local(&self, id: usize) -> Result<(), ()> {
		self.local_waker
			.upgrade()
			.ok_or(())?
			.borrow_mut()
			.push_back(id);
		Ok(())
	}

	fn wake_by_id(&self, id: usize) {
		if let Some(task_queue) = self.local_waker.upgrade() {
			task_queue.borrow_mut().push_back(id);
		} else {
			let _ = self.foreign_waker.send(id);
		}
		if !self.waked.swap(true, Ordering::Relaxed) {
			self.waker.lock().unwrap().wake_by_ref();
		}
	}

	pub(crate) fn reset(&self, waker: &task::Waker) {
		if let Ok(mut waker_lock) = self.waker.lock() {
			if !waker_lock.will_wake(waker) {
				*waker_lock = waker.clone();
			}
		} else {
			//XXX: waker panicked
		}
		self.waked.store(false, Ordering::Relaxed);
	}
}
