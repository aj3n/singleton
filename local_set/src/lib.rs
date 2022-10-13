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

mod local_cell;
mod waker;

scoped_tls::scoped_thread_local!(static CURRENT: Context);
struct Context {
	tasks: RefCell<Slab<Option<TaskWrapper>>>,
	to_wake_ref: local_cell::LocalCellRef,

	// XXX:maybe some global weak reference instead?
	foreign_waker: mpsc::Sender<usize>,

	//TODO: move into to LocalCell
	waker: Option<task::Waker>,
	waked: Cell<bool>,
}

pub struct LocalSet {
	to_wake_cell: local_cell::LocalCell,
	to_wake_foreign: mpsc::Receiver<usize>,
	context: Context,
	to_wake: VecDeque<usize>,
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
		me.local_set.context.waked.set(false);
		me.local_set.context.waker = Some(cx.waker().clone());

		CURRENT.set(&me.local_set.context, || {
			if let Poll::Ready(output) = me.fut.poll(cx) {
				return Poll::Ready(output);
			}

			// limit poll times to avoid some runtime's budget mechanism
			for _ in 0..MAX_RUN_PER_POLL {
				if me.local_set.to_wake.is_empty() {
					me.local_set
						.to_wake
						.extend(me.local_set.to_wake_cell.fetch())
				}

				// FIXME: foreign_waker starvation
				let next = me
					.local_set
					.to_wake
					.pop_front()
					.or_else(|| me.local_set.to_wake_foreign.try_recv().ok());

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
					break;
				}
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
			to_wake_foreign: rx,
			to_wake: Default::default(),
			context: Context {
				tasks: RefCell::new(Slab::new()),
				to_wake_ref: From::from(&to_wake_cell),
				waker: None,
				waked: Cell::new(false),
				foreign_waker: tx,
			},
			to_wake_cell,
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
				to_wake: ctx.to_wake_ref.clone(),
				id,
				remote_waker: ctx.foreign_waker.clone(),
			}),
			fut,
		});
		ctx.to_wake_ref.notify(id).unwrap();
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
