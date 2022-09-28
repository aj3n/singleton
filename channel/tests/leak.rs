use channel::*;
use std::{cell::RefCell, collections::BTreeSet, rc::Rc};

#[test]
fn test_leak() {
	let (tx, rx) = bounded(100);
	let trash_bin = Rc::new(RefCell::new(BTreeSet::new()));

	for i in 0..10 {
		tx.try_send(Foo(i, trash_bin.clone())).unwrap();
	}

	let black_hole = Rc::new(RefCell::new(BTreeSet::new()));
	for _ in 0..5 {
		rx.try_recv().unwrap().1 = black_hole.clone();
	}

	drop(tx);
	drop(rx);

	assert_eq!(&*trash_bin.borrow_mut(), &(5..10).collect());

	struct Foo(i32, Rc<RefCell<BTreeSet<i32>>>);
	impl Drop for Foo {
		fn drop(&mut self) { self.1.borrow_mut().insert(self.0); }
	}
}
