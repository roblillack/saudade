//! [`Waker`]: wake an app's event loop from outside it.

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// A handle that wakes a running [`App`](crate::App) up from anywhere — another
/// thread, or code the event loop calls without going through a widget, such as
/// a callback the operating system makes straight into the program — and has it
/// dispatch an [`Event::Tick`](crate::Event::Tick) to the widget tree.
///
/// The event loop sleeps while nothing happens in its windows, so work that
/// arrives from elsewhere — a worker thread finishing a download, a file
/// watcher, a URL handed to the program by the desktop — needs a way to get the
/// widgets to look at it. The pattern is a queue the widgets can see (an
/// `Arc<Mutex<…>>`, a channel) plus a `Waker`: put the work in the queue, call
/// [`wake`](Self::wake), and drain the queue when the tick arrives.
///
/// ```no_run
/// # use saudade::*;
/// # use std::sync::mpsc;
/// # fn root(rx: mpsc::Receiver<String>) -> Label { Label::new(Rect::new(0, 0, 10, 10), "") }
/// let (tx, rx) = mpsc::channel::<String>();
/// let app = App::new(WindowConfig::new("Fetch", 320, 200), root(rx));
/// let waker = app.waker();
/// std::thread::spawn(move || {
///     tx.send("done".into()).unwrap();
///     waker.wake(); // the root's next Tick finds the message in `rx`
/// });
/// app.run();
/// ```
///
/// A `Waker` is cheap to clone and can be sent to and shared between threads.
/// Wakes that arrive before the loop has started, or while it is still busy, are
/// not lost: however many there were, the loop dispatches one tick for them as
/// soon as it can.
#[derive(Clone, Default)]
pub struct Waker {
    inner: Arc<Inner>,
}

#[derive(Default)]
struct Inner {
    /// Set by [`Waker::wake`], cleared by the runtime when it dispatches the
    /// tick for it.
    pending: AtomicBool,
    /// How the running backend's event loop is woken: a winit user event or
    /// nothing at all (the Wayland loop looks at `pending` every iteration).
    /// `None` until the loop starts.
    notify: Mutex<Option<Box<dyn Fn() + Send>>>,
}

impl Waker {
    /// Wake the event loop and have it dispatch an
    /// [`Event::Tick`](crate::Event::Tick) to the widget tree.
    pub fn wake(&self) {
        self.inner.pending.store(true, Ordering::SeqCst);
        if let Ok(notify) = self.inner.notify.lock()
            && let Some(notify) = notify.as_ref()
        {
            notify();
        }
    }

    /// Whether [`wake`](Self::wake) was called since the last time this was
    /// asked, clearing the flag. The runtime asks once per loop iteration.
    pub(crate) fn take(&self) -> bool {
        self.inner.pending.swap(false, Ordering::SeqCst)
    }

    /// Tell the waker how to rouse the event loop that has just started.
    pub(crate) fn install(&self, notify: impl Fn() + Send + 'static) {
        if let Ok(mut slot) = self.inner.notify.lock() {
            *slot = Some(Box::new(notify));
        }
    }
}

impl fmt::Debug for Waker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Waker")
            .field("pending", &self.inner.pending.load(Ordering::SeqCst))
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn a_wake_is_pending_until_taken() {
        let waker = Waker::default();
        assert!(!waker.take());
        waker.wake();
        assert!(waker.take());
        assert!(!waker.take(), "taking it clears it");
    }

    #[test]
    fn a_wake_before_the_loop_starts_is_kept() {
        let waker = Waker::default();
        waker.clone().wake();
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = calls.clone();
        waker.install(move || {
            counted.fetch_add(1, Ordering::SeqCst);
        });
        assert!(waker.take());
        waker.wake();
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "an installed loop is roused"
        );
    }

    #[test]
    fn a_waker_can_be_used_from_another_thread() {
        let waker = Waker::default();
        let remote = waker.clone();
        std::thread::spawn(move || remote.wake()).join().unwrap();
        assert!(waker.take());
    }
}
