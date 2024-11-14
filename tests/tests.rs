#[cfg(test)]
mod tests {
    extern crate mio_wakeq;

    use std::sync::mpsc;

    use mio::{Events, Poll, Token};

    use mio_wakeq::{EventId, EventQ, WakeQ};

    const WAKER: Token = Token(0);
    const EVENT0: EventId = EventId(0);
    const EVENT1: EventId = EventId(1);
    const EVENT128: EventId = EventId(128);

    #[test]
    fn single_event() {
        let mut poll = Poll::new().unwrap();
        let wakeq = EventQ::new(&poll, WAKER, 8).unwrap();

        let event_sender = wakeq.get_sender();
        std::thread::spawn(move || {
            let _ = event_sender.trigger_event(EVENT0);
        });

        let mut events = Events::with_capacity(32);
        poll.poll(&mut events, None).unwrap();
        assert!(!events.is_empty());

        for event in events.iter() {
            assert_eq!(event.token(), WAKER);

            for wev in wakeq.triggered_events() {
                assert_eq!(wev, EVENT0);
            }
        }
    }

    #[test]
    fn single_event_sz128() {
        let mut poll = Poll::new().unwrap();
        let wakeq = EventQ::new(&poll, WAKER, 192).unwrap();

        let event_sender = wakeq.get_sender();
        std::thread::spawn(move || {
            let _ = event_sender.trigger_event(EVENT128);
        });

        let mut events = Events::with_capacity(32);
        poll.poll(&mut events, None).unwrap();
        assert!(!events.is_empty());

        for event in events.iter() {
            assert_eq!(event.token(), WAKER);

            for wev in wakeq.triggered_events() {
                assert_eq!(wev, EVENT128);
            }
        }
    }

    #[test]
    fn multiple_events() {
        let mut event0_triggered = false;
        let mut event1_triggered = false;

        let mut poll = Poll::new().unwrap();
        let wakeq = EventQ::new(&poll, WAKER, 8).unwrap();

        let (c1_tx, c1_rx) = mpsc::sync_channel(1);

        let event_sender = wakeq.get_sender();
        std::thread::spawn(move || {
            let _ = event_sender.trigger_event(EVENT0);
            let _ = event_sender.trigger_event(EVENT1);

            let _ = c1_tx.send(());
        });

        let _ = c1_rx.recv();

        let mut events = Events::with_capacity(32);
        poll.poll(&mut events, None).unwrap();
        assert!(!events.is_empty());

        for event in events.iter() {
            assert_eq!(event.token(), WAKER);

            for wev in wakeq.triggered_events() {
                match wev {
                    EVENT0 => event0_triggered = true,
                    EVENT1 => event1_triggered = true,
                    _ => unreachable!(),
                };
            }
        }

        assert!(event0_triggered && event1_triggered);
    }

    #[test]
    fn empty_pending_after_processing() {
        let mut poll = Poll::new().unwrap();
        let wakeq = EventQ::new(&poll, WAKER, 8).unwrap();

        // Spawn a producer thread to set events
        let event_sender = wakeq.get_sender();
        std::thread::spawn(move || {
            let _ = event_sender.trigger_event(EVENT0);
        });

        let mut events = Events::with_capacity(32);
        poll.poll(&mut events, None).unwrap();
        assert!(!events.is_empty());

        for event in events.iter() {
            assert_eq!(event.token(), WAKER);

            for wev in wakeq.triggered_events() {
                assert_eq!(wev, EVENT0);
            }

            assert_eq!(wakeq.triggered_events().count(), 0);
        }
    }

    #[test]
    fn event_trigger_during_processing() {
        let mut poll = Poll::new().unwrap();
        let wakeq = EventQ::new(&poll, WAKER, 8).unwrap();

        let (c1_tx, c1_rx) = mpsc::sync_channel(1);
        let (c2_tx, c2_rx) = mpsc::sync_channel(1);

        let event_sender = wakeq.get_sender();
        std::thread::spawn(move || {
            let _ = event_sender.trigger_event(EVENT0);

            let _ = c1_rx.recv();

            let _ = event_sender.trigger_event(EVENT1);

            let _ = c2_tx.send(());
        });

        let mut events = Events::with_capacity(32);
        poll.poll(&mut events, None).unwrap();
        assert!(!events.is_empty());

        for event in events.iter() {
            assert_eq!(event.token(), WAKER);

            for wev in wakeq.triggered_events() {
                assert_eq!(wev, EVENT0);
            }

            let _ = c1_tx.send(());
            let _ = c2_rx.recv();

            for wev in wakeq.triggered_events() {
                assert_eq!(wev, EVENT1);
            }

            assert_eq!(wakeq.triggered_events().count(), 0);
        }
    }

    #[test]
    fn wakeq_multiple_events() {
        let mut val1_triggered = false;
        let mut val16_triggered = false;

        let mut poll = Poll::new().unwrap();
        let wakeq: WakeQ<usize> = WakeQ::new(&poll, WAKER).unwrap();

        let (c1_tx, c1_rx) = mpsc::sync_channel(1);

        let event_sender = wakeq.get_sender();
        std::thread::spawn(move || {
            let _ = event_sender.send_event(1);
            let _ = event_sender.send_event(16);

            let _ = c1_tx.send(());
        });

        let mut events = Events::with_capacity(32);
        poll.poll(&mut events, None).unwrap();
        assert!(!events.is_empty());

        let _ = c1_rx.recv();

        for event in events.iter() {
            assert_eq!(event.token(), WAKER);

            for wev in wakeq.iter_pending_events() {
                match wev {
                    1 => val1_triggered = true,
                    16 => val16_triggered = true,
                    _ => unreachable!(),
                };
            }

            assert_eq!(wakeq.iter_pending_events().count(), 0);
        }

        assert!(val1_triggered && val16_triggered);
    }
}
