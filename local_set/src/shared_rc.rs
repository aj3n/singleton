use alloc::{
	alloc::{Allocator, Global, Layout},
	boxed::Box,
};
use core::{
	cell::Cell,
	marker::{PhantomData, Unsize},
	ops::Deref,
	ptr::{self, NonNull},
	sync::atomic::{self, Ordering},
	usize,
};

/// # README
///
/// A `std::rc::Rc` fork , which is different from `std::rc::Rc<T>` in the following aspects:
///
/// - `Weak<T> : Sync + Send` no matter `T: !Sync` or `T: !Send`
/// - `Weak<T>::upgrade` will return `Some(Rc<T>)` if and only if `Weak<T>::upgrade` is called in the same thread `Rc<T>` constructed.
struct RcInner<T: ?Sized> {
	thread_id: u32,

	strong: Cell<usize>,

	weak: atomic::AtomicUsize,

	data: T,
}

pub struct Rc<T: ?Sized> {
	ptr: NonNull<RcInner<T>>,
	_marker: PhantomData<RcInner<T>>,
}

pub struct Weak<T: ?Sized> {
	ptr: NonNull<RcInner<T>>,
}

unsafe impl<T: ?Sized> Send for Weak<T> {}
unsafe impl<T: ?Sized> Sync for Weak<T> {}

impl<T: ?Sized> Weak<T> {}

impl<T: ?Sized> Drop for Weak<T> {
	fn drop(&mut self) {
		unsafe {
			let inner = self.ptr.as_ref();
			if inner.weak.fetch_sub(1, Ordering::Release) == 1 {
				// XXX: acquire?
				Global.deallocate(self.ptr.cast(), Layout::for_value_raw(self.ptr.as_ptr()))
			}
		}
	}
}

impl<T: ?Sized> Weak<T> {
	pub fn upgrade(&self) -> Option<Rc<T>> {
		unsafe {
			let inner = self.ptr.as_ref();
			if inner.thread_id != crate::thread_id::get() {
				return None;
			}

			let strong = inner.strong.get();
			if strong == 0 {
				return None;
			}
			inner.strong.set(strong + 1);

			Some(Rc {
				ptr: self.ptr,
				_marker: PhantomData,
			})
		}
	}
}

impl<T: ?Sized> Rc<T> {
	unsafe fn from_inner(ptr: NonNull<RcInner<T>>) -> Self {
		Self {
			ptr,
			_marker: PhantomData,
		}
	}

	fn inner(&self) -> &RcInner<T> { unsafe { self.ptr.as_ref() } }

	pub fn downgrade(this: &Self) -> Weak<T> {
		let mut cur = this.inner().weak.load(Ordering::Relaxed);
		loop {
			match this.inner().weak.compare_exchange_weak(
				cur,
				cur + 1,
				Ordering::Acquire,
				Ordering::Relaxed,
			) {
				Ok(_) => return Weak { ptr: this.ptr },
				Err(old) => cur = old,
			}
		}
	}

	pub fn unsize<U: ?Sized>(self) -> Rc<U>
	where
		T: Unsize<U>,
	{
		let ptr = self.ptr;
		core::mem::forget(self);
		Rc {
			ptr,
			_marker: PhantomData,
		}
	}
}

impl<T> Rc<T> {
	pub fn new(data: T) -> Self {
		let x = Box::new(RcInner {
			strong: Cell::new(1),
			thread_id: crate::thread_id::get(),
			weak: atomic::AtomicUsize::new(1),
			data,
		});

		unsafe { Self::from_inner(Box::leak(x).into()) }
	}
}

impl<T: ?Sized> Deref for Rc<T> {
	type Target = T;

	fn deref(&self) -> &Self::Target { &self.inner().data }
}
impl<T: ?Sized> AsRef<T> for Rc<T> {
	fn as_ref(&self) -> &T { self }
}

impl<T: ?Sized> Drop for Rc<T> {
	#[inline]
	fn drop(&mut self) {
		let strong = self.inner().strong.get();

		if self.inner().strong.replace(strong - 1) != 1 {
			return;
		}

		// XXX: acquire?
		unsafe {
			ptr::drop_in_place(&mut (*self.ptr.as_ptr()).data);

			drop(Weak { ptr: self.ptr })
		}
	}
}

#[cfg(test)]
mod test {
	use super::*;
	use alloc::string::{String, ToString};
	#[test]
	fn test_rc() {
		let arr = Rc::new([
			"1".to_string(),
			"2".to_string(),
			"3".to_string(),
			"4".to_string(),
		]);
		let slc: Rc<[String]> = arr.unsize();
		assert_eq!(slc.len(), 4);
		let slc2 = Rc::downgrade(&slc).upgrade().unwrap();
		assert_eq!(slc2.len(), 4);
		for (s, i) in slc.iter().zip(1..) {
			assert_eq!(s, &i.to_string());
		}
	}
}
