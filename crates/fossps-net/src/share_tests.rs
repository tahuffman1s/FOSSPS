//! Files offered to players about to join, with every datagram carried by
//! hand.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use crate::connect::{Closed, Heard, Host, HostSettings, Link, LinkHeard, LinkSettings};
use crate::share::{PIECE_BYTES, WINDOW};
use crate::{MAX_PACKET_BYTES, MAX_SHARED_BYTES, MAX_SHARED_FILES, ShareError, hash_of};

const STEP: Duration = Duration::from_millis(10);

fn address(port: u16) -> SocketAddr {
    SocketAddr::from(([203, 0, 113, 5], port))
}

/// Bytes that differ from one place to the next, so a piece put in the
/// wrong place shows.
fn file(len: usize) -> Arc<[u8]> {
    (0..len)
        .map(|at| (at.wrapping_mul(31) ^ (at >> 8)) as u8)
        .collect()
}

/// One player and one server, with a clock both read.
struct Pair {
    host: Host,
    link: Link,
    from: SocketAddr,
    now: Duration,
    to_host: Vec<u8>,
    to_link: Vec<u8>,
    /// How many pieces the server has sent.
    sent: u32,
    /// Whether the piece with this count, of those the server sends, is
    /// lost on the way.
    lose: fn(u32) -> bool,
    /// Whether the server answers anything.
    silent: bool,
}

impl Pair {
    fn new() -> Self {
        Self {
            host: Host::new(HostSettings::default()),
            link: Link::new(Duration::ZERO, LinkSettings::default()),
            from: address(5000),
            now: Duration::ZERO,
            to_host: Vec::new(),
            to_link: Vec::new(),
            sent: 0,
            lose: |_| false,
            silent: false,
        }
    }

    /// Lets the link say what it has to and the host answer, a hundredth
    /// of a second later than last time. Gives what the link made of the
    /// answers and what the host made of the question.
    fn step(&mut self) -> (Vec<LinkHeard>, Option<Heard>) {
        self.now += STEP;
        let mut heard = Vec::new();
        if let Some(why) = self.link.update(self.now, &mut self.to_host) {
            return (vec![LinkHeard::Closed(why)], None);
        }
        if self.to_host.is_empty() || self.silent {
            return (heard, None);
        }
        let said = self
            .host
            .receive(self.now, self.from, &self.to_host, &mut self.to_link);
        if !self.to_link.is_empty() {
            heard.extend(self.link.receive(self.now, &self.to_link));
        }
        if let Some(Heard::Fetch(fetch)) = said {
            let mut ahead = 0;
            while self.host.write_piece(fetch, ahead, &mut self.to_link) {
                assert!(self.to_link.len() <= MAX_PACKET_BYTES);
                ahead += 1;
                self.sent += 1;
                if !(self.lose)(self.sent) {
                    heard.extend(self.link.receive(self.now, &self.to_link));
                }
            }
        }
        (heard, said)
    }

    /// Steps until the link makes something of an answer.
    fn until_heard(&mut self) -> LinkHeard {
        for _ in 0..100_000 {
            if let Some(heard) = self.step().0.first() {
                return *heard;
            }
        }
        panic!("the link heard nothing");
    }

    /// Steps until the host is asked for a place, and gives it.
    fn join(&mut self, id: u8) {
        for _ in 0..1000 {
            if let (_, Some(Heard::Join(request))) = self.step() {
                self.host.accept(self.now, request, id, &mut self.to_link);
                assert_eq!(
                    self.link.receive(self.now, &self.to_link),
                    Some(LinkHeard::Joined(id))
                );
                return;
            }
        }
        panic!("the link never asked for a place");
    }
}

#[test]
fn a_player_fetches_what_the_server_offers_and_then_joins() {
    let mut pair = Pair::new();
    let level = file(100_000);
    let map = file(700);
    pair.host.share(0, level.clone()).unwrap();
    pair.host.share(2, map.clone()).unwrap();

    assert_eq!(pair.until_heard(), LinkHeard::Offered);
    let offer = *pair.link.offer();
    assert_eq!(offer.file(0).unwrap().len, 100_000);
    assert_eq!(offer.file(0).unwrap().hash, hash_of(&level));
    assert_eq!(offer.file(1), None);
    assert_eq!(offer.file(2).unwrap().len, 700);
    assert_eq!(offer.file(MAX_SHARED_FILES), None);

    assert!(pair.link.fetch(pair.now, 0));
    assert_eq!(pair.link.fetched(), None);
    assert_eq!(pair.until_heard(), LinkHeard::Fetched(0));
    assert_eq!(pair.link.fetched(), Some(&*level));
    assert_eq!(pair.link.progress(), None);

    assert!(pair.link.fetch(pair.now, 2));
    assert_eq!(pair.until_heard(), LinkHeard::Fetched(2));
    assert_eq!(pair.link.fetched(), Some(&*map));

    pair.join(4);
    assert_eq!(pair.link.id(), Some(4));
    // Too late to fetch anything more.
    assert!(!pair.link.fetch(pair.now, 0));
}

#[test]
fn a_player_who_fetches_nothing_joins_as_before() {
    let mut pair = Pair::new();
    pair.host.share(0, file(5000)).unwrap();
    assert_eq!(pair.until_heard(), LinkHeard::Offered);
    pair.join(1);
    assert_eq!(pair.sent, 0);
}

#[test]
fn a_server_with_nothing_on_offer_says_nothing_about_files() {
    let mut pair = Pair::new();
    pair.join(1);
    assert!(pair.link.offer().is_empty());
}

#[test]
fn the_offer_keeps_the_answer_shorter_than_the_request() {
    let mut pair = Pair::new();
    for slot in 0..MAX_SHARED_FILES {
        pair.host.share(slot, file(10 + slot)).unwrap();
    }
    pair.step();
    assert_eq!(pair.to_host.len(), MAX_PACKET_BYTES);
    assert!(pair.to_link.len() < 200, "{}", pair.to_link.len());
}

#[test]
fn lost_pieces_are_asked_for_again() {
    let mut pair = Pair::new();
    let level = file(40 * PIECE_BYTES + 17);
    pair.host.share(0, level.clone()).unwrap();
    // One piece in four is lost, and so is the whole of the second answer.
    pair.lose =
        |sent| sent.wrapping_mul(0x9E37_79B1) >> 30 == 0 || (WINDOW..2 * WINDOW).contains(&sent);
    assert_eq!(pair.until_heard(), LinkHeard::Offered);
    assert!(pair.link.fetch(pair.now, 0));
    assert_eq!(pair.until_heard(), LinkHeard::Fetched(0));
    assert_eq!(pair.link.fetched(), Some(&*level));
    assert!(pair.sent > 41);
}

#[test]
fn progress_counts_the_bytes_that_have_arrived() {
    let mut pair = Pair::new();
    pair.host
        .share(0, file(3 * WINDOW as usize * PIECE_BYTES))
        .unwrap();
    assert_eq!(pair.until_heard(), LinkHeard::Offered);
    assert_eq!(pair.link.progress(), None);
    assert!(pair.link.fetch(pair.now, 0));
    let whole = 3 * u64::from(WINDOW) * PIECE_BYTES as u64;
    assert_eq!(pair.link.progress(), Some((0, whole)));
    pair.step();
    assert_eq!(pair.link.progress(), Some((whole / 3, whole)));
}

#[test]
fn one_request_is_answered_with_one_window_and_no_more() {
    let mut pair = Pair::new();
    pair.host.share(0, file(100 * PIECE_BYTES)).unwrap();
    assert_eq!(pair.until_heard(), LinkHeard::Offered);
    assert!(pair.link.fetch(pair.now, 0));
    let (_, said) = pair.step();
    let Some(Heard::Fetch(fetch)) = said else {
        panic!("expected a fetch, got {said:?}");
    };
    assert_eq!(pair.sent, WINDOW);
    assert!(!pair.host.write_piece(fetch, WINDOW, &mut pair.to_link));
    assert!(pair.to_link.is_empty());
    assert!(!pair.host.write_piece(fetch, u32::MAX, &mut pair.to_link));
}

#[test]
fn pieces_are_only_sent_to_an_address_that_proved_itself() {
    let mut pair = Pair::new();
    pair.host.share(0, file(5000)).unwrap();
    assert_eq!(pair.until_heard(), LinkHeard::Offered);
    assert!(pair.link.fetch(pair.now, 0));
    assert_eq!(pair.link.update(pair.now, &mut pair.to_host), None);
    assert!(!pair.to_host.is_empty());

    // The same request in another address's name, as a flooder would send.
    assert_eq!(
        pair.host
            .receive(pair.now, address(6000), &pair.to_host, &mut pair.to_link),
        None
    );
    assert!(pair.to_link.is_empty());
    // With a number that was never given.
    let mut forged = pair.to_host.clone();
    forged[10] ^= 1;
    assert_eq!(
        pair.host
            .receive(pair.now, pair.from, &forged, &mut pair.to_link),
        None
    );
    // Long after the number was given.
    let late = pair.now + Duration::from_secs(20);
    assert_eq!(
        pair.host
            .receive(late, pair.from, &pair.to_host, &mut pair.to_link),
        None
    );
    // For a file the server does not offer.
    let mut other = pair.to_host.clone();
    other[18] ^= 1;
    assert_eq!(
        pair.host
            .receive(pair.now, pair.from, &other, &mut pair.to_link),
        None
    );
    // Past the end of the file.
    let mut past = pair.to_host.clone();
    past[26..30].copy_from_slice(&5_u32.to_le_bytes());
    assert_eq!(
        pair.host
            .receive(pair.now, pair.from, &past, &mut pair.to_link),
        None
    );
    // As it was written, it is answered.
    assert!(matches!(
        pair.host
            .receive(pair.now, pair.from, &pair.to_host, &mut pair.to_link),
        Some(Heard::Fetch(_))
    ));
}

#[test]
fn a_file_that_does_not_match_its_hash_ends_the_stay() {
    let mut pair = Pair::new();
    pair.host.share(0, file(3000)).unwrap();
    assert_eq!(pair.until_heard(), LinkHeard::Offered);
    assert!(pair.link.fetch(pair.now, 0));
    assert_eq!(pair.link.update(pair.now, &mut pair.to_host), None);
    let Some(Heard::Fetch(fetch)) =
        pair.host
            .receive(pair.now, pair.from, &pair.to_host, &mut pair.to_link)
    else {
        panic!("expected a fetch");
    };
    let mut heard = Vec::new();
    let mut ahead = 0;
    while pair.host.write_piece(fetch, ahead, &mut pair.to_link) {
        // The server, or somebody on the way, changes one byte.
        if ahead == 1 {
            let last = pair.to_link.len() - 1;
            pair.to_link[last] ^= 0x40;
        }
        heard.extend(pair.link.receive(pair.now, &pair.to_link));
        ahead += 1;
    }
    assert_eq!(heard, [LinkHeard::Closed(Closed::DamagedFile)]);
    assert_eq!(pair.link.closed(), Some(Closed::DamagedFile));
    assert_eq!(pair.link.fetched(), None);
    assert!(
        Closed::DamagedFile
            .to_string()
            .contains("Try joining again")
    );
}

#[test]
fn pieces_that_cannot_be_right_are_ignored() {
    let mut pair = Pair::new();
    let level = file(2 * PIECE_BYTES + 5);
    pair.host.share(0, level.clone()).unwrap();
    assert_eq!(pair.until_heard(), LinkHeard::Offered);
    assert!(pair.link.fetch(pair.now, 0));
    assert_eq!(pair.link.update(pair.now, &mut pair.to_host), None);
    let Some(Heard::Fetch(fetch)) =
        pair.host
            .receive(pair.now, pair.from, &pair.to_host, &mut pair.to_link)
    else {
        panic!("expected a fetch");
    };
    assert!(pair.host.write_piece(fetch, 0, &mut pair.to_link));
    let good = pair.to_link.clone();

    // Too short, too long, for another file, past the end, in another
    // player's name: none is taken.
    let mut short = good.clone();
    short.pop();
    let mut long = good.clone();
    long.push(0);
    let mut other_file = good.clone();
    other_file[10] ^= 1;
    let mut past = good.clone();
    past[18..22].copy_from_slice(&3_u32.to_le_bytes());
    let mut stranger = good.clone();
    stranger[2] ^= 1;
    // The last piece's place holds five bytes, not a full piece.
    let mut misplaced = good.clone();
    misplaced[18..22].copy_from_slice(&2_u32.to_le_bytes());
    for bad in [short, long, other_file, past, stranger, misplaced] {
        assert_eq!(pair.link.receive(pair.now, &bad), None);
        assert_eq!(pair.link.progress().unwrap().0, 0);
    }

    assert_eq!(pair.link.receive(pair.now, &good), None);
    assert_eq!(pair.link.progress().unwrap().0, PIECE_BYTES as u64);
    // The same piece again counts once.
    assert_eq!(pair.link.receive(pair.now, &good), None);
    assert_eq!(pair.link.progress().unwrap().0, PIECE_BYTES as u64);
    assert_eq!(pair.until_heard(), LinkHeard::Fetched(0));
    assert_eq!(pair.link.fetched(), Some(&*level));
}

#[test]
fn a_file_too_long_to_take_is_not_fetched() {
    let mut pair = Pair::new();
    pair.host.share(0, file(10)).unwrap();
    pair.now += STEP;
    assert_eq!(pair.link.update(pair.now, &mut pair.to_host), None);
    pair.host
        .receive(pair.now, pair.from, &pair.to_host, &mut pair.to_link);
    // The server claims a file of 4 GiB less one byte.
    pair.to_link[19..23].copy_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(
        pair.link.receive(pair.now, &pair.to_link),
        Some(LinkHeard::Offered)
    );
    assert_eq!(pair.link.offer().file(0).unwrap().len, u32::MAX);
    assert!(!pair.link.fetch(pair.now, 0));
    assert!(!pair.link.fetch(pair.now, 1));
    assert!(!pair.link.fetch(pair.now, 99));
    assert_eq!(pair.link.progress(), None);
}

#[test]
fn an_offer_that_cannot_be_read_is_ignored() {
    let mut pair = Pair::new();
    pair.host.share(0, file(10)).unwrap();
    pair.now += STEP;
    assert_eq!(pair.link.update(pair.now, &mut pair.to_host), None);
    pair.host
        .receive(pair.now, pair.from, &pair.to_host, &mut pair.to_link);
    // A slot that does not exist.
    let mut slots = pair.to_link.clone();
    slots[18] |= 1 << MAX_SHARED_FILES;
    assert_eq!(pair.link.receive(pair.now, &slots), None);
    // Cut off in the middle of the hash.
    let cut = &pair.to_link[..pair.to_link.len() - 4];
    assert_eq!(pair.link.receive(pair.now, cut), None);
    assert!(pair.link.offer().is_empty());
    assert_eq!(
        pair.link.receive(pair.now, &pair.to_link),
        Some(LinkHeard::Offered)
    );
}

#[test]
fn a_server_that_stops_sending_is_given_up_on() {
    let mut pair = Pair::new();
    pair.host.share(0, file(200 * PIECE_BYTES)).unwrap();
    assert_eq!(pair.until_heard(), LinkHeard::Offered);
    assert!(pair.link.fetch(pair.now, 0));
    pair.step();
    assert!(pair.link.progress().unwrap().0 > 0);
    pair.silent = true;
    let stopped = pair.now;
    assert_eq!(pair.until_heard(), LinkHeard::Closed(Closed::TimedOut));
    let waited = pair.now - stopped;
    assert!(waited > Duration::from_secs(10) && waited < Duration::from_secs(11));
}

#[test]
fn a_fetch_longer_than_a_number_lasts_goes_on_and_still_joins() {
    let mut pair = Pair::new();
    let level = file(2048 * PIECE_BYTES);
    pair.host.share(0, level.clone()).unwrap();
    // Seven pieces in eight are lost, so the fetch takes a long while.
    pair.lose = |sent| sent.wrapping_mul(0x9E37_79B1) >> 29 != 0;
    assert_eq!(pair.until_heard(), LinkHeard::Offered);
    assert!(pair.link.fetch(pair.now, 0));
    assert_eq!(pair.until_heard(), LinkHeard::Fetched(0));
    assert!(pair.now > Duration::from_secs(20), "{:?}", pair.now);
    assert_eq!(pair.link.fetched(), Some(&*level));
    pair.join(0);
}

#[test]
fn a_player_who_takes_a_while_over_a_file_still_joins() {
    let mut pair = Pair::new();
    pair.host.share(0, file(500)).unwrap();
    assert_eq!(pair.until_heard(), LinkHeard::Offered);
    assert!(pair.link.fetch(pair.now, 0));
    assert_eq!(pair.until_heard(), LinkHeard::Fetched(0));
    // Reading the file took longer than the number is good for.
    pair.now += Duration::from_secs(30);
    pair.join(2);
}

#[test]
fn a_file_changed_while_it_is_fetched_is_offered_again() {
    let mut pair = Pair::new();
    pair.host.share(1, file(300 * PIECE_BYTES)).unwrap();
    pair.lose = |sent| sent % 2 == 0;
    assert_eq!(pair.until_heard(), LinkHeard::Offered);
    let old = pair.link.offer().file(1).unwrap();
    assert!(pair.link.fetch(pair.now, 1));
    pair.step();

    let new = file(900);
    pair.host.share(1, new.clone()).unwrap();
    assert_eq!(pair.until_heard(), LinkHeard::Offered);
    assert!(pair.now < Duration::from_secs(5), "{:?}", pair.now);
    assert_ne!(pair.link.offer().file(1), Some(old));
    assert_eq!(pair.link.fetched(), None);
    assert!(pair.link.fetch(pair.now, 1));
    assert_eq!(pair.until_heard(), LinkHeard::Fetched(1));
    assert_eq!(pair.link.fetched(), Some(&*new));
    pair.join(0);
}

#[test]
fn a_slot_is_emptied_and_a_file_that_cannot_be_offered_says_why() {
    let mut host = Host::new(HostSettings::default());
    assert_eq!(
        host.share(MAX_SHARED_FILES, file(1)),
        Err(ShareError::NoSuchSlot)
    );
    assert_eq!(host.share(0, file(0)), Err(ShareError::Empty));
    let big: Arc<[u8]> = vec![0; MAX_SHARED_BYTES + 1].into();
    let error = host.share(0, big).unwrap_err();
    assert_eq!(
        error,
        ShareError::TooBig {
            len: MAX_SHARED_BYTES + 1
        }
    );
    assert!(error.to_string().contains("257 MB"), "{error}");
    assert!(error.to_string().contains("256 MB"), "{error}");

    let mut pair = Pair::new();
    pair.host.share(0, file(10)).unwrap();
    pair.host.unshare(0);
    pair.host.unshare(MAX_SHARED_FILES);
    pair.join(0);
    assert!(pair.link.offer().is_empty());
}

#[test]
fn one_lost_piece_is_asked_for_again_within_a_moment() {
    let mut pair = Pair::new();
    let level = file(64 * PIECE_BYTES);
    pair.host.share(0, level.clone()).unwrap();
    pair.lose = |sent| sent == 5;
    assert_eq!(pair.until_heard(), LinkHeard::Offered);
    let began = pair.now;
    assert!(pair.link.fetch(pair.now, 0));
    assert_eq!(pair.until_heard(), LinkHeard::Fetched(0));
    assert_eq!(pair.link.fetched(), Some(&*level));
    // Three answers and one short wait, not a quarter of a second's.
    assert!(
        pair.now - began < Duration::from_millis(120),
        "{:?}",
        pair.now - began
    );
}

#[test]
fn a_server_that_sends_a_piece_now_and_then_is_given_up_on() {
    let mut pair = Pair::new();
    pair.host.share(0, file(100 * PIECE_BYTES)).unwrap();
    assert_eq!(pair.until_heard(), LinkHeard::Offered);
    let began = pair.now;
    assert!(pair.link.fetch(pair.now, 0));
    assert_eq!(pair.link.update(pair.now, &mut pair.to_host), None);
    let Some(Heard::Fetch(fetch)) =
        pair.host
            .receive(pair.now, pair.from, &pair.to_host, &mut pair.to_link)
    else {
        panic!("expected a fetch");
    };
    // One piece every nine seconds: never silent for the ten that end a
    // stay, and never done.
    let mut ended = None;
    for second in 1..200_u32 {
        pair.now += Duration::from_secs(1);
        if second % 9 == 0 {
            assert!(pair.host.write_piece(fetch, second / 9, &mut pair.to_link));
            assert_eq!(pair.link.receive(pair.now, &pair.to_link), None);
        }
        ended = pair.link.update(pair.now, &mut pair.to_host);
        if ended.is_some() {
            break;
        }
    }
    assert_eq!(ended, Some(Closed::NeverReady));
    let waited = pair.now - began;
    assert!(
        waited > Duration::from_secs(30) && waited < Duration::from_secs(40),
        "{waited:?}"
    );
    assert!(Closed::NeverReady.to_string().contains("too slowly"));
}

#[test]
fn a_server_that_keeps_changing_its_offer_is_given_up_on() {
    let mut pair = Pair::new();
    pair.host.share(0, file(500 * PIECE_BYTES)).unwrap();
    pair.lose = |_| true;
    assert_eq!(pair.until_heard(), LinkHeard::Offered);
    assert!(pair.link.fetch(pair.now, 0));
    let mut offers = 1;
    let ended = loop {
        // Each time the player starts on the file, the server swaps it.
        pair.host
            .share(0, file(500 * PIECE_BYTES + offers))
            .unwrap();
        match pair.until_heard() {
            LinkHeard::Offered => {
                offers += 1;
                assert!(offers < 100, "the player never gave up");
                assert!(pair.link.fetch(pair.now, 0));
            }
            other => break other,
        }
    };
    assert_eq!(ended, LinkHeard::Closed(Closed::NeverReady));
    assert_eq!(offers, 9);
    assert_eq!(pair.link.closed(), Some(Closed::NeverReady));
}
