use std::cell::UnsafeCell;

struct NoRefCell<T>(UnsafeCell<T>);

impl<T> NoRefCell<T> {
	fn borrow(&self) -> &T { unsafe { &*self.0.get() } }

	#[allow(clippy::mut_from_ref)]
	fn borrow_mut(&self) -> &mut T { unsafe { &mut *self.0.get() } }
}
