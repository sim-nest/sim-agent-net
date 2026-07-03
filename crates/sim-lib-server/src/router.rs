use std::{
    collections::VecDeque,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use sim_kernel::{Error, Result};

use crate::ServerFrame;

/// Allocates message ids and queues inbound server frames for a connection.
#[derive(Default)]
pub struct FrameRouter {
    next_id: AtomicU64,
    inbound: Mutex<VecDeque<ServerFrame>>,
}

impl FrameRouter {
    /// Allocates and returns the next unique message id.
    pub fn fresh_msg_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed) + 1
    }

    /// Returns the message id the next [`FrameRouter::fresh_msg_id`] call would
    /// produce, without consuming it.
    pub fn peek_next_msg_id(&self) -> u64 {
        self.next_id.load(Ordering::Relaxed) + 1
    }

    /// Pushes a frame onto the back of the inbound queue.
    ///
    /// Returns [`Error::PoisonedLock`] rather than panicking when the queue
    /// mutex is poisoned, matching the sibling server locks.
    pub fn push_inbound(&self, frame: ServerFrame) -> Result<()> {
        self.inbound
            .lock()
            .map_err(|_| Error::PoisonedLock("frame router inbound queue"))?
            .push_back(frame);
        Ok(())
    }

    /// Pops the next frame from the front of the inbound queue, if any.
    ///
    /// Returns [`Error::PoisonedLock`] rather than panicking when the queue
    /// mutex is poisoned, matching the sibling server locks.
    pub fn pop_inbound(&self) -> Result<Option<ServerFrame>> {
        Ok(self
            .inbound
            .lock()
            .map_err(|_| Error::PoisonedLock("frame router inbound queue"))?
            .pop_front())
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, thread};

    use sim_kernel::{Error, Symbol};

    use super::FrameRouter;
    use crate::{FrameEnvelope, FrameKind, ServerFrame};

    fn poison(router: &Arc<FrameRouter>) {
        let poisoner = Arc::clone(router);
        let _ = thread::spawn(move || {
            let _guard = poisoner.inbound.lock().unwrap();
            panic!("intentionally poison the inbound queue mutex");
        })
        .join();
    }

    fn frame() -> ServerFrame {
        ServerFrame::new(
            Symbol::new("lisp"),
            FrameKind::Notify,
            FrameEnvelope::default(),
            Vec::new(),
        )
    }

    #[test]
    fn push_and_pop_return_poisoned_lock_instead_of_panicking() {
        let router = Arc::new(FrameRouter::default());
        poison(&router);

        assert!(matches!(
            router.push_inbound(frame()),
            Err(Error::PoisonedLock("frame router inbound queue"))
        ));
        assert!(matches!(
            router.pop_inbound(),
            Err(Error::PoisonedLock("frame router inbound queue"))
        ));
    }
}
