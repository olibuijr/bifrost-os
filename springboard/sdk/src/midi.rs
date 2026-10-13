//! MIDI for apps: endpoints, packets and the routing hub, shared with the MIDI server.
//!
//! The models live in `bifrost_midi`, so an app and the MIDI daemon use one vocabulary and one packet type:
//! - `endpoints`: the registry of clients, devices, entities, sources and destinations, and source connections.
//! - `hub`: the routing core. `Hub::create_app` gives an app a client with a virtual source and destination,
//!   `send_from_source` queues a packet for every client connected to a source, and `drain_inbox` returns what
//!   a client received at or before a host-clock time.
//! - `ump`: Universal MIDI Packets (MIDI 2.0) with MIDI 1.0 conversion at the edges.
//! - `events`: MIDI 1.0 channel voice events, a parser for byte streams and a note tracker.
//! - `thru`: play-through with channel filters, value maps and control transforms.
//! - `clock`: conversion between host-clock nanoseconds and audio sample frames.
//!
//! Nothing here opens a device. A caller that owns hardware adds its endpoints to a `Registry`.

pub use bifrost_midi::{clock, endpoints, events, hub, thru, ump};

#[cfg(test)]
mod tests {
    use super::endpoints::Direction;
    use super::hub::Hub;
    use super::ump::Ump;

    #[test]
    fn app_receives_only_from_connected_sources() {
        let mut hub = Hub::new();
        let (app, _src, _dst) = hub.create_app("Test");
        let (_other, other_src, _) = hub.create_app("Other");
        // An app's own virtual source is a source, so the app may listen to it.
        let (_, own_src, _) = hub.create_app("Third");
        assert!(hub.registry.connect_source(app, own_src));
        let packet = Ump::one(0x2090_3C64);
        assert_eq!(hub.send_from_source(own_src, 100, packet), 1);
        assert_eq!(hub.send_from_source(other_src, 100, packet), 0);
        assert_eq!(hub.pending(app), 1);
        let got = hub.drain_inbox(app, 100);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].source, own_src);
        assert_eq!(hub.pending(app), 0);
    }

    #[test]
    fn virtual_endpoints_are_sources_and_destinations() {
        let mut hub = Hub::new();
        let (client, src, dst) = hub.create_app("Synth");
        assert_eq!(hub.registry.endpoint(src).map(|e| e.direction), Some(Direction::Source));
        assert_eq!(hub.registry.endpoint(dst).map(|e| e.direction), Some(Direction::Destination));
        assert!(hub.dispose_app(client));
        assert!(hub.registry.endpoint(src).is_none());
    }
}
