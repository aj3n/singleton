use std::{
	cell::RefCell,
	future::Future,
	mem::MaybeUninit,
	pin::Pin,
	task::{Context, Poll},
};

mod counter;
//mod norefcell;

struct Inner<T> {
	head: usize,
	tail: usize,
	last_recv: bool,
	buffer: Box<[MaybeUninit<T>]>,
	recv_ops: event_listener::Event,
	send_ops: event_listener::Event,
}

impl<T> Inner<T> {
	#[inline]
	fn new(cap: usize) -> Self {
		Self {
			buffer: std::iter::repeat_with(MaybeUninit::uninit)
				.take(cap)
				.collect(),
			head: 0,
			tail: 0,
			last_recv: true,
			recv_ops: event_listener::Event::default(),
			send_ops: event_listener::Event::default(),
		}
	}

	unsafe fn recv(&mut self) -> T {
		debug_assert!(!self.is_empty());

		let v = self.buffer.get_unchecked(self.head).assume_init_read();

		self.head += 1;
		if self.head == self.buffer.len() {
			self.head = 0;
		}
		// notify waiting senders
		self.send_ops.notify(1);

		self.last_recv = true;
		v
	}

	pub(crate) fn poll_send(&mut self, v: &mut Option<T>) -> Poll<()> {
		if self.is_full() {
			Poll::Pending
		} else {
			unsafe { self.send(v.take().unwrap()) };
			Poll::Ready(())
		}
	}

	pub(crate) fn poll_recv(&mut self) -> Poll<T> {
		if self.is_empty() {
			Poll::Pending
		} else {
			unsafe { Poll::Ready(self.recv()) }
		}
	}

	unsafe fn send(&mut self, v: T) {
		debug_assert!(!self.is_full());
		self.buffer.get_unchecked_mut(self.tail).write(v);

		self.tail += 1;
		if self.tail == self.buffer.len() {
			self.tail = 0;
		}
		self.recv_ops.notify(1);
		self.last_recv = false;
	}

	fn disconnect(&mut self) {
		self.recv_ops.notify(usize::MAX);
		self.send_ops.notify(usize::MAX);
	}

	fn is_full(&self) -> bool { self.head == self.tail && !self.last_recv }

	fn is_empty(&self) -> bool { self.head == self.tail && self.last_recv }
}

impl<T> Drop for Inner<T> {
	fn drop(&mut self) {
		if self.is_empty() {
			return;
		}

		if self.head < self.tail {
			for slot in &mut self.buffer[self.head..self.tail] {
				unsafe { slot.assume_init_drop() }
			}
		} else {
			for slot in &mut self.buffer[self.head..] {
				unsafe { slot.assume_init_drop() }
			}
			for slot in &mut self.buffer[..self.tail] {
				unsafe { slot.assume_init_drop() }
			}
		}
	}
}

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
	Disconnected,
}

#[derive(Debug, PartialEq, Eq)]
pub enum TrySendError {
	Full,
	Disconnected,
}

pub struct Sender<T>(counter::Sender<RefCell<Inner<T>>>);

pub struct Send<'a, T> {
	sender: &'a Sender<T>,
	listener: Option<event_listener::EventListener>,
	msg: Option<T>,
}

impl<'a, T> Unpin for Send<'a, T> {}

impl<'a, T> Future for Send<'a, T> {
	type Output = Result<(), Error>;
	fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
		let mut this = Pin::new(self);
		let mut inner_mut = this.sender.0.as_inner().borrow_mut();
		if inner_mut.poll_send(&mut this.msg).is_ready() {
			return Poll::Ready(Ok(()));
		}

		if this.sender.0.is_disconnected() {
			return Poll::Ready(Err(Error::Disconnected));
		}

		let listener = this
			.listener
			.get_or_insert_with(|| inner_mut.send_ops.listen());

		let _ = Pin::new(listener).poll(cx);

		Poll::Pending
	}
}

impl<T> Sender<T> {
	pub fn try_send(&self, v: T) -> Result<(), TrySendError> {
		if self.0.is_disconnected() {
			return Err(TrySendError::Disconnected);
		}
		let mut inner_mut = self.0.as_inner().borrow_mut();
		if inner_mut.is_full() {
			return Err(TrySendError::Full);
		}
		unsafe { inner_mut.send(v) };
		Ok(())
	}

	pub fn send(&self, v: T) -> Send<'_, T> {
		Send {
			sender: self,
			listener: None,
			msg: Some(v),
		}
	}
}

impl<T> Drop for Sender<T> {
	fn drop(&mut self) { self.0.release(|c| c.borrow_mut().disconnect()); }
}

#[derive(Debug)]
pub enum TryRecvError {
	Empty,
	Disconnected,
}

pub struct Receiver<T>(counter::Receiver<RefCell<Inner<T>>>);

pub struct Recv<'a, T> {
	receiver: &'a Receiver<T>,
	listener: Option<event_listener::EventListener>,
}

impl<'a, T> Unpin for Recv<'a, T> {}

impl<'a, T> Future for Recv<'a, T> {
	type Output = Result<T, Error>;

	fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
		let mut this = Pin::new(self);
		let mut inner_mut = this.receiver.0.as_inner().borrow_mut();
		if let Poll::Ready(v) = inner_mut.poll_recv() {
			return Poll::Ready(Ok(v));
		}

		if this.receiver.0.is_disconnected() {
			return Poll::Ready(Err(Error::Disconnected));
		}
		let listener = this
			.listener
			.get_or_insert_with(|| inner_mut.recv_ops.listen());

		let _ = Pin::new(listener).poll(cx);
		Poll::Pending
	}
}

impl<T> Receiver<T> {
	pub fn try_recv(&self) -> Result<T, TryRecvError> {
		if self.0.is_disconnected() {
			return Err(TryRecvError::Disconnected);
		}
		let mut inner_mut = self.0.as_inner().borrow_mut();
		if inner_mut.is_empty() {
			return Err(TryRecvError::Empty);
		};
		Ok(unsafe { inner_mut.recv() })
	}

	pub fn recv(&self) -> Recv<'_, T> {
		Recv {
			receiver: self,
			listener: None,
		}
	}
}

impl<T> Drop for Receiver<T> {
	fn drop(&mut self) { self.0.release(|c| c.borrow_mut().disconnect()); }
}

pub fn bounded<T>(cap: usize) -> (Sender<T>, Receiver<T>) {
	let chan = RefCell::new(Inner::new(cap));
	let (sender_counter, receiver_counter) = counter::new(chan);

	(Sender(sender_counter), Receiver(receiver_counter))
}
