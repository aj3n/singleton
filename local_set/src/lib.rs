#![feature(allocator_api, unsize, layout_for_ptr)]

use std::{
	cell::{Cell, RefCell},
	collections::VecDeque,
	future::Future,
	pin::Pin,
	rc::Rc,
	sync::{mpsc, Arc},
	task::{self, Poll},
};

use slab::Slab;
use waker::CachedWaker;

mod shared_rc;
mod waker;
//mod scoped;

scoped_tls::scoped_thread_local!(static CURRENT: Rc<Context<>>);
struct Context {
	tasks: RefCell<Slab<Option<TaskWrapper>>>,
	waker: Arc<CachedWaker>,
}

impl Context {
	#[inline]
	fn spawn<F: Future + 'static>(
		self: &Rc<Self>,
		fut: F,
	) -> JoinHandle<<F as Future>::Output> {
		let mut tasks = self.tasks.borrow_mut();
		let id = tasks.insert(None);
		let handle = JoinHandle {
			ctx: Rc::new(JoinHandleCtx {
				output: Cell::new(None),
				waker: Cell::new(None),
				canceled: Cell::new(false),
				id: Cell::new(id),
			}),
			detached: false,
		};
		let task = Box::pin(TaskImpl {
			ctx: handle.ctx.clone(),
			waker: Arc::new(self.waker(id)),
			fut,
		});
		self.waker.wake_local(id).unwrap();
		tasks[id] = Some(task);

		handle
	}

	#[inline]
	fn waker(&self, id: usize) -> waker::Waker {
		waker::Waker {
			waker: self.waker.clone(),
			id,
		}
	}
}

pub struct LocalSet {
	context: Rc<Context>,

	scheduler: Scheduler,
}

struct Scheduler {
	task_queue: shared_rc::Rc<RefCell<VecDeque<usize>>>,
	task_queue_foreign: mpsc::Receiver<usize>,
	tick: u8,
}

impl Scheduler {
	fn fetch(&mut self) -> Option<usize> {
		const FOREIGN_QUEUE_INTERVAL: u8 = 31;
		self.tick = self.tick.wrapping_add(1);
		// FIXME: foreign_waker starvation
		if self.tick % FOREIGN_QUEUE_INTERVAL == 0 {
			self.task_queue
				.borrow_mut()
				.pop_front()
				.or_else(|| self.task_queue_foreign.try_recv().ok())
		} else {
			self.task_queue_foreign
				.try_recv()
				.ok()
				.or_else(|| self.task_queue.borrow_mut().pop_front())
		}
	}
}

impl LocalSet {
	pub async fn run_until<F>(&mut self, fut: F) -> F::Output
	where
		F: Future,
	{
		// take a position without actually storing the future
		// XXX: can't impl Drop for RunUntil, may leak if RunUntil dropped without returning Poll::Ready
		let id = self.context.tasks.borrow_mut().insert(None);
		self.context.waker.wake_local(id).unwrap();
		RunUntil {
			waker: Arc::new(self.context.waker(id)),
			local_set: self,
			fut,
		}
		.await
	}

	fn _block_on<F: Future>(&mut self, _fut: F) -> <F as Future>::Output {
		todo!("should only be called by async executor");
	}

	fn new() -> Self {
		let task_queue = shared_rc::Rc::new(RefCell::new(VecDeque::new()));
		let (tx, rx) = mpsc::channel();
		Self {
			context: Rc::new(Context {
				tasks: RefCell::new(Slab::new()),
				waker: Arc::new(CachedWaker::new(shared_rc::Rc::downgrade(&task_queue), tx)),
			}),
			scheduler: Scheduler {
				task_queue_foreign: rx,
				task_queue,
				tick: 0,
			},
		}
	}
}

impl Default for LocalSet {
	fn default() -> Self { Self::new() }
}

type TaskWrapper = Pin<Box<dyn Task + 'static>>;

struct JoinHandleCtx<T> {
	output: Cell<Option<T>>,
	waker: Cell<Option<task::Waker>>,
	id: Cell<usize>,
	canceled: Cell<bool>,
}

pub struct JoinHandle<T> {
	ctx: Rc<JoinHandleCtx<T>>,
	detached: bool,
}

impl<T: Unpin> Unpin for JoinHandle<T> {}

trait Task {
	fn poll(self: Pin<&mut Self>) -> Poll<()>;
	fn canceled(self: Pin<&Self>) -> bool;
}

#[pin_project::pin_project]
struct TaskImpl<F: Future> {
	ctx: Rc<JoinHandleCtx<F::Output>>,
	waker: Arc<waker::Waker>,
	#[pin]
	fut: F,
}

impl<F: Future> Task for TaskImpl< F> {
	fn poll(self: Pin<&mut Self>) -> Poll<()> {
		let me = self.project();
		let waker = waker::waker_ref(me.waker);
		let mut cx = std::task::Context::from_waker(&waker);

		match me.fut.poll(&mut cx) {
			Poll::Ready(output) => {
				me.ctx.output.set(Some(output));
				if let Some(waker) = me.ctx.waker.take() {
					waker.wake()
				}
				Poll::Ready(())
			}
			_ => Poll::Pending,
		}
	}

	fn canceled(self: Pin<&Self>) -> bool { self.project_ref().ctx.canceled.get() }
}

#[pin_project::pin_project]
struct RunUntil<'a, F: Future> {
	local_set: &'a mut LocalSet,
	waker: Arc<waker::Waker>,
	#[pin]
	fut: F,
}

const MAX_RUN_PER_POLL: u8 = 61;

impl<'a, F: Future> Future for RunUntil<'a, F> {
	type Output = F::Output;

	fn poll(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
		let mut me = self.project();

		me.local_set.context.waker.reset(cx.waker());

		CURRENT.set(&me.local_set.context, || {
			// limit poll times to avoid some runtime's budget mechanism
			let mut i = 0;
			while i < MAX_RUN_PER_POLL {
				i += 1;
				let next = me.local_set.scheduler.fetch();

				if let Some(next) = next {
					let task = me
						.local_set
						.context
						.tasks
						.borrow_mut()
						.get_mut(next)
						.and_then(Option::take);
					if let Some(mut task) = task {
						if !task.as_ref().canceled() && task.as_mut().poll().is_pending() {
							me.local_set.context.tasks.borrow_mut()[next] = Some(task);
						} else {
							me.local_set.context.tasks.borrow_mut().remove(next);
						}
					} else if next == me.waker.id {
						let waker = waker::waker_ref(me.waker);
						let mut cx = std::task::Context::from_waker(&waker);
						if let Poll::Ready(v) = me.fut.as_mut().poll(&mut cx) {
							me.local_set.context.tasks.borrow_mut().try_remove(next);
							return Poll::Ready(v);
						}
					} else {
						//XXX: task missiing or waker outliving the future, should trigger a warning
					}
				} else {
					break;
				}
			}
			if i == MAX_RUN_PER_POLL {
				cx.waker().wake_by_ref();
			}
			Poll::Pending
		})
	}
}

// must called within a LocalSet::run_until
pub fn spawn_local<F>(fut: F) -> JoinHandle<F::Output>
where
	F: Future + 'static,
	F::Output: 'static,
{
	CURRENT.with(|ctx| ctx.spawn(fut))
}

impl<T> JoinHandle<T> {
	pub fn detach(mut self) { self.detached = true; }
	pub fn cancel(self) -> Option<T> { self.ctx.output.take() }
}

impl<T> Future for JoinHandle<T> {
	type Output = T;

	fn poll(
		self: std::pin::Pin<&mut Self>,
		cx: &mut std::task::Context<'_>,
	) -> std::task::Poll<Self::Output> {
		match self.ctx.output.take() {
			Some(v) => Poll::Ready(v),
			None => {
				self.ctx.waker.set(Some(cx.waker().clone()));
				Poll::Pending
			}
		}
	}
}

impl<T> Drop for JoinHandle<T> {
	fn drop(&mut self) {
		if !self.detached {
			if let Some(waker) = self.ctx.waker.take() {
				waker.wake();
			}
			self.ctx.canceled.set(true);
		}
	}
}
