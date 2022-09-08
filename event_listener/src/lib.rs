/// mostly modified from https://github.com/smol-rs/event-listener/blob/a01518f41c5c4acd71fb6a55ef2cc2675310dfca/src/lib.rs
use std::{
	cell::{Cell, RefCell, UnsafeCell},
	future::Future,
	pin::Pin,
	ptr::NonNull,
	rc::Rc,
	task::{Context, Poll, Waker},
};

#[derive(Default)]
pub struct Event {
	inner: UnsafeCell<Option<Rc<Inner>>>,
}

impl Event {
	fn inner(&self) -> &Rc<Inner> {
		unsafe {
			match &mut *self.inner.get() {
				empty @ None => empty.insert(Rc::new(Inner {
					list: RefCell::new(List {
						head: None,
						tail: None,
						start: None,
						len: 0,
						notified: 0,
						cache_used: false,
					}),
					cache: UnsafeCell::new(Entry {
						state: Cell::new(State::Created),
						prev: Cell::new(None),
						next: Cell::new(None),
					}),
				})),
				Some(inner) => inner,
			}
		}
	}

	fn try_inner(&self) -> Option<&Inner> { unsafe { (*self.inner.get()).as_deref() } }

	pub fn listen(&self) -> EventListener {
		let inner = self.inner();

		EventListener {
			inner: inner.clone(),
			entry: Some(inner.list.borrow_mut().insert(inner.cache_ptr())),
		}
	}

	pub fn notify(&self, n: usize) {
		if let Some(inner) = self.try_inner() {
			let mut list = inner.list.borrow_mut();
			if list.notified < n {
				list.notify(n);
			}
		}
	}

	pub fn notify_additional(&self, n: usize) {
		if let Some(inner) = self.try_inner() {
			let mut list = inner.list.borrow_mut();
			if list.notified < n {
				list.notify_additional(n);
			}
		}
	}
}

struct Inner {
	cache: UnsafeCell<Entry>,
	list: RefCell<List>,
}

impl Inner {
	#[inline(always)]
	fn cache_ptr(&self) -> NonNull<Entry> { unsafe { NonNull::new_unchecked(self.cache.get()) } }
}

pub struct EventListener {
	entry: Option<NonNull<Entry>>,
	inner: Rc<Inner>,
}

impl Future for EventListener {
	type Output = ();

	fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
		let entry = match self.entry {
			None => unreachable!("cannot poll a completed `EventListener` future"),
			Some(entry) => entry,
		};
		let state = unsafe { &entry.as_ref().state };

		match state.replace(State::Notified(false)) {
			State::Notified(_) => {
				// If this listener has been notified, remove it from the list and return.
				self.inner
					.list
					.borrow_mut()
					.remove(entry, self.inner.cache_ptr());
				self.entry = None;
				return Poll::Ready(());
			}
			State::Created => {
				// If the listener was just created, put it in the `Polling` state.
				state.set(State::Polling(cx.waker().clone()));
			}
			State::Polling(w) => {
				// If the listener was in the `Polling` state, update the waker.
				if w.will_wake(cx.waker()) {
					state.set(State::Polling(w));
				} else {
					state.set(State::Polling(cx.waker().clone()));
				}
			}
		}
		Poll::Pending
	}
}

impl Drop for EventListener {
	fn drop(&mut self) {
		// If this listener has never picked up a notification...
		if let Some(entry) = self.entry.take() {
			let mut list = self.inner.list.borrow_mut();

			// But if a notification was delivered to it...
			if let State::Notified(additional) = list.remove(entry, self.inner.cache_ptr()) {
				// Then pass it on to another active listener.
				if additional {
					list.notify_additional(1);
				} else {
					list.notify(1);
				}
			}
		}
	}
}

enum State {
	/// It has just been created.
	Created,

	/// It has received a notification.
	///
	/// The `bool` is `true` if this was an "additional" notification.
	Notified(bool),

	/// An async task is polling it.
	Polling(Waker),
}

impl State {
	/// Returns `true` if this is the `Notified` state.
	#[inline]
	fn is_notified(&self) -> bool { matches!(self, Self::Notified(_)) }
}
struct Entry {
	/// THe state of this listener.
	state: Cell<State>,

	/// Previous entry in the linked list.
	prev: Cell<Option<NonNull<Entry>>>,

	/// Next entry in the linked list.
	next: Cell<Option<NonNull<Entry>>>,
}

/// A linked list of entries.
struct List {
	/// First entry in the list.
	head: Option<NonNull<Entry>>,

	/// Last entry in the list.
	tail: Option<NonNull<Entry>>,

	/// The first unnotified entry in the list.
	start: Option<NonNull<Entry>>,

	/// Total number of entries in the list.
	len: usize,

	/// The number of notified entries in the list.
	notified: usize,

	/// Whether the cached entry is used.
	cache_used: bool,
}

impl List {
	/// Inserts a new entry into the list.
	fn insert(&mut self, cache: NonNull<Entry>) -> NonNull<Entry> {
		unsafe {
			let entry = Entry {
				state: Cell::new(State::Created),
				prev: Cell::new(self.tail),
				next: Cell::new(None),
			};

			let entry = if self.cache_used {
				// Allocate an entry that is going to become the new tail.
				NonNull::new_unchecked(Box::into_raw(Box::new(entry)))
			} else {
				// No need to allocate - we can use the cached entry.
				self.cache_used = true;
				cache.as_ptr().write(entry);
				cache
			};

			// Replace the tail with the new entry.
			match std::mem::replace(&mut self.tail, Some(entry)) {
				None => self.head = Some(entry),
				Some(t) => t.as_ref().next.set(Some(entry)),
			}

			// If there were no unnotified entries, this one is the first now.
			if self.start.is_none() {
				self.start = self.tail;
			}

			// Bump the entry count.
			self.len += 1;

			entry
		}
	}

	/// Removes an entry from the list and returns its state.
	fn remove(&mut self, entry: NonNull<Entry>, cache: NonNull<Entry>) -> State {
		unsafe {
			let prev = entry.as_ref().prev.get();
			let next = entry.as_ref().next.get();

			// Unlink from the previous entry.
			match prev {
				None => self.head = next,
				Some(p) => p.as_ref().next.set(next),
			}

			// Unlink from the next entry.
			match next {
				None => self.tail = prev,
				Some(n) => n.as_ref().prev.set(prev),
			}

			// If this was the first unnotified entry, move the pointer to the next one.
			if self.start == Some(entry) {
				self.start = next;
			}

			// Extract the state.
			let state = if std::ptr::eq(entry.as_ptr(), cache.as_ptr()) {
				// Free the cached entry.
				self.cache_used = false;
				entry.as_ref().state.replace(State::Created)
			} else {
				// Deallocate the entry.
				Box::from_raw(entry.as_ptr()).state.into_inner()
			};

			// Update the counters.
			if state.is_notified() {
				self.notified -= 1;
			}
			self.len -= 1;

			state
		}
	}

	/// Notifies a number of entries.
	#[cold]
	fn notify(&mut self, mut n: usize) {
		if n <= self.notified {
			return;
		}
		n -= self.notified;

		for _ in 0..n {
			// Notify the first unnotified entry.
			match self.start {
				None => break,
				Some(e) => {
					// Get the entry and move the pointer forward.
					let e = unsafe { e.as_ref() };
					self.start = e.next.get();

					// Set the state of this entry to `Notified` and notify.
					match e.state.replace(State::Notified(false)) {
						State::Notified(_) => {}
						State::Created => {}
						State::Polling(w) => w.wake(),
						//State::Waiting(t) => t.unpark(),
					}

					// Update the counter.
					self.notified += 1;
				}
			}
		}
	}

	/// Notifies a number of additional entries.
	#[cold]
	fn notify_additional(&mut self, n: usize) {
		for _ in 0..n {
			// Notify the first unnotified entry.
			match self.start {
				None => break,
				Some(e) => {
					// Get the entry and move the pointer forward.
					let e = unsafe { e.as_ref() };
					self.start = e.next.get();

					// Set the state of this entry to `Notified` and notify.
					match e.state.replace(State::Notified(true)) {
						State::Notified(_) => {}
						State::Created => {}
						State::Polling(w) => w.wake(),
						//State::Waiting(t) => t.unpark(),
					}

					// Update the counter.
					self.notified += 1;
				}
			}
		}
	}
}

#[cfg(test)]
mod test {
	// TODO: check `!Send + !Sync` for `Event` | `EventListener`
}
