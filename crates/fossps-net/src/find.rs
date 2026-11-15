//! Finding the servers on the local network without knowing an address.
//!
//! Whoever is looking sends a small question to everyone on the network at
//! once (a broadcast) and to this machine. Every server answers with its
//! name and how many players it has, and the answer's sender is the
//! address to join.
//!
//! The question is padded to [`QUERY_BYTES`] and no answer is longer, so a
//! server cannot be made to send a third party more than was spent on
//! asking. As in [`crate::connect`], nothing here opens a socket:
//! [`crate::UdpSearch`] puts it on a real one.

use std::hash::{BuildHasher, RandomState};
use std::net::SocketAddr;

use crate::snapshot::MAX_PLAYERS;
use crate::wire::{Reader, VERSION, Writer};

/// Somebody asks which servers there are. Padded to [`QUERY_BYTES`].
pub(crate) const QUERY: u8 = 23;
/// A server says what it is.
const INFO: u8 = 24;

/// Four bytes every question carries, so stray traffic is not answered.
const MAGIC: [u8; 4] = *b"FPSF";

/// How long a question is. No answer is longer.
pub(crate) const QUERY_BYTES: usize = 64;

/// The most servers one search gives. A local network has far fewer, and
/// somebody making servers up cannot fill the list without end.
const MOST_FOUND: usize = 64;

/// The most bytes of a server's name that are sent.
pub const MAX_NAME_BYTES: usize = 32;

/// A server that answered a search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundServer {
    /// Where to join it.
    pub address: SocketAddr,
    /// The name the server gave itself, usually its game's name. May be
    /// empty.
    pub name: String,
    /// How many players are on it.
    pub players: u8,
    /// How many players it has room for in all.
    pub places: u8,
    /// Whether it runs the same version of the netcode as this program.
    /// Joining one that does not is refused.
    pub same_version: bool,
}

/// What a server says about itself to whoever is looking.
#[derive(Debug, Clone)]
pub(crate) struct Notice {
    /// A number made up when the server starts, so one server heard at
    /// two of its addresses is counted once.
    id: u64,
    name: String,
    places: u8,
}

impl Notice {
    pub(crate) fn new() -> Self {
        Self {
            id: RandomState::new().hash_one(0_u8),
            name: String::new(),
            places: u8::try_from(MAX_PLAYERS).unwrap_or(u8::MAX),
        }
    }

    /// Sets the name, cut to [`MAX_NAME_BYTES`] at a whole character.
    pub(crate) fn set_name(&mut self, name: &str) {
        let mut end = name.len().min(MAX_NAME_BYTES);
        while !name.is_char_boundary(end) {
            end -= 1;
        }
        self.name.clear();
        self.name.push_str(name.get(..end).unwrap_or_default());
    }

    /// Answers the question in `datagram`, if it is one, into `reply`,
    /// which the caller has emptied. The answer is written whatever
    /// version asked, since a server of another version is worth showing.
    pub(crate) fn answer(&self, datagram: &[u8], players: usize, reply: &mut Vec<u8>) {
        if datagram.len() < QUERY_BYTES {
            return;
        }
        let mut reader = Reader::raw(datagram);
        if reader.u8() != Ok(QUERY) || reader.u8().is_err() {
            return;
        }
        if [reader.u8(), reader.u8(), reader.u8(), reader.u8()] != MAGIC.map(Ok) {
            return;
        }
        let Ok(salt) = reader.u64() else { return };
        let mut writer = Writer::open(reply, INFO);
        writer.u64(salt);
        writer.u64(self.id);
        writer.u8(u8::try_from(players).unwrap_or(u8::MAX));
        writer.u8(self.places);
        writer.u8(u8::try_from(self.name.len()).unwrap_or(0));
        writer.bytes(self.name.as_bytes());
    }
}

/// One search: the question, and who has answered it so far.
#[derive(Debug)]
pub(crate) struct Search {
    /// A number made up for this search. Servers repeat it, so answers to
    /// somebody else's question, or made up by a stranger who cannot see
    /// the question, are not believed.
    salt: u64,
    /// The servers heard from already.
    seen: Vec<u64>,
}

impl Search {
    pub(crate) fn new() -> Self {
        Self {
            salt: RandomState::new().hash_one(1_u8),
            seen: Vec::new(),
        }
    }

    /// Writes the question to send.
    pub(crate) fn write_query(&self, out: &mut Vec<u8>) {
        let mut writer = Writer::open(out, QUERY);
        writer.bytes(&MAGIC);
        writer.u64(self.salt);
        writer.pad_to(QUERY_BYTES);
    }

    /// Reads one datagram that came from `from`: a server not heard from
    /// before in this search, or nothing.
    pub(crate) fn receive(&mut self, from: SocketAddr, datagram: &[u8]) -> Option<FoundServer> {
        if datagram.len() > QUERY_BYTES {
            return None;
        }
        let mut reader = Reader::raw(datagram);
        if reader.u8().ok()? != INFO {
            return None;
        }
        let version = reader.u8().ok()?;
        if reader.u64().ok()? != self.salt {
            return None;
        }
        let id = reader.u64().ok()?;
        let players = reader.u8().ok()?;
        let places = reader.u8().ok()?;
        let len = usize::from(reader.u8().ok()?);
        let name = reader.rest().get(..len.min(MAX_NAME_BYTES))?;
        if self.seen.contains(&id) || self.seen.len() >= MOST_FOUND {
            return None;
        }
        self.seen.push(id);
        Some(FoundServer {
            address: from,
            // A name is shown in a terminal: nothing in it may move the
            // cursor, ring the bell, hide, or turn the line's order round.
            name: String::from_utf8_lossy(name)
                .chars()
                .filter(|letter| {
                    !letter.is_control()
                        && !matches!(
                            letter,
                            '\u{200B}'..='\u{200F}'
                                | '\u{202A}'..='\u{202E}'
                                | '\u{2066}'..='\u{2069}'
                                | '\u{FEFF}'
                        )
                })
                .collect(),
            players,
            places,
            same_version: version == VERSION,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn address(last: u8) -> SocketAddr {
        SocketAddr::from(([192, 168, 1, last], 24680))
    }

    fn asked(search: &Search) -> Vec<u8> {
        let mut query = Vec::new();
        search.write_query(&mut query);
        query
    }

    fn answered(notice: &Notice, query: &[u8], players: usize) -> Vec<u8> {
        let mut reply = Vec::new();
        notice.answer(query, players, &mut reply);
        reply
    }

    #[test]
    fn a_server_answers_with_its_name_and_how_full_it_is() {
        let mut notice = Notice::new();
        notice.set_name("zombie-night");
        let mut search = Search::new();

        let reply = answered(&notice, &asked(&search), 3);

        assert_eq!(
            search.receive(address(20), &reply),
            Some(FoundServer {
                address: address(20),
                name: "zombie-night".to_owned(),
                players: 3,
                places: 64,
                same_version: true,
            })
        );
    }

    #[test]
    fn no_answer_is_longer_than_the_question() {
        let mut notice = Notice::new();
        // Four bytes a letter, and 33 bytes would split the ninth.
        notice.set_name(&"🎯".repeat(20));
        assert_eq!(notice.name, "🎯".repeat(8));
        let search = Search::new();
        let query = asked(&search);
        assert_eq!(query.len(), QUERY_BYTES);

        let reply = answered(&notice, &query, 64);
        assert!(!reply.is_empty() && reply.len() <= query.len());

        // A question cut short gets no answer at all.
        assert!(answered(&notice, &query[..QUERY_BYTES - 1], 0).is_empty());
    }

    #[test]
    fn one_server_heard_at_two_addresses_is_found_once() {
        let notice = Notice::new();
        let mut search = Search::new();
        let reply = answered(&notice, &asked(&search), 0);

        assert!(search.receive(address(20), &reply).is_some());
        assert_eq!(search.receive(address(21), &reply), None);

        let other = answered(&Notice::new(), &asked(&search), 0);
        assert!(search.receive(address(22), &other).is_some());
    }

    #[test]
    fn an_answer_to_another_search_is_not_believed() {
        let notice = Notice::new();
        let reply = answered(&notice, &asked(&Search::new()), 0);

        assert_eq!(Search::new().receive(address(20), &reply), None);
    }

    #[test]
    fn a_server_of_another_version_is_found_and_marked() {
        let notice = Notice::new();
        let mut search = Search::new();
        let mut query = asked(&search);
        query[1] = VERSION + 1;
        let mut reply = answered(&notice, &query, 0);
        assert!(!reply.is_empty(), "another version's question is answered");
        reply[1] = VERSION + 1;

        let found = search.receive(address(20), &reply).unwrap();
        assert!(!found.same_version);
    }

    #[test]
    fn rubbish_is_ignored_and_a_name_cannot_carry_control_characters() {
        let notice = Notice::new();
        let mut search = Search::new();
        assert!(answered(&notice, &[0; QUERY_BYTES], 0).is_empty());
        assert!(answered(&notice, &[], 0).is_empty());
        assert_eq!(search.receive(address(20), &[]), None);
        assert_eq!(search.receive(address(20), &[INFO; 200]), None);

        let mut reply = answered(&notice, &asked(&search), 0);
        // A name that claims more bytes than came is not read.
        let last = reply.len() - 1;
        reply[last] = 9;
        assert_eq!(search.receive(address(20), &reply), None);

        reply[last] = 6;
        reply.extend_from_slice("a\x1b\u{202E}b".as_bytes());
        assert_eq!(search.receive(address(20), &reply).unwrap().name, "ab");
    }

    #[test]
    fn a_search_stops_at_sixty_four_servers() {
        let mut search = Search::new();
        let query = asked(&search);
        let found = (0..100)
            .filter(|_| {
                let reply = answered(&Notice::new(), &query, 0);
                search.receive(address(20), &reply).is_some()
            })
            .count();
        assert_eq!(found, MOST_FOUND);
    }
}
