use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::thread::JoinHandle;

type Job = Box<dyn FnOnce() + Send + 'static>;

/// A fixed-size pool for dispatching thread-independent work.
pub struct WorkerPool {
    sender: Option<mpsc::Sender<Job>>,
    workers: Vec<JoinHandle<()>>,
}

impl WorkerPool {
    /// Starts a pool with at least one worker.
    pub fn new(size: usize) -> Self {
        let (sender, receiver) = mpsc::channel::<Job>();
        let receiver = Arc::new(Mutex::new(receiver));
        let mut workers = Vec::with_capacity(size.max(1));
        for _ in 0..size.max(1) {
            let rx = receiver.clone();
            workers.push(std::thread::spawn(move || {
                loop {
                    let job = match rx.lock() {
                        Ok(receiver) => receiver.recv(),
                        Err(_) => break,
                    };
                    match job {
                        Ok(job) => job(),
                        Err(_) => break,
                    }
                }
            }));
        }
        Self {
            sender: Some(sender),
            workers,
        }
    }

    /// Queues one owned job for execution.
    pub fn execute<F: FnOnce() + Send + 'static>(&self, f: F) {
        if let Some(sender) = &self.sender {
            let _ = sender.send(Box::new(f));
        }
    }
}

impl Drop for WorkerPool {
    fn drop(&mut self) {
        self.sender.take();
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

/// Returns the process-wide worker pool sized to available parallelism.
pub fn default_worker_pool() -> &'static WorkerPool {
    static POOL: OnceLock<WorkerPool> = OnceLock::new();
    POOL.get_or_init(|| {
        let size = std::thread::available_parallelism()
            .map(|parallelism| parallelism.get())
            .unwrap_or(1);
        WorkerPool::new(size)
    })
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::WorkerPool;

    #[test]
    fn worker_pool_executes_jobs_and_joins_on_drop() {
        let (tx, rx) = mpsc::channel();
        {
            let pool = WorkerPool::new(2);
            pool.execute(move || {
                let _ = tx.send("done");
            });
            assert_eq!(rx.recv().unwrap(), "done");
        }
    }
}
