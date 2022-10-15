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

mod local_cell;
mod waker;

scoped_tls::scoped_thread_local!(static CURRENT: Context);
struct Context {
	tasks: RefCell<Slab<Option<TaskWrapper>>>,
	waker: Arc<CachedWaker>,
}

pub struct LocalSet {
	context: Context,

	scheduler: Scheduler,
}

struct Scheduler {
	task_queue_cell: local_cell::LocalCell,
	task_queue_foreign: mpsc::Receiver<usize>,
	task_queue: VecDeque<usize>,
}

impl Scheduler {
	fn fetch(&mut self) -> Option<usize> {
		if self.task_queue.is_empty() {
			self.task_queue.extend(self.task_queue_cell.fetch())
		}
		// FIXME: foreign_waker starvation
		self.task_queue
			.pop_front()
			.or_else(|| self.task_queue_foreign.try_recv().ok())
	}

	fn give_back(&mut self, id: usize) { self.task_queue.push_front(id); }
}

impl LocalSet {
	pub async fn run_until<F>(&mut self, fut: F) -> F::Output
	where
		F: Future,
	{
		RunUntil {
			local_set: self,
			fut,
		}
		.await
	}
}

type TaskWrapper = Pin<Box<dyn Task>>;

struct JoinHandleCtx<T> {
	output: Cell<Option<T>>,
	waker: Cell<Option<task::Waker>>,
	id: Cell<usize>,
}

pub struct JoinHandle<T> {
	ctx: Rc<JoinHandleCtx<T>>,
}
impl<T: Unpin> Unpin for JoinHandle<T> {}

impl<T> JoinHandle<T> {
	fn new(id: usize) -> Self {
		Self {
			ctx: Rc::new(JoinHandleCtx {
				output: Cell::new(None),
				waker: Cell::new(None),
				id: Cell::new(id),
			}),
		}
	}
}

trait Task {
	fn poll(self: Pin<&mut Self>) -> Poll<()>;
}

#[pin_project::pin_project]
struct TaskImpl<F: Future> {
	ctx: Rc<JoinHandleCtx<F::Output>>,
	waker: Arc<waker::Waker>,
	#[pin]
	fut: F,
}

impl<F: Future> Task for TaskImpl<F> {
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
}

#[pin_project::pin_project]
struct RunUntil<'a, F: Future> {
	local_set: &'a mut LocalSet,
	#[pin]
	fut: F,
}

const MAX_RUN_PER_POLL: u8 = 16;

impl<'a, F: Future> Future for RunUntil<'a, F> {
	type Output = F::Output;

	fn poll(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
		let me = self.project();

		me.local_set.context.waker.reset(cx.waker());

		CURRENT.set(&me.local_set.context, || {
			if let Poll::Ready(output) = me.fut.poll(cx) {
				return Poll::Ready(output);
			}

			// limit poll times to avoid some runtime's budget mechanism
			for _ in 0..MAX_RUN_PER_POLL {
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
						if task.as_mut().poll().is_ready() {
							me.local_set.context.tasks.borrow_mut().remove(next);
						} else {
							me.local_set.context.tasks.borrow_mut()[next] = Some(task);
						}
					}
				} else {
					return Poll::Pending;
				}
			}
			if let Some(id) = me.local_set.scheduler.fetch() {
				me.local_set.scheduler.give_back(id);
				cx.waker().wake_by_ref();
			}
			Poll::Pending
		})
	}
}

impl Default for LocalSet {
	fn default() -> Self {
		let to_wake_cell = local_cell::LocalCell::new();
		let (tx, rx) = mpsc::channel();
		Self {
			context: Context {
				tasks: RefCell::new(Slab::new()),
				waker: Arc::new(CachedWaker::new((&to_wake_cell).into(), tx)),
			},
			scheduler: Scheduler {
				task_queue_foreign: rx,
				task_queue: Default::default(),
				task_queue_cell: to_wake_cell,
			},
		}
	}
}

// must called within a LocalSet::run_until
pub fn spawn_local<F>(fut: F) -> JoinHandle<F::Output>
where
	F: Future + 'static,
	F::Output: 'static,
{
	CURRENT.with(|ctx| {
		let mut tasks = ctx.tasks.borrow_mut();
		let id = tasks.insert(None);
		let handle = JoinHandle::new(id);
		let task = Box::pin(TaskImpl {
			ctx: handle.ctx.clone(),
			waker: Arc::new(waker::Waker {
				waker: ctx.waker.clone(),
				id,
			}),
			fut,
		});
		ctx.waker.wake_local(id).unwrap();
		tasks[id] = Some(task);

		handle
	})
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
