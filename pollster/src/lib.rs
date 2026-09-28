use std::{
    future::{Future, IntoFuture},
    sync::Arc,
    task::{Context, Poll, Wake, Waker},
    thread,
};

struct Signal {
    /// The thread that owns the signal.
    owning_thread: thread::Thread,
}

impl Wake for Signal {
    fn wake(self: Arc<Self>) {
        self.owning_thread.unpark();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.owning_thread.unpark();
    }
}

thread_local! {
    // A local reusable waker for each thread.
    static LOCAL_WAKER: Waker = {
        let signal = Arc::new(Signal {
            owning_thread: thread::current(),
        });
        Waker::from(signal)
    };
}

pub fn block_on<F: IntoFuture>(fut: F) -> F::Output {
    let mut fut = core::pin::pin!(fut.into_future());

    // A signal used to wake up the thread for polling as the future moves to completion.
    LOCAL_WAKER.with(|waker| {
        // Create a context to be passed to the future.
        let mut context = Context::from_waker(waker);

        // Poll the future to completion.
        loop {
            match fut.as_mut().poll(&mut context) {
                Poll::Pending => thread::park(),
                Poll::Ready(item) => break item,
            }
        }
    })
}
