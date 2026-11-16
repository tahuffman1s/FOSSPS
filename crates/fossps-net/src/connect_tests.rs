//! Joining, leaving and going silent, with every datagram carried by hand.

use std::net::SocketAddr;
use std::time::Duration;

use crate::MAX_PACKET_BYTES;
use crate::connect::{
    Closed, DATA_HEADER, Heard, Host, HostSettings, JoinRequest, Link, LinkHeard, LinkSettings,
    Refusal,
};

fn address(port: u16) -> SocketAddr {
    SocketAddr::from(([203, 0, 113, 5], port))
}

fn seconds(seconds: f32) -> Duration {
    Duration::from_secs_f32(seconds)
}

/// One player and one server, and the datagram each last wrote.
struct Pair {
    host: Host,
    link: Link,
    from: SocketAddr,
    to_host: Vec<u8>,
    to_link: Vec<u8>,
}

impl Pair {
    fn new() -> Self {
        Self {
            host: Host::new(HostSettings::default()),
            link: Link::new(Duration::ZERO, LinkSettings::default()),
            from: address(5000),
            to_host: Vec::new(),
            to_link: Vec::new(),
        }
    }

    /// The link writes what it has to say; the host reads it.
    fn ask(&mut self, now: Duration) -> Option<Heard> {
        assert_eq!(self.link.update(now, &mut self.to_host), None);
        assert!(!self.to_host.is_empty(), "the link had nothing to send");
        self.host
            .receive(now, self.from, &self.to_host, &mut self.to_link)
    }

    /// The link reads what the host last wrote.
    fn answer(&mut self, now: Duration) -> Option<LinkHeard> {
        assert!(!self.to_link.is_empty(), "the host had nothing to send");
        self.link.receive(now, &self.to_link)
    }

    /// Runs the first three datagrams and gives the request to answer.
    fn request(&mut self) -> JoinRequest {
        assert_eq!(self.ask(Duration::ZERO), None);
        assert_eq!(self.answer(Duration::ZERO), None);
        match self.ask(Duration::ZERO) {
            Some(Heard::Join(request)) => request,
            other => panic!("expected a join, got {other:?}"),
        }
    }

    fn join(&mut self, id: u8) {
        let request = self.request();
        self.host
            .accept(Duration::ZERO, request, id, &mut self.to_link);
        assert_eq!(self.answer(Duration::ZERO), Some(LinkHeard::Joined(id)));
    }
}

#[test]
fn a_player_joins_in_four_datagrams_and_packets_then_pass_both_ways() {
    let mut pair = Pair::new();
    pair.join(7);
    assert_eq!(pair.link.id(), Some(7));
    assert_eq!(pair.host.address(7), Some(pair.from));
    assert_eq!(pair.host.players().collect::<Vec<_>>(), [7]);

    assert!(pair.link.wrap(b"commands", &mut pair.to_host));
    let now = seconds(1.0);
    assert_eq!(
        pair.host
            .receive(now, pair.from, &pair.to_host, &mut pair.to_link),
        Some(Heard::Packet(7))
    );
    assert_eq!(&pair.to_host[DATA_HEADER..], b"commands");

    assert_eq!(
        pair.host.wrap(7, b"snapshot", &mut pair.to_link),
        Some(pair.from)
    );
    assert_eq!(
        pair.link.receive(now, &pair.to_link),
        Some(LinkHeard::Packet)
    );
    assert_eq!(&pair.to_link[DATA_HEADER..], b"snapshot");
}

#[test]
fn a_full_packet_still_fits_when_wrapped() {
    let mut pair = Pair::new();
    pair.join(0);
    let full = [9; MAX_PACKET_BYTES];
    assert!(pair.link.wrap(&full, &mut pair.to_host));
    assert_eq!(
        pair.host
            .receive(Duration::ZERO, pair.from, &pair.to_host, &mut pair.to_link),
        Some(Heard::Packet(0))
    );
    pair.to_host.push(0);
    assert_eq!(
        pair.host
            .receive(Duration::ZERO, pair.from, &pair.to_host, &mut pair.to_link),
        None
    );
}

#[test]
fn no_answer_to_a_stranger_is_longer_than_what_they_sent() {
    let mut pair = Pair::new();
    assert_eq!(pair.ask(Duration::ZERO), None);
    assert_eq!(pair.to_host.len(), MAX_PACKET_BYTES);
    assert!(pair.to_link.len() < pair.to_host.len());

    // A request cut short gets no answer at all.
    let short = pair.to_host[..100].to_vec();
    assert_eq!(
        pair.host
            .receive(Duration::ZERO, pair.from, &short, &mut pair.to_link),
        None
    );
    assert!(pair.to_link.is_empty());
}

#[test]
fn the_server_remembers_nothing_until_the_address_is_proved() {
    let mut pair = Pair::new();
    for port in 0..1000 {
        pair.from = address(port);
        assert_eq!(pair.ask(Duration::ZERO), None);
        pair.link = Link::new(Duration::ZERO, LinkSettings::default());
    }
    assert_eq!(pair.host.players().count(), 0);
}

#[test]
fn a_number_given_to_one_address_is_no_use_from_another() {
    let mut pair = Pair::new();
    assert_eq!(pair.ask(Duration::ZERO), None);
    assert_eq!(pair.answer(Duration::ZERO), None);
    assert_eq!(pair.link.update(Duration::ZERO, &mut pair.to_host), None);
    assert_eq!(
        pair.host.receive(
            Duration::ZERO,
            address(6000),
            &pair.to_host,
            &mut pair.to_link
        ),
        None
    );
    assert!(pair.to_link.is_empty());
}

#[test]
fn a_made_up_number_does_not_join() {
    let mut pair = Pair::new();
    assert_eq!(pair.ask(Duration::ZERO), None);
    // Change the number the server sent before the link reads it.
    // It sits after the kind, the version and the player's own number.
    pair.to_link[17] ^= 1;
    assert_eq!(pair.answer(Duration::ZERO), None);
    assert_eq!(pair.ask(Duration::ZERO), None);
    assert!(pair.to_link.is_empty());
}

#[test]
fn a_number_goes_stale() {
    let mut pair = Pair::new();
    assert_eq!(pair.ask(Duration::ZERO), None);
    assert_eq!(pair.answer(Duration::ZERO), None);
    assert_eq!(pair.link.update(Duration::ZERO, &mut pair.to_host), None);
    assert_eq!(
        pair.host
            .receive(seconds(16.0), pair.from, &pair.to_host, &mut pair.to_link),
        None
    );
    assert!(matches!(
        pair.host
            .receive(seconds(7.0), pair.from, &pair.to_host, &mut pair.to_link),
        Some(Heard::Join(_))
    ));
}

#[test]
fn a_lost_acceptance_is_said_again_without_a_second_join() {
    let mut pair = Pair::new();
    let request = pair.request();
    pair.host
        .accept(Duration::ZERO, request, 3, &mut pair.to_link);
    // The acceptance is lost; the link answers the challenge again.
    pair.to_link.clear();
    assert_eq!(pair.ask(seconds(0.2)), None);
    assert_eq!(pair.answer(seconds(0.2)), Some(LinkHeard::Joined(3)));
    assert_eq!(pair.host.players().count(), 1);
}

#[test]
fn asking_is_repeated_at_the_resend_time_and_given_up_after_a_while() {
    let mut link = Link::new(Duration::ZERO, LinkSettings::default());
    let mut out = Vec::new();
    assert_eq!(link.update(Duration::ZERO, &mut out), None);
    assert!(!out.is_empty());
    assert_eq!(link.update(seconds(0.05), &mut out), None);
    assert!(out.is_empty());
    assert_eq!(link.update(seconds(0.11), &mut out), None);
    assert!(!out.is_empty());
    assert_eq!(link.update(seconds(5.1), &mut out), Some(Closed::NoAnswer));
    assert!(out.is_empty());
    assert_eq!(link.closed(), Some(Closed::NoAnswer));
    assert_eq!(link.update(seconds(5.2), &mut out), None);
}

#[test]
fn a_full_server_says_so() {
    let mut pair = Pair::new();
    let request = pair.request();
    Host::refuse(request, Refusal::Full, &mut pair.to_link);
    assert_eq!(
        pair.answer(Duration::ZERO),
        Some(LinkHeard::Closed(Closed::Full))
    );
    assert_eq!(pair.link.closed(), Some(Closed::Full));
}

#[test]
fn another_version_is_told_so_from_either_side() {
    let mut pair = Pair::new();
    assert_eq!(pair.link.update(Duration::ZERO, &mut pair.to_host), None);
    // A player from a newer engine.
    pair.to_host[1] += 1;
    assert_eq!(
        pair.host
            .receive(Duration::ZERO, pair.from, &pair.to_host, &mut pair.to_link),
        None
    );
    // And the server's answer is read even if its version is not ours.
    pair.to_link[1] += 1;
    assert_eq!(
        pair.answer(Duration::ZERO),
        Some(LinkHeard::Closed(Closed::WrongVersion))
    );
}

#[test]
fn a_refusal_for_somebody_else_is_not_believed() {
    let mut pair = Pair::new();
    let request = pair.request();
    Host::refuse(request, Refusal::Full, &mut pair.to_link);
    let mut other = Link::new(Duration::ZERO, LinkSettings::default());
    assert_eq!(other.receive(Duration::ZERO, &pair.to_link), None);
    assert_eq!(other.closed(), None);
}

#[test]
fn leaving_frees_the_place_and_a_late_copy_of_the_join_does_not_take_it_back() {
    let mut pair = Pair::new();
    let request = pair.request();
    let response = pair.to_host.clone();
    pair.host
        .accept(Duration::ZERO, request, 2, &mut pair.to_link);
    assert_eq!(pair.answer(Duration::ZERO), Some(LinkHeard::Joined(2)));

    assert!(pair.link.leave(&mut pair.to_host));
    assert_eq!(pair.link.closed(), Some(Closed::Left));
    let now = seconds(1.0);
    assert_eq!(
        pair.host
            .receive(now, pair.from, &pair.to_host, &mut pair.to_link),
        Some(Heard::Left(2))
    );
    assert_eq!(pair.host.players().count(), 0);
    // The second and third goodbyes, and the old joining datagram.
    assert_eq!(
        pair.host
            .receive(now, pair.from, &pair.to_host, &mut pair.to_link),
        None
    );
    assert_eq!(
        pair.host
            .receive(now, pair.from, &response, &mut pair.to_link),
        None
    );
    assert!(!pair.link.wrap(b"late", &mut pair.to_host));
}

#[test]
fn a_silent_player_is_dropped_and_an_empty_datagram_keeps_one() {
    let mut pair = Pair::new();
    pair.join(0);
    assert_eq!(pair.host.timed_out(seconds(9.0)), None);
    assert!(pair.link.wrap(&[], &mut pair.to_host));
    assert_eq!(
        pair.host
            .receive(seconds(9.0), pair.from, &pair.to_host, &mut pair.to_link),
        None
    );
    assert_eq!(pair.host.timed_out(seconds(18.0)), None);
    assert_eq!(pair.host.timed_out(seconds(19.5)), Some(0));
    assert_eq!(pair.host.timed_out(seconds(19.5)), None);
    assert_eq!(pair.host.wrap(0, b"snapshot", &mut pair.to_link), None);
}

#[test]
fn a_silent_server_ends_the_game() {
    let mut pair = Pair::new();
    pair.join(0);
    let mut out = Vec::new();
    assert_eq!(pair.link.update(seconds(9.0), &mut out), None);
    assert_eq!(pair.host.wrap(0, &[], &mut pair.to_link), Some(pair.from));
    assert_eq!(pair.link.receive(seconds(9.0), &pair.to_link), None);
    assert_eq!(pair.link.update(seconds(18.0), &mut out), None);
    assert_eq!(
        pair.link.update(seconds(19.5), &mut out),
        Some(Closed::TimedOut)
    );
    assert_eq!(pair.link.id(), None);
}

#[test]
fn packets_in_another_players_name_are_dropped() {
    let mut pair = Pair::new();
    pair.join(0);
    assert!(pair.link.wrap(b"commands", &mut pair.to_host));
    // Right number, wrong address.
    assert_eq!(
        pair.host.receive(
            Duration::ZERO,
            address(6000),
            &pair.to_host,
            &mut pair.to_link
        ),
        None
    );
    // Right address, wrong number.
    pair.to_host[5] ^= 1;
    assert_eq!(
        pair.host
            .receive(Duration::ZERO, pair.from, &pair.to_host, &mut pair.to_link),
        None
    );
    // And the same towards the player.
    assert_eq!(
        pair.host.wrap(0, b"snapshot", &mut pair.to_link),
        Some(pair.from)
    );
    pair.to_link[5] ^= 1;
    assert_eq!(pair.link.receive(Duration::ZERO, &pair.to_link), None);
}

#[test]
fn a_kicked_player_and_a_stopped_server_are_told_apart() {
    for (why, seen) in [
        (Refusal::Kicked, Closed::Kicked),
        (Refusal::Stopping, Closed::ServerStopped),
    ] {
        let mut pair = Pair::new();
        pair.join(1);
        assert_eq!(
            pair.host.drop_player(1, why, &mut pair.to_link),
            Some(pair.from)
        );
        assert_eq!(pair.answer(Duration::ZERO), Some(LinkHeard::Closed(seen)));
        assert_eq!(pair.host.players().count(), 0);
    }
}

#[test]
fn rubbish_is_ignored_by_both_sides() {
    let mut pair = Pair::new();
    pair.join(0);
    let mut noise = 0x2545_f491_4f6c_dd1d_u64;
    let mut datagram = Vec::new();
    for len in 0..300 {
        datagram.clear();
        for _ in 0..len {
            noise ^= noise << 13;
            noise ^= noise >> 7;
            noise ^= noise << 17;
            datagram.push(noise.to_le_bytes()[0]);
        }
        assert_eq!(
            pair.host
                .receive(Duration::ZERO, pair.from, &datagram, &mut pair.to_link),
            None
        );
        assert!(pair.to_link.is_empty());
        assert_eq!(pair.link.receive(Duration::ZERO, &datagram), None);
    }
    assert_eq!(pair.link.id(), Some(0));
}

#[test]
fn one_address_holds_one_place_at_a_time() {
    let mut pair = Pair::new();
    pair.join(0);
    // The same address and port asks again as if it were someone new.
    pair.link = Link::new(Duration::ZERO, LinkSettings::default());
    assert_eq!(pair.ask(Duration::ZERO), None);
    assert_eq!(pair.answer(Duration::ZERO), None);
    assert_eq!(pair.ask(Duration::ZERO), None);
    assert!(pair.to_link.is_empty());
    // Once the first stay has timed out, it may.
    assert_eq!(pair.host.timed_out(seconds(11.0)), Some(0));
    let now = seconds(11.0);
    pair.link = Link::new(now, LinkSettings::default());
    assert_eq!(pair.ask(now), None);
    assert_eq!(pair.answer(now), None);
    assert!(matches!(pair.ask(now), Some(Heard::Join(_))));
}
