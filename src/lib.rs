//! # mio-wakeq
//!
//! A simple custom event delivery mechanism based on `mio`'s `Waker`
//! functionality for `mio`-based systems. `mio`'s `Poll` is limited to a single
//! `Waker`, which restricts it to handling only one type of external event.
//! However, `mio-wakeq` allows multiple events to be managed and exposed
//! through a single `Waker` within the same `Poll` instance.
//!
//! ```rust
//! use mio::{Events, Poll, Token};
//! use mio_wakeq::{EventId, EventQ, WakeQ};
//!
//! const WAKER: Token = Token(0);
//! const EVENT0: EventId = EventId(0);
//!
//! let mut poll = Poll::new().unwrap();
//! let wakeq = EventQ::new(&poll, WAKER, 8).unwrap();
//!
//! let event_sender = wakeq.get_sender();
//! std::thread::spawn(move || {
//!     event_sender.trigger_event(EVENT0);
//! });
//!
//! let mut events = Events::with_capacity(32);
//! poll.poll(&mut events, None).unwrap();
//!
//! for event in events.iter() {
//!     match event.token() {
//!         WAKER => {
//!             for wev in wakeq.triggered_events() {
//!                 assert_eq!(wev, EVENT0);
//!             }
//!         }
//!
//!         _ => unimplemented!(),
//!     }
//! }
//! ```

use mio::{Poll, Token, Waker};
use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{
    mpsc::{self, Receiver, Sender},
    Arc,
};

//
//
// EventId
//
//

/// Event Id to specify uniqiue custom Event
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EventId(pub usize);

impl From<usize> for EventId {
    fn from(id: usize) -> Self {
        EventId(id)
    }
}

impl From<EventId> for usize {
    fn from(event_id: EventId) -> Self {
        event_id.0
    }
}

//
//
// WakeQSender
//
//

/// Queue Sender for the custom type message
///
/// Used from the app thread to notify poll loop.
/// Consist of async Tx channel and the mio Waker
pub struct WakeQSender<T> {
    tx: Sender<T>,
    waker: Arc<Waker>,
}

impl<T> WakeQSender<T> {
    /// Send the custom type message and wake the poll loop
    pub fn send_event(&self, event: T) -> io::Result<()> {
        self.tx.send(event).map_err(|err| {
            io::Error::new(
                io::ErrorKind::Other,
                format!("Failed to send message: {}", err),
            )
        })?;

        self.waker.wake()
    }
}

//
//
// WakeQ
//
//

/// Waker Queue for custom type message
///
/// Should be single instance per poll loop.
/// There are two halfs distributed between threads:
/// - `rx` - process messages inside the poll loop
/// - `sender` - shared between notifiers thread to expose custom messages to the poll loop
pub struct WakeQ<T> {
    rx: Receiver<T>,
    sender: WakeQSender<T>,
}

impl<T> WakeQ<T> {
    /// Create new Waker Queue. Register Waker into the exist Poll instance
    /// and create async channel as a message delivery mechanism.
    pub fn new(poll: &Poll, token: Token) -> io::Result<Self> {
        let (tx, rx) = mpsc::channel();
        let waker = Waker::new(poll.registry(), token)?;
        let sender = WakeQSender {
            tx,
            waker: Arc::new(waker),
        };
        Ok(WakeQ { rx, sender })
    }

    /// Get the cloned `WakeQSender`
    pub fn get_sender(&self) -> WakeQSender<T> {
        self.sender.clone()
    }

    /// Iterator for messages in the channel.
    /// Obtain all the messages via non-blocking way.
    /// Used for the processing side in the poll loop.
    pub fn iter_pending_events(&self) -> impl Iterator<Item = T> + '_ {
        std::iter::from_fn(|| self.rx.try_recv().ok())
    }
}

impl<T> Clone for WakeQSender<T> {
    fn clone(&self) -> Self {
        WakeQSender {
            tx: self.tx.clone(),
            waker: Arc::clone(&self.waker),
        }
    }
}

//
//
// EventQSender
//
//

/// Event Sender for the event trigerring mechanism.
///
/// Used by the app thread to notify when an event happens.
/// Consist of poll Waker and shared reference to Events storage cloned
/// from the `EventQ`
#[derive(Clone)]
pub struct EventQSender {
    events: Arc<Vec<AtomicUsize>>,
    waker: Arc<Waker>,
}

impl EventQSender {
    /// Trigger the event by EventId and wake the poll loop from notifier thread
    pub fn trigger_event(&self, event_id: EventId) -> io::Result<()> {
        let event_id = usize::from(event_id);
        let (chunk_idx, bit_idx) = Self::event_position(event_id);
        self.events[chunk_idx].fetch_or(1 << bit_idx, Ordering::SeqCst);
        self.waker.wake()
    }

    fn event_position(event_id: usize) -> (usize, u32) {
        let chunk_idx = event_id / usize::BITS as usize;
        let bit_idx = (event_id % usize::BITS as usize) as u32;
        (chunk_idx, bit_idx)
    }
}

//
//
// EventQ
//
//

/// Events storage and processor
///
/// Should be single instance per poll loop.
/// `events` is a event storage that shared by reference with the `sender`s.
/// `sender` shared between notifiers thread to expose custom messages to the poll loop
pub struct EventQ {
    events: Arc<Vec<AtomicUsize>>,
    sender: EventQSender,
}

impl EventQ {
    /// Create new `EventQ`. Register Waker into the exist Poll instance
    /// and create shared storage for the specified number of events
    pub fn new(poll: &Poll, token: Token, num_events: usize) -> io::Result<Self> {
        let num_chunks = (num_events + usize::BITS as usize - 1) / usize::BITS as usize;
        let events = Arc::new((0..num_chunks).map(|_| AtomicUsize::new(0)).collect());
        let waker = Arc::new(Waker::new(poll.registry(), token)?);

        let sender = EventQSender {
            events: Arc::clone(&events),
            waker,
        };
        Ok(EventQ { events, sender })
    }

    /// Get the cloned `EventQSender`
    pub fn get_sender(&self) -> EventQSender {
        self.sender.clone()
    }

    /// Iterator for pending events.
    /// Created over the already triggered events at the moment.
    /// All events in the `EventQ` are reset before the iterator is created, which allows
    /// triggering new events and scheduling the poll wake during current proccesing
    pub fn triggered_events(&self) -> EventQIterator {
        let triggered_events = self
            .events
            .iter()
            .map(|chunk| chunk.swap(0, Ordering::SeqCst))
            .collect();
        EventQIterator {
            events: triggered_events,
            chunk_idx: 0,
            bit_idx: 0,
        }
    }
}

//
//
// EventQIterator
//
//

/// Event's iterator for the pendings events from `EventQ`
pub struct EventQIterator {
    events: Vec<usize>,
    chunk_idx: usize,
    bit_idx: u32,
}

impl Iterator for EventQIterator {
    type Item = EventId;

    fn next(&mut self) -> Option<Self::Item> {
        while self.chunk_idx < self.events.len() {
            let chunk = self.events[self.chunk_idx];
            while self.bit_idx < usize::BITS {
                let bit = 1 << self.bit_idx;
                let bit_position = self.chunk_idx * usize::BITS as usize + self.bit_idx as usize;
                self.bit_idx += 1;
                if chunk & bit != 0 {
                    return Some(EventId(bit_position));
                }
            }
            self.chunk_idx += 1;
            self.bit_idx = 0;
        }
        None
    }
}
