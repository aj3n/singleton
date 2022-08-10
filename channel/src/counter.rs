pub(crate) struct Counter<C> {
	senders: usize,
	receivers: usize,
	destroy: bool,
	chan: C,
}

pub(crate) fn new<C>(chan: C) -> (Sender<C>, Receiver<C>) {
	let counter = Box::into_raw(Box::new(Counter {
		senders: 1,
		receivers: 1,
		destroy: false,
		chan,
	}));

	(Sender(counter), Receiver(counter))
}

pub(crate) struct Sender<C>(*mut Counter<C>);

impl<C> Sender<C> {
	pub(crate) fn as_inner(&self) -> &C { unsafe { &(*self.0).chan } }
	fn counter_mut(&mut self) -> &mut Counter<C> { unsafe { &mut *self.0 } }
	pub(crate) fn is_disconnected(&self) -> bool { unsafe { (*self.0).receivers == 0 } }
	pub(crate) fn release<F: FnOnce(&C)>(&mut self, on_disconn: F) {
		let counter = self.counter_mut();
		counter.senders -= 1;
		if counter.senders == 0 {
			on_disconn(&counter.chan);
			if std::mem::replace(&mut counter.destroy, true) {
				unsafe { drop(Box::from_raw(self.0)) }
			}
		}
	}
}

pub(crate) struct Receiver<C>(*mut Counter<C>);

impl<C> Receiver<C> {
	pub(crate) fn as_inner(&self) -> &C { unsafe { &(*self.0).chan } }
	fn counter_mut(&mut self) -> &mut Counter<C> { unsafe { &mut *self.0 } }
	pub(crate) fn is_disconnected(&self) -> bool { unsafe { (*self.0).senders == 0 } }
	pub(crate) fn release<F: FnOnce(&C)>(&mut self, on_disconn: F) {
		let counter = self.counter_mut();
		counter.receivers -= 1;
		if counter.receivers == 0 {
			on_disconn(&counter.chan);
			if std::mem::replace(&mut counter.destroy, true) {
				unsafe { drop(Box::from_raw(self.0)) }
			}
		}
	}
}
