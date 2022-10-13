#[cfg(test)]
mod test {

	struct Tester<T>(std::marker::PhantomData<T>);

	impl<T> Tester<T> {
		fn new(_: &T) -> Self { Self(Default::default()) }
	}

	impl<T: Send> Tester<T> {
		const SEND: bool = true;
	}

	impl<T: Sync> Tester<T> {
		const SYNC: bool = true;
	}

	trait NotSend {
		const SEND: bool = false;

		fn is_send(&self) -> bool { Self::SEND }
	}

	impl<T> NotSend for Tester<T> {}

	trait NotSync {
		const SYNC: bool = false;
		fn is_sync(&self) -> bool { Self::SYNC }
	}

	impl<T> NotSync for Tester<T> {}

	#[allow(clippy::assertions_on_constants)]
	#[test]
	fn non_send_sync() {
		use local_set::*;
		assert!(Tester::<i32>::SEND);
		assert!(Tester::<i32>::SYNC);

		assert!(!Tester::<JoinHandle<i32>>::SEND);
		assert!(!Tester::<JoinHandle<i32>>::SYNC);

		assert!(!Tester::<LocalSet>::SEND);
		assert!(!Tester::<LocalSet>::SYNC);

		let mut s = LocalSet::default();
		let r = Tester::new(&s.run_until(std::future::ready(0)));
		assert!(!r.is_send());
		assert!(!r.is_sync());
	}
}
