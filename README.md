# mio-wakeq

A simple custom event delivery mechanism based on [mio](https://crates.io/crates/mio)'s `Waker` functionality for `mio`-based systems. `mio`'s `Poll` is limited to a single `Waker`, which restricts it to handling only one type of external event.
However, `mio-wakeq` allows multiple events to be managed and exposed through a single `Waker` within the same `Poll` instance.

# example

```rust
use mio::{Events, Poll, Token};
use mio_wakeq::{EventId, EventQ, WakeQ};

const WAKER: Token = Token(0);
const EVENT0: EventId = EventId(0);

let mut poll = Poll::new().unwrap();
let wakeq = EventQ::new(&poll, WAKER, 8).unwrap();

let event_sender = wakeq.get_sender();
std::thread::spawn(move || {
    event_sender.trigger_event(EVENT0);
});

let mut events = Events::with_capacity(32);
poll.poll(&mut events, None).unwrap();

for event in events.iter() {
    match event.token() {
        WAKER => {
            for wev in wakeq.triggered_events() {
                assert_eq!(wev, EVENT0);
            }
        }

        _ => unimplemented!(),
    }
}
```