//! Opt-in, path-free evidence for a controlled removal investigation.

use std::collections::VecDeque;
use std::fmt::Write as _;
use std::sync::{Arc, Mutex, mpsc};
use std::time::Instant;

use super::RemovalEvent;

#[derive(Clone, Debug)]
pub(super) struct Progress(Arc<Mutex<Trace>>);

#[derive(Debug)]
struct Trace {
    start: Instant,
    history: VecDeque<String>,
    sent: [u64; 4],
    received: [u64; 4],
    empty: u64,
    disconnected: u64,
}

fn kind(event: &RemovalEvent) -> usize {
    match event {
        RemovalEvent::Cleanup(_) => 0,
        RemovalEvent::Start { .. } => 1,
        RemovalEvent::Item { .. } => 2,
        RemovalEvent::Done { .. } => 3,
    }
}

impl Progress {
    fn new() -> Self {
        Self(Arc::new(Mutex::new(Trace {
            start: Instant::now(),
            history: VecDeque::new(),
            sent: [0; 4],
            received: [0; 4],
            empty: 0,
            disconnected: 0,
        })))
    }

    pub(super) fn record(&self, stage: &str) {
        let mut trace = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // Bounded even when a contrived plan has many targets.
        if trace.history.len() == 128 {
            trace.history.pop_front();
        }
        let elapsed = trace.start.elapsed();
        trace.history.push_back(format!(
            "{elapsed:?} {:?}: {stage}",
            std::thread::current().id()
        ));
    }

    pub(super) fn polled(
        &self,
        event: &Result<RemovalEvent, mpsc::TryRecvError>,
    ) {
        let mut trace = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match event {
            Ok(event) => trace.received[kind(event)] += 1,
            Err(mpsc::TryRecvError::Empty) => trace.empty += 1,
            Err(mpsc::TryRecvError::Disconnected) => trace.disconnected += 1,
        }
    }

    pub(super) fn snapshot(&self) -> String {
        let trace = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut text = format!(
            "elapsed={:?} event order=[Cleanup, Start, Item, Done] sent={:?} received={:?} empty={} disconnected={}\n",
            trace.start.elapsed(),
            trace.sent,
            trace.received,
            trace.empty,
            trace.disconnected
        );
        for line in &trace.history {
            let _ = writeln!(text, "{line}");
        }
        text
    }
}

#[derive(Clone)]
pub(super) struct EventSender {
    sender: mpsc::Sender<RemovalEvent>,
    pub(super) progress: Progress,
}

impl EventSender {
    pub(super) fn new(sender: mpsc::Sender<RemovalEvent>) -> Self {
        Self {
            sender,
            progress: Progress::new(),
        }
    }

    pub(super) fn send(
        &self,
        event: RemovalEvent,
    ) -> Result<(), mpsc::SendError<RemovalEvent>> {
        let index = kind(&event);
        let result = self.sender.send(event);
        if result.is_ok() {
            self.progress
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .sent[index] += 1;
            self.progress.record(&format!("sent event {index}"));
        } else {
            self.progress.record(&format!("send failed event {index}"));
        }
        result
    }

    pub(super) fn worker(&self) -> WorkerExit {
        WorkerExit(self.progress.clone())
    }
}

pub(super) struct WorkerExit(Progress);

impl Drop for WorkerExit {
    fn drop(&mut self) {
        self.0.record(if std::thread::panicking() {
            "worker unwinding"
        } else {
            "worker exited normally"
        });
    }
}
