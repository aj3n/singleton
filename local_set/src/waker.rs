use std::{
	marker::PhantomData,
	mem::ManuallyDrop,
	ops::Deref,
	sync::{mpsc, Arc},
	task::{RawWaker, RawWakerVTable},
};

use futures::task::AtomicWaker;

use crate::local_cell;

pub(crate) struct Waker {
	pub(crate) local_waker: local_cell::LocalCellRef,
	pub(crate) foreign_waker: mpsc::Sender<usize>,

	pub(crate) waker: Arc<AtomicWaker>,
	pub(crate) id: usize,
}

impl Waker {
	fn wake(&self) {
		if self.local_waker.notify(self.id).is_err() {
			let _ = self.foreign_waker.send(self.id);
		}
		self.waker.wake();
	}
}

pub(crate) fn waker_ref(w: &Arc<Waker>) -> WakerRef<'_> {
	let ptr = Arc::as_ptr(w) as *const ();

	let waker = unsafe { std::task::Waker::from_raw(RawWaker::new(ptr, waker_vtable())) };

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
	waker: ManuallyDrop<std::task::Waker>,
	_p: PhantomData<&'a ()>,
}

impl Deref for WakerRef<'_> {
	type Target = std::task::Waker;

	fn deref(&self) -> &std::task::Waker { &self.waker }
}
