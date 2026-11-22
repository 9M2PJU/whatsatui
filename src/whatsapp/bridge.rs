//! Receives protocol events without blocking the synchronous dispatch path.
//!
//! `whatsapp-rust` calls `EventHandler::handle_event` from the thread that
//! decodes an incoming node, which can be a tokio runtime worker. The handler
//! must not block. `block_in_place` hands a worker's core to a blocking-pool
//! thread, and a burst of events against a full bounded queue can consume every
//! blocking thread, leaving the runtime unable to drain the queue; the app then
//! sits on "Connecting" until it is restarted. An unbounded channel keeps
//! `send` non-blocking, and the ingest task applies backpressure where waiting
//! is safe by awaiting the bounded application-event channel and storage.
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
pub(super) struct Bridge<T> {
    tx: UnboundedSender<T>,
}
pub(super) fn channel<T>() -> (Bridge<T>, UnboundedReceiver<T>) {
    let (tx, rx) = unbounded_channel();
    (Bridge { tx }, rx)
}
impl<T> Bridge<T> {
    /// Queues an event synchronously. This runs on the protocol's dispatch
    /// path, so it must never wait for the consumer.
    pub fn send(&self, value: T) -> Result<(), ()> {
        self.tx.send(value).map_err(|_| ())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    #[test]
    fn a_stalled_consumer_never_blocks_the_producer() {
        let (bridge, mut rx) = channel();
        let (done, finished) = std::sync::mpsc::channel();
        let producer = std::thread::spawn(move || {
            for index in 0..100_000u32 {
                bridge.send(index).unwrap();
            }
            done.send(()).unwrap();
        });
        finished
            .recv_timeout(Duration::from_secs(5))
            .expect("a stalled consumer must not block the protocol event handler");
        producer.join().unwrap();
        for expected in 0..100_000u32 {
            assert_eq!(rx.try_recv(), Ok(expected));
        }
        assert!(rx.try_recv().is_err());
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_burst_from_a_runtime_worker_does_not_wait_on_the_consumer() {
        let (bridge, mut rx) = channel();
        let burst = tokio::spawn(async move {
            for index in 0..100_000u32 {
                bridge.send(index).unwrap();
            }
        });
        tokio::time::timeout(Duration::from_secs(5), burst)
            .await
            .expect("event dispatch must not block a runtime worker")
            .unwrap();
        for expected in 0..100_000u32 {
            assert_eq!(rx.try_recv(), Ok(expected));
        }
        assert!(rx.try_recv().is_err());
    }
    #[test]
    fn a_closed_consumer_reports_closed_instead_of_blocking() {
        let (bridge, rx) = channel();
        drop(rx);
        assert!(bridge.send(1).is_err());
    }
}
