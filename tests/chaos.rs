// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The Chaosnet's own parts, held to their sources without a board: the
//! PROM images MIT burned against the tables they were programmed from,
//! and, as the layers come, the packet, the wire and the transport.

use muir::chaos::ether::Node;
use muir::chaos::packet::{Framed, Packet};
use muir::chaos::server::{self, Out, Response, Server, Service, Session, op};
use muir::chaos::status::Status;
use muir::chaos::time::Time;
use muir::chaos::{Config, packet, wire};
use muir::prom::parse_mit;

mod support;
use support::release;

/// **An address is read either way it is written.** A Chaosnet address is
/// sixteen bits, the subnet in the high byte and the host in the low, and
/// the memo and the host tables write the whole number in octal --- where
/// the byte boundary falls inside a digit, so `3050` hides "subnet 6, host
/// 50". The flags take both spellings, each half in octal too.
#[test]
fn an_address_is_read_either_way_it_is_written() {
    use muir::chaos::parse_address;
    assert_eq!(parse_address("3050"), Some(0o3050));
    assert_eq!(parse_address("6:50"), Some(0o3050), "subnet 6, host 50");
    assert_eq!(parse_address("6:60"), Some(0o3060), "and the server beside it");
    assert_eq!(parse_address("23:6"), Some(0o11406), "MIT's own OZ");
    // Subnet 376 is Chaosnet's private range, the 192.168 of it: the
    // Global Chaosnet's own documentation says "no routing information
    // about that subnet should be sent outside that subnet, no packets
    // from that subnet should be sent to other subnets, and any packets
    // received from that subnet on another subnet should be dropped". A
    // local bridge and the emulators behind it live in 177001..177377.
    assert_eq!(parse_address("376:41"), Some(0o177041), "the private subnet");
    assert_eq!(parse_address("177001"), Some(0o177001), "its first host");
    assert_eq!(parse_address("177377"), Some(0o177377), "and its last");
    // Neither half may be zero: a zero host is the subnet's broadcast, not
    // a host, and MIT's interface takes the destination word "or 0 to
    // broadcast it". A machine configured as one would take the whole
    // subnet's traffic for its own.
    assert_eq!(parse_address("0:0"), None, "not a host");
    assert_eq!(parse_address("6:0"), None, "a zero host is subnet 6's broadcast");
    assert_eq!(parse_address("0:50"), None, "and a zero subnet is no subnet");
    assert_eq!(parse_address("0"), None, "the bare form too");
    assert_eq!(parse_address("377"), None, "which is every bare octal below 400");
    assert_eq!(parse_address("26000"), None, "subnet 54's broadcast");
    assert_eq!(parse_address("26001"), Some(0o26001), "and its first host");
    assert_eq!(parse_address("26377"), Some(0o26377), "and its last");
    assert_eq!(parse_address("377:377"), Some(0o177777));
    assert_eq!(parse_address("400:1"), None, "a subnet is eight bits");
    assert_eq!(parse_address("6:400"), None, "and so is a host");
    assert_eq!(parse_address("6:"), None);
    assert_eq!(parse_address(":50"), None);
    assert_eq!(parse_address("6:50:1"), None);
    assert_eq!(parse_address("3058"), None, "octal");
    assert_eq!(parse_address("6:58"), None, "in both halves");
    assert_eq!(parse_address("200000"), None, "sixteen bits");
    assert_eq!(parse_address(""), None);
}

/// **The band's own hosts are named by the run, and are not the
/// defaults.** Two facts, and they are asserted together so that they
/// stay apart on purpose rather than by accident.
///
/// The first: System 100's band is `MIT-LISPM-1` at 3050 and calls its
/// file and time host `MIT-OZ` at 3060 --- `MIT-OZ` being the `SYS` host
/// of `site.lisp`, where the boot goes for its time and its files. The
/// release's builders trimmed that table to exactly those two hosts
/// (discrepancy 60), so the band expects them. [`support::CHAOS_100`] is
/// the pair every test that boots this band hands it, and it is read off
/// `sys/site/hosts.text` here so that a change on either side shows.
///
/// The second: `Config::default()` is not that pair and is no band's. It
/// is this machine at 177001 with its server at 177002, subnet 376, the
/// Chaosnet's private and non-routable range, so that a run started with
/// no `--chaos-address` cannot answer at an address a real Chaosnet
/// allocated to someone else. muir models the CADR and not one
/// distribution of it, and a default out of one band's host table would
/// be the wrong default for every other band.
#[test]
fn the_bands_own_hosts_are_named_and_are_not_the_defaults() {
    let d = Config::default();
    assert_eq!(
        (d.address, d.server_address),
        (0o177001, 0o177002),
        "the defaults are the private subnet's, not a band's"
    );
    assert_eq!((d.address >> 8, d.server_address >> 8), (0o376, 0o376), "both on subnet 376");

    let Some(text) = release("site/hosts.text") else { return };
    // `HOST MIT-OZ,<tabs>CHAOS 3060,SERVER,UNIX,VAX,[OZ]`
    let address = |host: &str| -> u16 {
        let line = text
            .lines()
            .find(|l| l.starts_with(&format!("HOST {host},")))
            .unwrap_or_else(|| panic!("{host} is in hosts.text"));
        let after = line.split("CHAOS ").nth(1).expect("a CHAOS address");
        let octal: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
        u16::from_str_radix(&octal, 8).unwrap()
    };
    let named: Vec<&str> = text
        .lines()
        .filter_map(|l| l.strip_prefix("HOST "))
        .filter_map(|l| l.split(',').next())
        .collect();
    assert_eq!(
        named,
        ["MIT-LISPM-1", "MIT-OZ"],
        "the builders trimmed the table to two, so nothing here is on subnet 376"
    );
    assert_eq!(
        support::CHAOS_100,
        (address("MIT-LISPM-1"), address("MIT-OZ")),
        "what a run hands this band is what the band's own host table gives it"
    );
    assert_ne!(
        support::CHAOS_100,
        (d.address, d.server_address),
        "and it is handed to the band, never defaulted to"
    );
}

/// Reads a `.promt` table: rows of 0/1 columns, `inputs` of them then
/// the output columns; comment and heading lines are skipped. Returns
/// each row's input bits and output bits, in column order.
fn table(text: &str, inputs: usize, outputs: usize) -> Vec<(Vec<u8>, Vec<u8>)> {
    text.lines()
        .filter_map(|l| {
            let cols: Vec<&str> = l.split(';').next().unwrap().split_whitespace().collect();
            if cols.len() < inputs + outputs || !cols.iter().all(|c| c == &"0" || c == &"1") {
                return None;
            }
            let bit = |c: &str| c.parse::<u8>().unwrap();
            Some((
                cols[..inputs].iter().map(|c| bit(c)).collect(),
                cols[inputs..inputs + outputs].iter().map(|c| bit(c)).collect(),
            ))
        })
        .collect()
}

/// **The modulator's PROM is its table.** `mit/chaos/lmmodu.promt` lays out
/// the 32 x 8 state machine at LMMODU 0A10 by input --- pins 14 to 10,
/// `BSY DAT TS3 TS2 TS1`, the address from its top bit down --- with two
/// output columns, "NORMAL OUT" and "PRECHARGE OUT", each `PCE -CK DO TS3
/// TS2 TS1` for outputs 6 to 1, pins 6 to 1, bits 5 to 0. `lmmodu.prom`
/// says "NORMAL VERSION" and `lmmodu.prom2` "PRECHARGE VERSION", and
/// each must be its column exactly: two files that reached us by
/// different routes, agreeing.
#[test]
fn the_modulator_prom_is_its_table() {
    let rows = table(include_str!("../mit/chaos/lmmodu.promt"), 5, 12);
    assert_eq!(rows.len(), 32, "thirty-two rows of five inputs");
    let normal = parse_mit(include_str!("../mit/chaos/lmmodu.prom")).unwrap();
    let precharge = parse_mit(include_str!("../mit/chaos/lmmodu.prom2")).unwrap();
    assert_eq!((normal.len(), precharge.len()), (32, 32));
    for (ins, outs) in &rows {
        let address = ins.iter().fold(0usize, |a, &b| a << 1 | b as usize);
        let word = |bits: &[u8]| bits.iter().fold(0u8, |a, &b| a << 1 | b);
        let (want_normal, want_precharge) = (word(&outs[..6]), word(&outs[6..]));
        assert_eq!(
            normal[address], want_normal,
            "normal version at {address:o}: image {:o}, table {want_normal:o}",
            normal[address]
        );
        assert_eq!(
            precharge[address], want_precharge,
            "precharge version at {address:o}: image {:o}, table {want_precharge:o}",
            precharge[address]
        );
    }
    // And the two versions differ only in the precharge-enable bit and the
    // states that use it, never in the clock: `-CK`, bit 4, is the
    // transmitter's output clock either way.
    for a in 0..32 {
        assert_eq!(normal[a] & 0o20, precharge[a] & 0o20, "-CK differs at {a:o}");
    }
}

/// **The address comparator's PROM is its table, and the table says how
/// the chip is addressed.** `mit/chaos/lmmynm.promt` lists the 256 x 4 PROM at
/// LMMYNM 0D01 by its input *pins* --- `15 1 2 3 4 7 6 5`, which the
/// netlist wires to `RS2 RS1 PRBCT5 PRBCT4 PRIWBEG MATCH.ANY.DEST RDATA
/// MY#.BIT` --- and its output pins `9 10 11 12`. `lmmynm.prom` is by
/// address, and the two agree only when the first printed input column
/// is the address's top bit and the last its bottom, and the last output
/// column its bottom bit: so pin 5 is `A0`, 6 is `A1`, 7 is `A2`, 4 is
/// `A3`, 3 is `A4`, 2 is `A5`, 1 is `A6`, 15 is `A7`, and `DO1` on pin 12
/// is the low bit. That is the 256 x 4 PROM family's pinout, which the
/// 1975 TI sheet's `AD A`..`AD H` lettering does not spell out, and it is
/// what `part::pinout("74S287")` follows.
#[test]
fn the_address_prom_is_its_table() {
    let rows = table(include_str!("../mit/chaos/lmmynm.promt"), 8, 4);
    assert_eq!(rows.len(), 256, "two hundred and fifty-six rows of eight inputs");
    let image = parse_mit(include_str!("../mit/chaos/lmmynm.prom")).unwrap();
    assert_eq!(image.len(), 256);
    for (ins, outs) in &rows {
        let address = ins.iter().fold(0usize, |a, &b| a << 1 | b as usize);
        let want = outs.iter().fold(0u8, |a, &b| a << 1 | b);
        assert_eq!(
            image[address], want,
            "at {address:o}: image {:o}, table {want:o} (inputs {ins:?})",
            image[address]
        );
    }
}

/// **The check word is the board's.** The words `a_packet_loops_back_through_the_board`
/// writes, with the source address the hardware appends, and the check
/// word the netlist board produced for them: `135771`. Of the 9401's
/// polynomials, the two bit orders and the two seeds, only CRC-16 from a
/// cleared register over the words most-significant bit first gives it.
#[test]
fn the_check_word_is_the_boards() {
    let words = [0o400, 0o4, 0o3050, 0, 0o3050, 0o21, 1, 0, 0o44524, 0o42515, 0o3050, 0o3050];
    assert_eq!(packet::check_word(&words), 0o135771);
}

/// A packet's words and bytes, there and back, odd byte count included.
#[test]
fn a_packet_goes_to_words_and_back() {
    let p = packet::Packet {
        opcode: 1,
        forward: 0,
        dest: 0o3060,
        dest_index: 0,
        source: 0o3050,
        source_index: 0o21,
        number: 1,
        ack: 0,
        data: b"TIME".to_vec(),
    };
    let buffer = p.to_buffer(0o3060);
    assert_eq!(&buffer[..8], &[0o400, 4, 0o3060, 0, 0o3050, 0o21, 1, 0]);
    assert_eq!(&buffer[8..], &[0o44524, 0o42515, 0o3060], "TI, ME, then the cable destination");
    let (back, dest) = packet::Packet::from_buffer(&buffer).unwrap();
    assert_eq!((back, dest), (p.clone(), 0o3060));
    let odd = packet::Packet { data: b"HELLO".to_vec(), ..p };
    let (back, _) = packet::Packet::from_buffer(&odd.to_buffer(0)).unwrap();
    assert_eq!(back.data, b"HELLO");
}

/// **Framing, coding, decoding and unframing round trip**, and the bits
/// on the cable are in AIM-628's order: check word first, then source,
/// destination, the data words backwards, each least-significant bit
/// first, a zero last.
#[test]
fn a_frame_survives_the_wire() {
    let buffer = [0o400, 0o4, 0o3050, 0, 0o3050, 0o21, 1, 0, 0o44524, 0o42515, 0o3050];
    let bits = packet::frame(&buffer, 0o3050);
    assert_eq!(bits.len(), 13 * 16 + 1, "thirteen words and the trailing zero");
    // The first sixteen bits on the cable are the check word, low bit first.
    let first: u16 = bits[..16].iter().rev().fold(0, |w, &b| w << 1 | b as u16);
    assert_eq!(first, 0o135771, "the check word goes first");
    let second: u16 = bits[16..32].iter().rev().fold(0, |w, &b| w << 1 | b as u16);
    assert_eq!(second, 0o3050, "then the source");
    assert!(!bits[bits.len() - 1], "and a zero last");
    let changes = wire::encode(&bits);
    // Every cell opens with an edge; a bit equal to the last adds one.
    assert_eq!(changes[0], (0, true));
    let packets = wire::decode(&changes);
    assert_eq!(packets.len(), 1, "one packet off the line");
    let framed = packet::unframe(&packets[0]).unwrap();
    assert_eq!(framed.buffer, buffer);
    assert_eq!((framed.source, framed.check, framed.check_ok), (0o3050, 0o135771, true));
    // A flipped bit is caught.
    let mut bad = packets[0].clone();
    bad[40] = !bad[40];
    assert!(!packet::unframe(&bad).unwrap().check_ok);
}

/// A packet as the ether would hand it to a node, from `from` at `index`.
fn arriving(p: &Packet) -> Framed {
    let buffer = p.to_buffer(p.dest);
    let mut over = buffer.clone();
    over.push(p.source);
    let check = packet::check_word(&over);
    Framed { buffer, source: p.source, check, check_ok: true }
}

fn rfc(from: (u16, u16), to: u16, number: u16, text: &str) -> Packet {
    Packet {
        opcode: op::RFC,
        forward: 0,
        dest: to,
        dest_index: 0,
        source: from.0,
        source_index: from.1,
        number,
        ack: 0,
        data: text.as_bytes().to_vec(),
    }
}

/// What the host sends next, as a packet.
fn next_from(h: &mut Server, now: u64) -> Option<Packet> {
    h.transmit(now).map(|b| Packet::from_buffer(&b).unwrap().0)
}

/// The Chaosnet server serves STATUS, under the band's own name for it.
///
/// `HOST-UP-P` asks every host it is given for `STATUS` and counts the ones
/// that answer, so a host that does not serve it is a host the machine
/// believes is down.
#[test]
fn the_chaosnet_server_serves_status_as_mit_oz() {
    let d = Config::default();
    let mut h = d.server(0);
    h.receive(100, &arriving(&rfc((d.address, 3), d.server_address, 1, "STATUS")));
    let ans = next_from(&mut h, 100).expect("MIT-OZ answers STATUS");
    assert_eq!(ans.opcode, op::ANS);
    let end = ans.data[..32].iter().position(|&b| b == 0).expect("a name");
    assert_eq!(&ans.data[..end], b"MIT-OZ", "the name the server was given");
    let id = u16::from_le_bytes([ans.data[32], ans.data[33]]);
    assert_eq!(
        id,
        0o400 + (d.server_address >> 8),
        "the subnet is the address's high byte, 376 for the defaults"
    );
}

/// **STATUS is a simple transaction, and HOSTAT is what reads it.**
///
/// AIM-628 §5: an RFC to `STATUS` evokes an ANS carrying the server's name in
/// the first 32 bytes, padded with nulls, and then one block per subnet the
/// host has meters for. The format of a block is not guesswork --- MIT's own
/// reader is `HOSTAT-FORMAT-ANS-1` in `sys/network/chaos/chsaux.lisp`, and
/// this asserts exactly what it decodes:
///
/// - the name is the bytes up to the first null in the first 32
///   (`STRING-SEARCH-CHAR 0 ... 0 32.`);
/// - a block begins at data word 16, which is where its `(I 24. ...)`
///   starts once the eight header words are taken off;
/// - a block is an identifier word and a count word, and the count is in
///   *16-bit words*, so the loop steps `(+ I 2 CT)`;
/// - an identifier of `0o400` and up is a subnet block for subnet
///   `ID - 0o400`, whose meters are 32 bits each, low word first ---
///   `(DPB (AREF PKT (1+ J)) #o2020 (AREF PKT J))`.
///
/// The meters are zero because the Chaosnet server keeps no per-subnet counters;
/// what it can answer truthfully is its name and which subnet it is on.
#[test]
fn status_answers_what_hostat_reads() {
    let mut h = Server::new(0o3060);
    h.serve(Box::new(Status::new("MIT-OZ")));
    h.receive(100, &arriving(&rfc((0o3050, 7), 0o3060, 0o1234, "STATUS")));
    let ans = next_from(&mut h, 100).expect("an answer");

    assert_eq!(ans.opcode, op::ANS);
    assert_eq!((ans.dest, ans.dest_index), (0o3050, 7), "back to the asker's index");
    assert_eq!(ans.ack, 0o1234, "acknowledging the RFC");
    assert_eq!(h.connections(), 0, "no connection was made");

    // The name: bytes to the first null, within the first 32.
    let d = &ans.data;
    assert!(d.len() >= 32, "the name field is 32 bytes");
    let end = d[..32].iter().position(|&b| b == 0).expect("null-terminated within 32");
    assert_eq!(&d[..end], b"MIT-OZ");
    assert!(d[end..32].iter().all(|&b| b == 0), "padded with nulls, not spaces");

    // One subnet block, read the way HOSTAT reads it.
    let word = |i: usize| u16::from_le_bytes([d[2 * i], d[2 * i + 1]]) as usize;
    let (id, count) = (word(16), word(17));
    assert_eq!(id, 0o400 + 6, "subnet 6, the band's own, in the 0o400 form");
    assert_eq!(count % 2, 0, "a 32-bit meter is two words");
    assert_eq!(d.len(), 32 + 4 + count * 2, "the block is all there is");

    let meter = |k: usize| word(18 + 2 * k) | word(19 + 2 * k) << 16;
    for k in 0..count / 2 {
        assert_eq!(meter(k), 0, "meter {k} is zero: the Chaosnet server keeps none");
    }
}

/// **TIME is a simple transaction.** AIM-628 §5.8: an RFC to `TIME`
/// evokes an ANS with the universal time in four bytes, least
/// significant first, and no connection. The ANS goes back to the
/// asker's index, acknowledging the RFC.
#[test]
fn time_answers_a_simple_transaction() {
    let mut h = Server::new(0o3060);
    h.serve(Box::new(Time::fixed(0x1234_5678)));
    h.receive(100, &arriving(&rfc((0o3050, 7), 0o3060, 0o1234, "TIME")));
    let ans = next_from(&mut h, 100).expect("an answer");
    assert_eq!(ans.opcode, op::ANS);
    assert_eq!((ans.dest, ans.dest_index), (0o3050, 7), "back to the asker's index");
    assert_eq!(ans.source, 0o3060);
    assert_eq!(ans.ack, 0o1234, "acknowledging the RFC");
    assert_eq!(ans.data, [0x78, 0x56, 0x34, 0x12], "least significant byte first");
    assert_eq!(next_from(&mut h, 100), None, "and nothing more");
    assert_eq!(h.connections(), 0, "no connection was made");
    // A packet for another host is not ours.
    h.receive(200, &arriving(&rfc((0o3050, 8), 0o3070, 1, "TIME")));
    assert_eq!(next_from(&mut h, 200), None);
    // An unknown contact is refused with a CLS.
    h.receive(300, &arriving(&rfc((0o3050, 9), 0o3060, 2, "NOSUCH")));
    let cls = next_from(&mut h, 300).expect("a refusal");
    assert_eq!(cls.opcode, op::CLS);
    assert_eq!((cls.dest, cls.dest_index), (0o3050, 9));
    assert!(String::from_utf8_lossy(&cls.data).contains("NOSUCH"), "with the reason");
    // A broadcast TIME is answered too, §4.5: "The TIME and STATUS protocols
    // ... will work through BRD packets"; the subnet bit map is skipped.
    let mut brd = rfc((0o3050, 10), 0, 3, "TIME");
    brd.opcode = op::BRD;
    brd.ack = 4;
    brd.data = [vec![0xff, 0xff, 0xff, 0xff], b"TIME".to_vec()].concat();
    h.receive(400, &arriving(&brd));
    let ans = next_from(&mut h, 400).expect("an answer to a broadcast");
    assert_eq!((ans.opcode, ans.dest_index), (op::ANS, 10));
}

/// A service that echoes what it is sent, for the stream protocol.
struct Echo;
struct EchoSession {
    pending: Vec<Out>,
    got_eof: bool,
}
impl Service for Echo {
    fn contact(&self) -> &str {
        "ECHO"
    }
    fn request(&mut self, _now: u64, args: &str, _from: (u16, u16)) -> Response {
        if args == "NO" {
            return Response::Refuse("Not today".into());
        }
        if args == "LONG" {
            return Response::Refuse("not today, and at length ".repeat(40));
        }
        Response::Accept(Box::new(EchoSession { pending: Vec::new(), got_eof: false }))
    }
}
impl Session for EchoSession {
    fn data(&mut self, _now: u64, _op: u8, bytes: &[u8]) {
        self.pending.push(Out::Data(bytes.to_vec()));
    }
    fn eof(&mut self, _now: u64) {
        self.got_eof = true;
        self.pending.push(Out::Eof);
    }
    fn closed(&mut self, _now: u64, _reason: &str) {}
    fn poll(&mut self, _now: u64) -> Vec<Out> {
        std::mem::take(&mut self.pending)
    }
}

/// **A stream connection opens, moves data both ways and closes**, as
/// AIM-628 §4.1 to §4.4 lay it out: RFC, then the server's OPN carrying
/// its index, initial packet number and window and acknowledging the
/// RFC; the user's STS; numbered data acknowledged in the header of what
/// goes back or in an STS; EOF answered by EOF; CLS.
#[test]
fn a_stream_opens_moves_data_and_closes() {
    let mut h = Server::new(0o3060);
    h.serve(Box::new(Echo));
    let me = (0o3050, 0o21);
    h.receive(0, &arriving(&rfc(me, 0o3060, 100, "ECHO")));
    let opn = next_from(&mut h, 0).expect("an OPN");
    assert_eq!(opn.opcode, op::OPN);
    assert_eq!((opn.dest, opn.dest_index), me);
    assert_ne!(opn.source_index, 0, "the server's index");
    assert_eq!(opn.ack, 100, "acknowledging the RFC");
    let receipt = u16::from_le_bytes([opn.data[0], opn.data[1]]);
    let window = u16::from_le_bytes([opn.data[2], opn.data[3]]);
    assert_eq!(receipt, 100, "the OPN's data is a receipt");
    assert!(window >= 1, "and a window");
    assert_eq!(h.connections(), 1);
    let server = (0o3060, opn.source_index);
    let mut number = 100u16;
    let mut sts = Packet {
        opcode: op::STS,
        forward: 0,
        dest: server.0,
        dest_index: server.1,
        source: me.0,
        source_index: me.1,
        number: number + 1,
        ack: opn.number,
        data: Vec::new(),
    };
    sts.data.extend_from_slice(&opn.number.to_le_bytes());
    sts.data.extend_from_slice(&5u16.to_le_bytes());
    h.receive(10, &arriving(&sts));
    assert_eq!(next_from(&mut h, 10), None, "an STS wants nothing back");
    // Data in: echoed back, numbered from the OPN's number on, and
    // acknowledging ours.
    number += 1;
    let dat = Packet {
        opcode: op::DAT,
        forward: 0,
        dest: server.0,
        dest_index: server.1,
        source: me.0,
        source_index: me.1,
        number,
        ack: opn.number,
        data: b"hello".to_vec(),
    };
    h.receive(20, &arriving(&dat));
    let echo = next_from(&mut h, 20).expect("the echo");
    assert_eq!(echo.opcode, op::DAT);
    assert_eq!(echo.data, b"hello");
    assert_eq!(echo.number, opn.number.wrapping_add(1), "numbered after the OPN");
    assert_eq!(echo.ack, number, "acknowledging our data");
    assert_eq!(next_from(&mut h, 20), None, "the acknowledgement rode on the echo, so no STS");
    // A duplicate of our data draws an STS with a receipt, not another echo.
    h.receive(30, &arriving(&dat));
    let s = next_from(&mut h, 30).expect("an STS for the duplicate");
    assert_eq!(s.opcode, op::STS);
    assert_eq!(u16::from_le_bytes([s.data[0], s.data[1]]), number, "receipting through our packet");
    // EOF in, EOF back.
    number += 1;
    let eof = Packet {
        opcode: op::EOF,
        forward: 0,
        dest: server.0,
        dest_index: server.1,
        source: me.0,
        source_index: me.1,
        number,
        ack: echo.number,
        data: Vec::new(),
    };
    h.receive(40, &arriving(&eof));
    let back = next_from(&mut h, 40).expect("an EOF back");
    assert_eq!(back.opcode, op::EOF);
    assert_eq!(back.ack, number);
    // CLS ends it.
    let cls = Packet {
        opcode: op::CLS,
        forward: 0,
        dest: server.0,
        dest_index: server.1,
        source: me.0,
        source_index: me.1,
        number: number + 1,
        ack: back.number,
        data: b"done".to_vec(),
    };
    h.receive(50, &arriving(&cls));
    assert_eq!(h.connections(), 0, "closed");
    // A refusal.
    h.receive(60, &arriving(&rfc((0o3050, 0o22), 0o3060, 200, "ECHO NO")));
    let cls = next_from(&mut h, 60).unwrap();
    assert_eq!((cls.opcode, cls.dest_index), (op::CLS, 0o22));
    assert_eq!(cls.data, b"Not today");
    // Unreceipted packets go again after half a second.
    h.receive(70, &arriving(&rfc((0o3050, 0o23), 0o3060, 300, "ECHO")));
    let opn2 = next_from(&mut h, 70).unwrap();
    assert_eq!(next_from(&mut h, 70 + server::RETRANSMIT_NS - 1), None);
    let again = next_from(&mut h, 70 + server::RETRANSMIT_NS).expect("retransmitted");
    assert_eq!((again.opcode, again.number), (op::OPN, opn2.number));
}

use muir::chaos::file::{self, File, NEWLINE};

/// A user end for the FILE protocol: enough of the transport to open the
/// control connection, listen for the data connection the server calls,
/// send commands and read what comes back on both.
struct Client {
    me: u16,
    server: u16,
    control: Option<Link>,
    data: Option<Link>,
    /// The contact name the data connection will be called on.
    listening: Option<String>,
    next_index: u16,
    /// Replies on the control connection, as text.
    replies: Vec<String>,
    /// What came down the data connection: `(opcode, bytes)`.
    down: Vec<(u8, Vec<u8>)>,
}

struct Link {
    mine: u16,
    theirs: u16,
    number: u16,
    last_received: u16,
}

fn text(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| b as char).collect()
}

impl Client {
    fn new(me: u16, server: u16) -> Client {
        Client {
            me,
            server,
            control: None,
            data: None,
            listening: None,
            next_index: 0o21,
            replies: Vec::new(),
            down: Vec::new(),
        }
    }

    fn packet(&self, link: &Link, opcode: u8, data: Vec<u8>) -> Packet {
        Packet {
            opcode,
            forward: 0,
            dest: self.server,
            dest_index: link.theirs,
            source: self.me,
            source_index: link.mine,
            number: link.number,
            ack: link.last_received,
            data,
        }
    }

    /// Opens the control connection: RFC `FILE 1`, the OPN back, an STS.
    fn connect(&mut self, h: &mut Server, now: u64) {
        let mine = self.next_index;
        self.next_index += 1;
        let rfc = rfc((self.me, mine), self.server, 100, "FILE 1");
        h.receive(now, &arriving(&rfc));
        let opn = next_from(h, now).expect("an OPN for FILE");
        assert_eq!((opn.opcode, opn.dest_index), (op::OPN, mine));
        let mut link =
            Link { mine, theirs: opn.source_index, number: 101, last_received: opn.number };
        let mut sts = self.packet(&link, op::STS, Vec::new());
        sts.data.extend_from_slice(&opn.number.to_le_bytes());
        sts.data.extend_from_slice(&5u16.to_le_bytes());
        h.receive(now, &arriving(&sts));
        link.number = 101;
        self.control = Some(link);
        self.drain(h, now);
    }

    /// Takes everything the server has to send, answering as a user end
    /// would: the RFC for the data connection with an OPN, and data with
    /// receipts.
    fn drain(&mut self, h: &mut Server, now: u64) {
        loop {
            let mut got_data = false;
            while let Some(p) = next_from(h, now) {
                if p.opcode == op::RFC {
                    // The server calling our listening contact: accept.
                    let contact = text(&p.data);
                    assert_eq!(Some(&contact), self.listening.as_ref(), "an RFC for {contact}");
                    let mine = self.next_index;
                    self.next_index += 1;
                    let link =
                        Link { mine, theirs: p.source_index, number: 500, last_received: p.number };
                    let mut opn = self.packet(&link, op::OPN, Vec::new());
                    opn.data.extend_from_slice(&p.number.to_le_bytes());
                    opn.data.extend_from_slice(&15u16.to_le_bytes());
                    h.receive(now, &arriving(&opn));
                    self.data = Some(Link { number: 501, ..link });
                    continue;
                }
                let which = if self.control.as_ref().is_some_and(|l| l.mine == p.dest_index) {
                    0
                } else if self.data.as_ref().is_some_and(|l| l.mine == p.dest_index) {
                    1
                } else {
                    panic!("a packet for index {} which is nobody's: {p:?}", p.dest_index);
                };
                let link =
                    if which == 0 { self.control.as_mut() } else { self.data.as_mut() }.unwrap();
                if op::is_controlled(p.opcode) {
                    assert_eq!(p.number, link.last_received.wrapping_add(1), "in order on {which}");
                    link.last_received = p.number;
                    got_data = true;
                    if which == 0 {
                        self.replies.push(text(&p.data));
                    } else {
                        self.down.push((p.opcode, p.data.clone()));
                    }
                }
            }
            if !got_data {
                break;
            }
            // Receipt what came, on both, so the window opens again.
            for which in 0..2 {
                let Some(link) =
                    (if which == 0 { self.control.as_ref() } else { self.data.as_ref() })
                else {
                    continue;
                };
                let mut sts = self.packet(link, op::STS, Vec::new());
                sts.data.extend_from_slice(&link.last_received.to_le_bytes());
                sts.data.extend_from_slice(&15u16.to_le_bytes());
                h.receive(now, &arriving(&sts));
            }
        }
    }

    /// Sends a data packet up the data connection, as a user end writing
    /// a file does.
    fn send_data(&mut self, h: &mut Server, now: u64, opcode: u8, bytes: &[u8]) {
        let link = self.data.as_mut().expect("a data connection");
        let p = Packet {
            opcode,
            forward: 0,
            dest: self.server,
            dest_index: link.theirs,
            source: self.me,
            source_index: link.mine,
            number: link.number,
            ack: link.last_received,
            data: bytes.to_vec(),
        };
        link.number = link.number.wrapping_add(1);
        h.receive(now, &arriving(&p));
        self.drain(h, now);
    }

    /// Sends a command on the control connection and returns the reply
    /// with its transaction id.
    fn command(&mut self, h: &mut Server, now: u64, cmd: &str) -> String {
        let tid = cmd.split(' ').next().unwrap().to_string();
        let link = self.control.as_mut().unwrap();
        let bytes: Vec<u8> = cmd.chars().map(|c| c as u32 as u8).collect();
        let p = Packet {
            opcode: op::DAT,
            forward: 0,
            dest: self.server,
            dest_index: link.theirs,
            source: self.me,
            source_index: link.mine,
            number: link.number,
            ack: link.last_received,
            data: bytes,
        };
        link.number = link.number.wrapping_add(1);
        h.receive(now, &arriving(&p));
        self.drain(h, now);
        self.take_reply(&tid)
            .unwrap_or_else(|| panic!("no reply to {tid}: replies {:?}", self.replies))
    }

    /// [`Client::command`] for a command that need not have been answered
    /// yet: the reply, if one has come, is left to be taken later.
    fn send_command(&mut self, h: &mut Server, now: u64, cmd: &str) {
        let link = self.control.as_mut().unwrap();
        let bytes: Vec<u8> = cmd.chars().map(|c| c as u32 as u8).collect();
        let p = Packet {
            opcode: op::DAT,
            forward: 0,
            dest: self.server,
            dest_index: link.theirs,
            source: self.me,
            source_index: link.mine,
            number: link.number,
            ack: link.last_received,
            data: bytes,
        };
        link.number = link.number.wrapping_add(1);
        h.receive(now, &arriving(&p));
        self.drain(h, now);
    }

    /// The reply to `tid`, taken off the pile if it has come.
    fn take_reply(&mut self, tid: &str) -> Option<String> {
        let i = self.replies.iter().position(|r| r.starts_with(&format!("{tid} ")))?;
        Some(self.replies.remove(i))
    }
}

/// **The FILE protocol serves a file and a directory, read-only**, as the
/// band's client `qfile.lisp` speaks it and the Unix server `FILE.c`
/// answered it: login, a data connection the server calls, OPEN with the
/// properties line and the truename, the file down the data connection
/// in the Lisp Machine character set, EOF, CLOSE answered and then the
/// synchronous mark; DIRECTORY as records; PROBE of a missing file as an
/// FNF error; a compiled file recognised by its magic and sent binary.
#[test]
fn the_file_service_serves_files_and_directories() {
    let root = std::env::temp_dir().join(format!("muir-chaos-file-{}", std::process::id()));
    let sys = root.join("tree").join("sys");
    std::fs::create_dir_all(sys.join("sub")).unwrap();
    std::fs::write(sys.join("hello.lisp"), "hello\nworld\n").unwrap();
    let mut qfasl = file::QFASL_MAGIC.to_vec();
    qfasl.extend_from_slice(&[1, 2, 3, 4, 5]);
    std::fs::write(sys.join("x.qfasl"), &qfasl).unwrap();

    let mut h = Server::new(0o3060);
    h.serve(Box::new(File::new(&root)));
    let mut c = Client::new(0o3050, 0o3060);
    c.connect(&mut h, 0);
    let nl = NEWLINE as char;

    // LOGIN: the user, a home directory, a personal name.
    let r = c.command(&mut h, 10, "T1  LOGIN LISPM LISPM ");
    assert_eq!(r, format!("T1  LOGIN LISPM /lispm/{nl}LISPM{nl}"));

    // DATA-CONNECTION: the server calls our output handle's name.
    c.listening = Some("O0001".into());
    let r = c.command(&mut h, 20, "T2  DATA-CONNECTION I0001 O0001");
    assert_eq!(r, "T2  DATA-CONNECTION");
    assert!(c.data.is_some(), "the data connection is open");

    // OPEN READ CHARACTER: properties, truename, then the file, then EOF.
    let r =
        c.command(&mut h, 30, &format!("T3 I0001 OPEN READ CHARACTER{nl}/tree/sys/hello.lisp{nl}"));
    let head = "T3 I0001 OPEN ";
    assert!(r.starts_with(head), "{r:?}");
    let rest = r.strip_prefix(head).unwrap();
    let (props, tn) = rest.split_once(nl).unwrap();
    let fields: Vec<&str> = props.split(' ').collect();
    assert_eq!(fields.len(), 4, "date, time, length, QFASL: {props:?}");
    assert_eq!(fields[0].len(), 8, "MM/DD/YY");
    assert_eq!(fields[1].len(), 8, "HH:MM:SS");
    assert_eq!(fields[2], "12", "the length in bytes");
    assert_eq!(fields[3], "NIL", "not a compiled file");
    assert_eq!(tn, format!("/tree/sys/hello.lisp{nl}"));
    let ops: Vec<u8> = c.down.iter().map(|(o, _)| *o).collect();
    assert_eq!(ops, [file::CHARACTER_OP, op::EOF], "the data as characters, then EOF");
    assert_eq!(text(&c.down[0].1), format!("hello{nl}world{nl}"), "newlines the Lisp Machine's");
    c.down.clear();

    // CLOSE: answered on the control connection, then the synchronous mark.
    let r = c.command(&mut h, 40, "T4 I0001 CLOSE");
    assert!(r.starts_with("T4 I0001 CLOSE "), "{r:?}");
    assert_eq!(c.down.iter().map(|(o, _)| *o).collect::<Vec<_>>(), [file::SYNC_MARK_OP]);
    c.down.clear();

    // PROBE of a file that is not there.
    let r = c.command(&mut h, 50, &format!("T5  OPEN PROBE CHARACTER{nl}/tree/sys/nope.lisp{nl}"));
    assert_eq!(r, "T5  ERROR FNF C File not found");

    // DIRECTORY: records down the data connection.
    let r = c.command(&mut h, 60, &format!("T6 I0001 DIRECTORY{nl}/tree/sys/*{nl}"));
    assert_eq!(r, "T6 I0001 DIRECTORY");
    let listing: String =
        c.down.iter().filter(|(o, _)| *o == file::CHARACTER_OP).map(|(_, d)| text(d)).collect();
    assert!(c.down.iter().any(|(o, _)| *o == op::EOF), "EOF after the listing");
    let records: Vec<&str> = listing.split(&format!("{nl}{nl}")).collect();
    assert!(records[0].starts_with(nl), "the first record has no pathname: {:?}", records[0]);
    assert!(records[0].contains("BLOCK-SIZE 1024"));
    let hello =
        records.iter().find(|r| r.starts_with("/tree/sys/hello.lisp")).expect("hello.lisp listed");
    assert!(hello.contains(&format!("{nl}LENGTH-IN-BYTES 12{nl}")), "{hello:?}");
    assert!(hello.contains(&format!("{nl}CREATION-DATE ")));
    let sub =
        records.iter().find(|r| r.starts_with("/tree/sys/sub")).expect("the directory listed");
    assert!(sub.ends_with(&format!("{nl}DIRECTORY T")), "{sub:?}");
    c.down.clear();
    let r = c.command(&mut h, 70, "T7 I0001 CLOSE");
    assert_eq!(r, "T7 I0001 CLOSE");
    c.down.clear();

    // A compiled file, opened with DEFAULT: recognised, and sent binary.
    let r = c.command(&mut h, 80, &format!("T8 I0001 OPEN READ DEFAULT{nl}/tree/sys/x.qfasl{nl}"));
    let props = r.strip_prefix("T8 I0001 OPEN ").unwrap().split(nl).next().unwrap();
    assert!(props.ends_with(" 5 T NIL"), "length in words, QFASL, not characters: {props:?}");
    let ops: Vec<u8> = c.down.iter().map(|(o, _)| *o).collect();
    assert_eq!(ops, [file::BINARY_OP, op::EOF]);
    assert_eq!(c.down[0].1, qfasl, "the bytes as they are");

    std::fs::remove_dir_all(&root).ok();
}

/// The character translation is `FILE.c`'s `to_lispm`, both ways round
/// the 0200 boundary.
#[test]
fn unix_text_becomes_lisp_machine_text() {
    assert_eq!(file::to_lispm(b"a\nb\tc"), [b'a', 0o215, b'b', 0o211, b'c']);
    assert_eq!(file::to_lispm(&[0o215, 0o377, 0o15, 0o177]), [0o15, 0o177, 0o212, 0o377]);
    assert_eq!(file::civil(0), (1970, 1, 1, 0, 0, 0));
    assert_eq!(file::civil(1_788_480_000), (2026, 9, 4, 0, 0, 0));
}

/// **The FILE protocol writes a file**, as `qfile.lisp`'s output stream
/// does it and `FILE.c` answered: `OPEN WRITE` on the output handle, the
/// data up the data connection, the user end's own synchronous mark, and
/// `CLOSE`, which renames the temporary into place and answers with the
/// file's date, its length and its truename. Until the close the real
/// file is untouched, which is what makes an abandoned write harmless.
#[test]
fn the_file_service_writes_files() {
    let root = std::env::temp_dir().join(format!("muir-chaos-write-{}", std::process::id()));
    let dir = root.join("tree").join("sys");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("old.lisp"), "was here\n").unwrap();

    let mut h = Server::new(0o3060);
    h.serve(Box::new(File::new(&root)));
    let mut c = Client::new(0o3050, 0o3060);
    c.connect(&mut h, 0);
    let nl = NEWLINE as char;
    c.command(&mut h, 10, "T1  LOGIN LISPM LISPM ");
    c.listening = Some("O0001".into());
    c.command(&mut h, 20, "T2  DATA-CONNECTION I0001 O0001");

    // OPEN WRITE on the output handle, and nothing is there yet.
    let r =
        c.command(&mut h, 30, &format!("T3 O0001 OPEN WRITE CHARACTER{nl}/tree/sys/new.lisp{nl}"));
    assert!(r.starts_with("T3 O0001 OPEN "), "{r:?}");
    assert!(!dir.join("new.lisp").exists(), "the real file waits for the close");
    assert!(
        std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .any(|e| e.file_name().to_string_lossy().starts_with('#')),
        "a temporary is there instead"
    );

    // The data, in the Lisp Machine character set, then the mark.
    c.send_data(&mut h, 40, file::CHARACTER_OP, &[b'h', b'i', NEWLINE, b'!', NEWLINE]);
    c.send_data(&mut h, 45, file::SYNC_MARK_OP, &[]);
    let r = c.command(&mut h, 50, "T4 O0001 CLOSE");
    let head = "T4 O0001 CLOSE ";
    assert!(r.starts_with(head), "{r:?}");
    let rest = r.strip_prefix(head).unwrap();
    let (props, tn) = rest.split_once(nl).unwrap();
    let fields: Vec<&str> = props.split(' ').collect();
    assert_eq!(fields.len(), 3, "date, time, length: {props:?}");
    assert_eq!(fields[2], "5", "the length written");
    assert_eq!(tn, format!("/tree/sys/new.lisp{nl}"));
    assert_eq!(
        std::fs::read(dir.join("new.lisp")).unwrap(),
        b"hi\n!\n",
        "the newlines came back to Unix's"
    );
    assert!(
        !std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .any(|e| e.file_name().to_string_lossy().starts_with('#')),
        "and the temporary is gone"
    );

    // Writing over a file that is there: refused with IF-EXISTS ERROR,
    // taken with SUPERSEDE.
    let r = c.command(
        &mut h,
        60,
        &format!("T5 O0001 OPEN WRITE CHARACTER IF-EXISTS ERROR{nl}/tree/sys/old.lisp{nl}"),
    );
    assert_eq!(r, "T5 O0001 ERROR FAE C File already exists");
    let r = c.command(
        &mut h,
        70,
        &format!("T6 O0001 OPEN WRITE CHARACTER IF-EXISTS SUPERSEDE{nl}/tree/sys/old.lisp{nl}"),
    );
    assert!(r.starts_with("T6 O0001 OPEN "), "{r:?}");
    c.send_data(&mut h, 75, file::CHARACTER_OP, b"now this");
    c.send_data(&mut h, 76, file::SYNC_MARK_OP, &[]);
    assert_eq!(
        std::fs::read_to_string(dir.join("old.lisp")).unwrap(),
        "was here\n",
        "still the old one"
    );
    c.command(&mut h, 80, "T7 O0001 CLOSE");
    assert_eq!(std::fs::read_to_string(dir.join("old.lisp")).unwrap(), "now this");

    // A write abandoned: DELETE on the handle, and the file is as it was.
    let r = c.command(
        &mut h,
        90,
        &format!("T8 O0001 OPEN WRITE CHARACTER IF-EXISTS SUPERSEDE{nl}/tree/sys/old.lisp{nl}"),
    );
    assert!(r.starts_with("T8 O0001 OPEN "));
    c.send_data(&mut h, 95, file::CHARACTER_OP, b"rubbish");
    let r = c.command(&mut h, 100, "T9 O0001 DELETE");
    assert_eq!(r, "T9 O0001 DELETE");
    assert_eq!(std::fs::read_to_string(dir.join("old.lisp")).unwrap(), "now this", "untouched");

    // A binary write goes through as it is.
    let r = c.command(&mut h, 110, &format!("TA O0001 OPEN WRITE BINARY{nl}/tree/sys/b.qfasl{nl}"));
    assert!(r.starts_with("TA O0001 OPEN "));
    c.send_data(&mut h, 115, file::BINARY_OP, &[0o215, 0o12, 0, 0o377]);
    c.send_data(&mut h, 116, file::SYNC_MARK_OP, &[]);
    c.command(&mut h, 120, "TB O0001 CLOSE");
    assert_eq!(
        std::fs::read(dir.join("b.qfasl")).unwrap(),
        [0o215, 0o12, 0, 0o377],
        "bytes as sent"
    );

    // A write where the file must exist and does not.
    let r = c.command(
        &mut h,
        130,
        &format!("TC O0001 OPEN WRITE CHARACTER IF-DOES-NOT-EXIST ERROR{nl}/tree/sys/nope{nl}"),
    );
    assert_eq!(r, "TC O0001 ERROR FNF C File not found");

    std::fs::remove_dir_all(&root).ok();
}

/// **The commands that change a directory**, each answered by name:
/// delete, rename, create a directory, create a link, change properties,
/// expunge, complete, and properties down a data connection.
#[test]
fn the_file_service_manages_a_directory() {
    let root = std::env::temp_dir().join(format!("muir-chaos-dir-{}", std::process::id()));
    let dir = root.join("tree").join("sys");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("one.lisp"), "1").unwrap();
    std::fs::write(dir.join("only.text"), "2").unwrap();

    let mut h = Server::new(0o3060);
    h.serve(Box::new(File::new(&root)));
    let mut c = Client::new(0o3050, 0o3060);
    c.connect(&mut h, 0);
    let nl = NEWLINE as char;
    c.command(&mut h, 10, "T1  LOGIN LISPM LISPM ");
    c.listening = Some("O0001".into());
    c.command(&mut h, 20, "T2  DATA-CONNECTION I0001 O0001");

    // CREATE-DIRECTORY, then again, which is an error.
    assert_eq!(
        c.command(&mut h, 30, &format!("T3  CREATE-DIRECTORY{nl}/tree/sys/made{nl}")),
        "T3  CREATE-DIRECTORY"
    );
    assert!(dir.join("made").is_dir());
    assert_eq!(
        c.command(&mut h, 31, &format!("T4  CREATE-DIRECTORY{nl}/tree/sys/made{nl}")),
        "T4  ERROR DAE C Directory already exists"
    );

    // RENAME, and over something that is there.
    assert_eq!(
        c.command(
            &mut h,
            40,
            &format!("T5  RENAME{nl}/tree/sys/one.lisp{nl}/tree/sys/two.lisp{nl}")
        ),
        "T5  RENAME"
    );
    assert!(dir.join("two.lisp").exists() && !dir.join("one.lisp").exists());
    assert_eq!(
        c.command(
            &mut h,
            41,
            &format!("T6  RENAME{nl}/tree/sys/two.lisp{nl}/tree/sys/only.text{nl}")
        ),
        "T6  ERROR REF C Rename to existing file"
    );
    assert_eq!(
        c.command(&mut h, 42, &format!("T7  RENAME{nl}/tree/sys/gone{nl}/tree/sys/x{nl}")),
        "T7  ERROR FNF C File not found"
    );

    // DELETE with a pathname, and one that is not there.
    assert_eq!(
        c.command(&mut h, 50, &format!("T8  DELETE{nl}/tree/sys/two.lisp{nl}")),
        "T8  DELETE"
    );
    assert!(!dir.join("two.lisp").exists());
    assert_eq!(
        c.command(&mut h, 51, &format!("T9  DELETE{nl}/tree/sys/two.lisp{nl}")),
        "T9  ERROR FNF C File not found"
    );

    // CREATE-LINK.
    assert_eq!(
        c.command(
            &mut h,
            60,
            &format!("TA  CREATE-LINK{nl}/tree/sys/link{nl}/tree/sys/only.text{nl}")
        ),
        "TA  CREATE-LINK"
    );
    assert_eq!(std::fs::read_to_string(dir.join("link")).unwrap(), "2");

    // CHANGE-PROPERTIES: one it has, and one it has not.
    assert_eq!(
        c.command(
            &mut h,
            70,
            &format!("TB  CHANGE-PROPERTIES{nl}/tree/sys/only.text{nl}AUTHOR LISPM{nl}")
        ),
        "TB  CHANGE-PROPERTIES"
    );
    let r = c.command(
        &mut h,
        71,
        &format!("TC  CHANGE-PROPERTIES{nl}/tree/sys/only.text{nl}COLOUR BLUE{nl}"),
    );
    assert_eq!(r, "TC  ERROR UKP C COLOUR cannot be set here");

    // EXPUNGE: a number, and here always none.
    assert_eq!(c.command(&mut h, 80, &format!("TD  EXPUNGE{nl}/tree/sys/{nl}")), "TD  EXPUNGE 0");

    // COMPLETE: one match completes, none is NIL.
    let r = c.command(&mut h, 90, &format!("TE  COMPLETE{nl}/tree/sys/x{nl}/tree/sys/onl{nl}"));
    assert_eq!(r, format!("TE  COMPLETE OLD{nl}/tree/sys/only.text{nl}"));
    let r = c.command(&mut h, 91, &format!("TF  COMPLETE{nl}/tree/sys/x{nl}/tree/sys/zzz{nl}"));
    assert_eq!(r, format!("TF  COMPLETE NIL{nl}/tree/sys/zzz{nl}"));

    // PROPERTIES: one record down the data connection.
    assert_eq!(
        c.command(&mut h, 100, &format!("TG I0001 PROPERTIES{nl}/tree/sys/only.text{nl}")),
        "TG I0001 PROPERTIES"
    );
    let record: String =
        c.down.iter().filter(|(o, _)| *o == file::CHARACTER_OP).map(|(_, d)| text(d)).collect();
    assert!(record.starts_with(&format!("/tree/sys/only.text{nl}")), "{record:?}");
    assert!(record.contains(&format!("{nl}LENGTH-IN-BYTES 1{nl}")), "{record:?}");
    c.down.clear();
    c.command(&mut h, 110, "TH I0001 CLOSE");

    // CONTINUE with nothing to continue, and with a transfer that is not
    // stopped: the server's own `BUG` either way, not an unknown command.
    assert_eq!(c.command(&mut h, 115, "TJ  CONTINUE"), "TJ  ERROR BUG C No transfer to continue");
    c.command(&mut h, 116, &format!("TK I0001 PROPERTIES{nl}/tree/sys/only.text{nl}"));
    assert_eq!(
        c.command(&mut h, 117, "TL I0001 CONTINUE"),
        "TL I0001 ERROR BUG C CONTINUE received when not in error state"
    );
    c.down.clear();
    c.command(&mut h, 118, "TM I0001 CLOSE");
    c.down.clear();

    // A command nobody serves.
    assert_eq!(c.command(&mut h, 120, "TI  SPARKLE"), "TI  ERROR UKC C SPARKLE is not served here");

    std::fs::remove_dir_all(&root).ok();
}

/// The character translation back, `FILE.c`'s `from_lispm`, and that it
/// undoes [`file::to_lispm`] on ordinary text.
#[test]
fn lisp_machine_text_becomes_unix_text() {
    assert_eq!(file::from_lispm(&[b'a', NEWLINE, b'b', 0o211, b'c']), b"a\nb\tc");
    assert_eq!(file::from_lispm(&[0o212]), [0o15]);
    assert_eq!(file::from_lispm(&[0o12]), [0o212]);
    for text in [&b"plain\ntext\n"[..], b"tabs\there\n", b"\x08\x0c\x7f"] {
        assert_eq!(file::from_lispm(&file::to_lispm(text)), text, "{text:?} round trips");
    }
}

/// **The FILE service never writes outside the tree it serves.** The root is
/// the server's `/`, and `--chaos-file-root`'s help promises that everything
/// the service writes, renames and deletes stays under it. The tree is the
/// root and what the root's own links lead to --- each release's fetch
/// script puts its sources there by such a link, under the name that band
/// asks for --- so a pathname is taken
/// component by component under the root with `..` and `.` refused; the root
/// itself is no file to open for writing, delete, rename or create; a link
/// deeper in that points out of the tree leads nowhere, for reading as for
/// writing; and a link in the root itself is served, read and written. Every
/// refusal is `ATD`, and afterwards nothing has appeared beside the root or in
/// the directory the deeper link pointed at.
#[test]
fn the_file_service_never_writes_outside_its_root() {
    let outside = std::env::temp_dir().join(format!("muir-chaos-escape-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&outside);
    let root = outside.join("root");
    let sys = root.join("tree").join("sys");
    let elsewhere = outside.join("elsewhere");
    let release = outside.join("release");
    std::fs::create_dir_all(&sys).unwrap();
    std::fs::create_dir_all(&elsewhere).unwrap();
    std::fs::create_dir_all(&release).unwrap();
    std::fs::write(elsewhere.join("victim.text"), "keep\n").unwrap();
    std::fs::write(sys.join("in.text"), "in\n").unwrap();
    std::fs::write(release.join("hello.text"), "hello\n").unwrap();
    // A link deeper in the tree that leads out of it, and one in the root
    // that is the operator's own mount.
    std::os::unix::fs::symlink(&elsewhere, sys.join("out")).unwrap();
    std::os::unix::fs::symlink(&release, root.join("mount")).unwrap();

    let mut h = Server::new(0o3060);
    h.serve(Box::new(File::new(&root)));
    let mut c = Client::new(0o3050, 0o3060);
    c.connect(&mut h, 0);
    let nl = NEWLINE as char;
    c.command(&mut h, 10, "T1  LOGIN LISPM LISPM ");
    c.listening = Some("O0001".into());
    c.command(&mut h, 20, "T2  DATA-CONNECTION I0001 O0001");
    let mut now = 30;
    let mut refused = |h: &mut Server, cmd: String| {
        now += 10;
        let tid = format!("T{now}");
        let r = c.command(h, now, &format!("{tid} {cmd}"));
        assert!(r.starts_with(&format!("{tid} ")) && r.contains(" ERROR ATD "), "{cmd:?}: {r:?}");
    };

    // The root itself, spelt three ways, and climbing out or staying put.
    for name in [
        "",
        "/",
        "//",
        "/../evil.text",
        "/tree/../../evil.text",
        "/./evil.text",
        "/tree/sys/./x.text",
    ] {
        refused(&mut h, format!("O0001 OPEN WRITE CHARACTER{nl}{name}{nl}"));
    }
    refused(&mut h, format!(" DELETE{nl}{nl}"));
    refused(&mut h, format!(" CREATE-DIRECTORY{nl}{nl}"));
    refused(&mut h, format!(" RENAME{nl}/tree/sys/in.text{nl}/../moved.text{nl}"));

    // Through the deeper link: writing, deleting, renaming into it, making a
    // directory in it, and reading from it.
    refused(&mut h, format!("O0001 OPEN WRITE CHARACTER{nl}/tree/sys/out/evil.text{nl}"));
    refused(&mut h, format!(" DELETE{nl}/tree/sys/out/victim.text{nl}"));
    refused(&mut h, format!(" RENAME{nl}/tree/sys/in.text{nl}/tree/sys/out/moved.text{nl}"));
    refused(&mut h, format!(" CREATE-DIRECTORY{nl}/tree/sys/out/newdir{nl}"));
    refused(&mut h, format!("I0001 OPEN READ CHARACTER{nl}/tree/sys/out/victim.text{nl}"));
    refused(&mut h, format!("I0001 DIRECTORY{nl}/tree/sys/out/*{nl}"));

    // Nothing moved, nothing appeared beside the root or where the deeper
    // link points, and no temporary was left anywhere.
    assert!(root.exists() && sys.join("in.text").exists());
    assert_eq!(std::fs::read_to_string(elsewhere.join("victim.text")).unwrap(), "keep\n");
    let names = |d: &std::path::Path| -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(d)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    };
    assert_eq!(names(&outside), ["elsewhere", "release", "root"]);
    assert_eq!(names(&elsewhere), ["victim.text"]);
    assert_eq!(names(&root), ["mount", "tree"]);
    assert_eq!(names(&sys), ["in.text", "out"]);
    assert_eq!(names(&release), ["hello.text"]);

    // The root's own link is part of the tree served: read and written
    // through, the way the release is under `/tree`.
    now += 10;
    let r = c.command(
        &mut h,
        now,
        &format!("T{now} I0001 OPEN PROBE CHARACTER{nl}/mount/hello.text{nl}"),
    );
    assert!(r.starts_with(&format!("T{now} I0001 OPEN ")), "{r:?}");
    now += 10;
    let r = c.command(
        &mut h,
        now,
        &format!("T{now} O0001 OPEN WRITE CHARACTER{nl}/mount/new.text{nl}"),
    );
    assert!(r.starts_with(&format!("T{now} O0001 OPEN ")), "{r:?}");
    assert!(names(&release).iter().any(|n| n.starts_with('#')), "its temporary is in the mount");

    // And a write under the root is still a write.
    now += 10;
    let r = c.command(
        &mut h,
        now,
        &format!("T{now} O0001 OPEN WRITE CHARACTER{nl}/tree/sys/ok.text{nl}"),
    );
    assert!(r.starts_with(&format!("T{now} O0001 OPEN ")), "{r:?}");
    std::fs::remove_dir_all(&outside).unwrap();
}

/// **A write that fails is stopped, marked and resumed.** A recoverable
/// error during a transfer sends an *asynchronous mark* down the data
/// connection --- `FILE.c`'s `fherror`, `TIDNO <handle> ERROR <code> R
/// <message>` --- which is the only thing that puts the client's stream
/// into the state where it will send `CONTINUE`. The bytes that could
/// not be written wait, and the `CONTINUE` writes them.
///
/// The failure here is a real one: the temporary file is taken away
/// under the transfer, so the append has nothing to append to.
#[test]
fn a_stalled_write_is_marked_and_continued() {
    let root = std::env::temp_dir().join(format!("muir-chaos-cont-{}", std::process::id()));
    let dir = root.join("tree").join("sys");
    std::fs::create_dir_all(&dir).unwrap();

    let mut h = Server::new(0o3060);
    h.serve(Box::new(File::new(&root)));
    let mut c = Client::new(0o3050, 0o3060);
    c.connect(&mut h, 0);
    let nl = NEWLINE as char;
    c.command(&mut h, 10, "T1  LOGIN LISPM LISPM ");
    c.listening = Some("O0001".into());
    c.command(&mut h, 20, "T2  DATA-CONNECTION I0001 O0001");
    c.command(&mut h, 30, &format!("T3 O0001 OPEN WRITE CHARACTER{nl}/tree/sys/w.text{nl}"));

    // The first chunk lands in the temporary.
    c.send_data(&mut h, 40, file::CHARACTER_OP, b"first ");
    let temp = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| p.file_name().unwrap().to_string_lossy().starts_with('#'))
        .expect("a temporary");
    assert_eq!(std::fs::read(&temp).unwrap(), b"first ");
    assert!(c.down.is_empty(), "nothing has gone wrong yet");

    // Take it away, and the next chunk cannot be written.
    std::fs::remove_file(&temp).unwrap();
    c.send_data(&mut h, 50, file::CHARACTER_OP, b"second");
    let marks: Vec<&(u8, Vec<u8>)> =
        c.down.iter().filter(|(o, _)| *o == file::ASYNC_MARK_OP).collect();
    assert_eq!(marks.len(), 1, "one asynchronous mark: {:?}", c.down);
    let mark = text(&marks[0].1);
    assert!(mark.starts_with("TIDNO O0001 ERROR NMR R "), "{mark:?}");
    c.down.clear();

    // Closing now would give a short file, so it does not: the error
    // comes back fatal and the temporary goes.
    let r = c.command(&mut h, 55, "T4 O0001 CLOSE");
    assert!(r.starts_with("T4 O0001 ERROR NMR F "), "{r:?}");
    assert!(!dir.join("w.text").exists(), "and nothing was put in place");

    // Again, and this time the trouble is cleared before continuing.
    c.command(&mut h, 60, &format!("T5 O0001 OPEN WRITE CHARACTER{nl}/tree/sys/w.text{nl}"));
    c.send_data(&mut h, 65, file::CHARACTER_OP, b"first ");
    let temp = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| p.file_name().unwrap().to_string_lossy().starts_with('#'))
        .expect("a temporary");
    std::fs::remove_file(&temp).unwrap();
    c.send_data(&mut h, 70, file::CHARACTER_OP, b"second");
    assert!(c.down.iter().any(|(o, _)| *o == file::ASYNC_MARK_OP), "stopped again");
    c.down.clear();

    // A CONTINUE while it is still broken is answered, and marks again.
    std::fs::write(&temp, b"first ").unwrap();
    std::fs::remove_file(&temp).unwrap();
    assert_eq!(c.command(&mut h, 75, "T6 O0001 CONTINUE"), "T6 O0001 CONTINUE");
    assert!(c.down.iter().any(|(o, _)| *o == file::ASYNC_MARK_OP), "and it failed again");
    c.down.clear();

    // Put the temporary back with what it had, and continue for real.
    std::fs::write(&temp, b"first ").unwrap();
    assert_eq!(c.command(&mut h, 80, "T7 O0001 CONTINUE"), "T7 O0001 CONTINUE");
    assert!(!c.down.iter().any(|(o, _)| *o == file::ASYNC_MARK_OP), "no mark this time");
    assert_eq!(std::fs::read(&temp).unwrap(), b"first second", "the held bytes went in");

    // And the write finishes normally.
    c.send_data(&mut h, 85, file::CHARACTER_OP, b" third");
    c.send_data(&mut h, 86, file::SYNC_MARK_OP, &[]);
    let r = c.command(&mut h, 90, "T8 O0001 CLOSE");
    assert!(r.starts_with("T8 O0001 CLOSE "), "{r:?}");
    assert_eq!(std::fs::read_to_string(dir.join("w.text")).unwrap(), "first second third");

    std::fs::remove_dir_all(&root).ok();
}

/// **`FILEPOS` moves a read**, which is what the client's
/// `:SET-BUFFER-POINTER` sends. It goes with the mark flag set, so the
/// reply is followed by a synchronous mark --- the client reads until
/// that to throw away what was in flight --- and then the file again
/// from the byte asked for.
#[test]
fn a_read_can_be_repositioned() {
    let root = std::env::temp_dir().join(format!("muir-chaos-pos-{}", std::process::id()));
    let dir = root.join("tree").join("sys");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("p.text"), "0123456789").unwrap();

    let mut h = Server::new(0o3060);
    h.serve(Box::new(File::new(&root)));
    let mut c = Client::new(0o3050, 0o3060);
    c.connect(&mut h, 0);
    let nl = NEWLINE as char;
    c.command(&mut h, 10, "T1  LOGIN LISPM LISPM ");
    c.listening = Some("O0001".into());
    c.command(&mut h, 20, "T2  DATA-CONNECTION I0001 O0001");
    c.command(&mut h, 30, &format!("T3 I0001 OPEN READ CHARACTER{nl}/tree/sys/p.text{nl}"));
    let whole: String =
        c.down.iter().filter(|(o, _)| *o == file::CHARACTER_OP).map(|(_, d)| text(d)).collect();
    assert_eq!(whole, "0123456789");
    c.down.clear();

    // Back to byte four: the mark first, then the rest.
    assert_eq!(c.command(&mut h, 40, "T4 I0001 FILEPOS 4"), "T4 I0001 FILEPOS");
    assert_eq!(c.down[0].0, file::SYNC_MARK_OP, "the mark comes first: {:?}", c.down);
    let rest: String =
        c.down.iter().filter(|(o, _)| *o == file::CHARACTER_OP).map(|(_, d)| text(d)).collect();
    assert_eq!(rest, "456789");
    assert!(c.down.iter().any(|(o, _)| *o == op::EOF), "and it ends");
    c.down.clear();

    // The end of the file is a position; past it is not.
    assert_eq!(c.command(&mut h, 50, "T5 I0001 FILEPOS 10"), "T5 I0001 FILEPOS");
    assert!(
        !c.down.iter().any(|(o, _)| *o == file::CHARACTER_OP),
        "nothing left to send: {:?}",
        c.down
    );
    c.down.clear();
    assert_eq!(
        c.command(&mut h, 60, "T6 I0001 FILEPOS 11"),
        "T6 I0001 ERROR FOR C Filepos out of range"
    );
    assert_eq!(c.command(&mut h, 70, "T7  FILEPOS 0"), "T7  ERROR BUG C No transfer to position");

    c.command(&mut h, 80, "T8 I0001 CLOSE");
    std::fs::remove_dir_all(&root).ok();
}

/// **Two files written one after the other on the same data connection,
/// the second long.**  What the band does when asked twice: `OPEN WRITE`
/// on the same output handle again, the data in as many packets as it
/// takes, the mark, `CLOSE`.  Both files go into place.
#[test]
fn the_file_service_writes_a_second_long_file_on_the_same_handle() {
    let root = std::env::temp_dir().join(format!("muir-chaos-write2-{}", std::process::id()));
    let dir = root.join("tmp");
    std::fs::create_dir_all(&dir).unwrap();
    let mut h = Server::new(0o3060);
    h.serve(Box::new(File::new(&root)));
    let mut c = Client::new(0o3050, 0o3060);
    c.connect(&mut h, 0);
    let nl = NEWLINE as char;
    c.command(&mut h, 10, "T1  LOGIN LISPM LISPM ");
    c.listening = Some("O0001".into());
    c.command(&mut h, 20, "T2  DATA-CONNECTION I0001 O0001");
    let r = c.command(&mut h, 30, &format!("T3 O0001 OPEN WRITE CHARACTER{nl}/tmp/first.text{nl}"));
    assert!(r.starts_with("T3 O0001 OPEN "), "{r:?}");
    c.send_data(&mut h, 40, file::CHARACTER_OP, &[b'o', b'n', b'e', NEWLINE]);
    c.send_data(&mut h, 45, file::SYNC_MARK_OP, &[]);
    let r = c.command(&mut h, 50, "T4 O0001 CLOSE");
    assert!(r.starts_with("T4 O0001 CLOSE "), "{r:?}");
    assert_eq!(std::fs::read(dir.join("first.text")).unwrap(), b"one\n");

    let r =
        c.command(&mut h, 60, &format!("T5 O0001 OPEN WRITE CHARACTER{nl}/tmp/second.text{nl}"));
    assert!(r.starts_with("T5 O0001 OPEN "), "{r:?}");
    let line: Vec<u8> =
        b"Data is address shifted 13 places".iter().copied().chain([NEWLINE]).collect();
    let mut all = Vec::new();
    for _ in 0..200 {
        all.extend_from_slice(&line);
    }
    let mut t = 70;
    for chunk in all.chunks(488) {
        c.send_data(&mut h, t, file::CHARACTER_OP, chunk);
        t += 1;
    }
    c.send_data(&mut h, t, file::SYNC_MARK_OP, &[]);
    let r = c.command(&mut h, t + 10, "T6 O0001 CLOSE");
    assert!(r.starts_with("T6 O0001 CLOSE "), "{r:?}");
    let got = std::fs::read(dir.join("second.text")).expect("the second file in place");
    assert_eq!(got.len(), all.len());
    assert!(
        !std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .any(|e| e.file_name().to_string_lossy().starts_with('#')),
        "no temporary left"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// **A reply longer than a packet crosses in as many as it takes.** The
/// control connection is a character stream and a packet carries 488
/// bytes of it. A command the user end sends fits one; the reply quoting
/// it back --- an unknown command's name in its `ERROR`, a `LOGIN`'s user
/// in its home directory --- need not, and goes as a stream does.
#[test]
fn a_reply_longer_than_a_packet_is_sent_in_pieces() {
    let root = std::env::temp_dir().join(format!("muir-chaos-long-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let mut h = Server::new(0o3060);
    h.serve(Box::new(File::new(&root)));
    let mut c = Client::new(0o3050, 0o3060);
    c.connect(&mut h, 0);
    let name = "X".repeat(470);
    let cmd = format!("T9  {name}");
    let (me, server) = (c.me, c.server);
    let link = c.control.as_mut().unwrap();
    let p = Packet {
        opcode: op::DAT,
        forward: 0,
        dest: server,
        dest_index: link.theirs,
        source: me,
        source_index: link.mine,
        number: link.number,
        ack: link.last_received,
        data: cmd.bytes().collect(),
    };
    link.number = link.number.wrapping_add(1);
    h.receive(10, &arriving(&p));
    c.drain(&mut h, 10);
    let all = c.replies.concat();
    assert!(all.starts_with("T9  ERROR UKC C "), "{all:?}");
    assert!(all.contains(&name), "the name is quoted back whole");
    let sizes: Vec<usize> = c.replies.iter().map(String::len).collect();
    assert!(sizes.len() > 1, "more than one packet: {sizes:?}");
    assert!(sizes.iter().all(|&n| n <= packet::MAX_DATA), "each within a packet: {sizes:?}");
    std::fs::remove_dir_all(&root).unwrap();
}

/// **A completion is cut between characters.** Names on the host are
/// UTF-8; two that share a lead byte and differ in the next used to be cut
/// inside the character, which is not a string and stopped the server.
#[test]
fn a_completion_is_cut_between_characters() {
    let root = std::env::temp_dir().join(format!("muir-chaos-utf8-{}", std::process::id()));
    let sys = root.join("tree").join("sys");
    std::fs::create_dir_all(&sys).unwrap();
    std::fs::write(sys.join("α1.lisp"), "").unwrap();
    std::fs::write(sys.join("β2.lisp"), "").unwrap();
    let mut h = Server::new(0o3060);
    h.serve(Box::new(File::new(&root)));
    let mut c = Client::new(0o3050, 0o3060);
    c.connect(&mut h, 0);
    let nl = NEWLINE as char;
    let r = c.command(&mut h, 10, &format!("T1  COMPLETE{nl}/tree/sys/x{nl}/tree/sys/{nl}"));
    assert_eq!(
        r,
        format!("T1  COMPLETE NIL{nl}/tree/sys/{nl}"),
        "nothing in common past the slash"
    );
    std::fs::remove_dir_all(&root).unwrap();
}

/// The names in `dir` that begin with `#`: the FILE service's temporaries.
fn temporaries(dir: &std::path::Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with('#'))
        .collect();
    v.sort();
    v
}

/// **A wildcard is matched in linear time.** `DIRECTORY` matches names
/// against the glob the user end sends, `*` any run and `?` any one
/// character. A matcher that tries every way of dividing the name among
/// the stars takes time exponential in their number on a name that almost
/// matches: ten stars against sixty `a`s is billions of tries, and the
/// engine's thread stops answering while it makes them. The answers are
/// the same; they come at once.
#[test]
fn a_wildcard_is_matched_in_linear_time() {
    use muir::chaos::file::matches;
    for (pattern, name, want) in [
        ("*", "anything", true),
        ("*", "", true),
        ("", "anything", true),
        ("*.lisp", "hello.lisp", true),
        ("*.lisp", "hello.qfasl", false),
        ("h?llo.*", "hello.lisp", true),
        ("h?llo.*", "hllo.lisp", false),
        ("hello.lisp", "hello.lisp", true),
        ("hello.lisp", "hello.lis", false),
        ("hello.lis", "hello.lisp", false),
        ("*o*", "hello", true),
        ("*x*", "hello", false),
        ("a*b*c", "abc", true),
        ("a*b*c", "aXbYc", true),
        ("a*b*c", "aXbY", false),
        ("*a", "aaa", true),
        ("a**", "a", true),
        ("?", "", false),
        ("*βγ", "αβγ", true),
    ] {
        assert_eq!(matches(pattern, name), want, "{pattern:?} against {name:?}");
    }
    let name = "a".repeat(60);
    let started = std::time::Instant::now();
    assert!(!matches("*a*a*a*a*a*a*a*a*a*a*b", &name), "there is no b in it");
    assert!(matches("*a*a*a*a*a*a*a*a*a*a*", &name));
    assert!(matches("*a*a*a*a*a*a*a*a*a*a*a", &name));
    assert!(started.elapsed() < std::time::Duration::from_secs(1), "{:?}", started.elapsed());

    // And through the service, on a directory holding such a name.
    let root = std::env::temp_dir().join(format!("muir-chaos-glob-{}", std::process::id()));
    let dir = root.join("tree");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(&name), "").unwrap();
    // Ten a's and a b: the one name the pattern matches.
    let hit = format!("{}b", "a".repeat(10));
    std::fs::write(dir.join(&hit), "").unwrap();
    let mut h = Server::new(0o3060);
    h.serve(Box::new(File::new(&root)));
    let mut c = Client::new(0o3050, 0o3060);
    c.connect(&mut h, 0);
    let nl = NEWLINE as char;
    c.listening = Some("O0001".into());
    c.command(&mut h, 10, "T1  DATA-CONNECTION I0001 O0001");
    let started = std::time::Instant::now();
    let r =
        c.command(&mut h, 20, &format!("T2 I0001 DIRECTORY{nl}/tree/*a*a*a*a*a*a*a*a*a*a*b{nl}"));
    assert_eq!(r, "T2 I0001 DIRECTORY");
    assert!(started.elapsed() < std::time::Duration::from_secs(1), "{:?}", started.elapsed());
    let listing: String =
        c.down.iter().filter(|(o, _)| *o == file::CHARACTER_OP).map(|(_, d)| text(d)).collect();
    assert!(listing.contains(&format!("/tree/{hit}{nl}")), "{listing:?}");
    assert!(!listing.contains(&name), "sixty a's do not end in b");
    std::fs::remove_dir_all(&root).ok();
}

/// **A FIFO in the root is not read.** Everything under the root that was
/// not a directory used to be read whole before it was answered, and a
/// named pipe with no writer blocks its reader --- the engine's thread ---
/// for ever, as a device would exhaust its memory. What is not a regular
/// file is refused with `WKF`, the band's `WRONG-KIND-OF-FILE` in
/// `sys/io/file/open.lisp`: on `OPEN READ`, on `PROBE`, and on an `OPEN
/// WRITE` over it.
#[test]
fn a_fifo_in_the_root_is_not_read() {
    let root = std::env::temp_dir().join(format!("muir-chaos-fifo-{}", std::process::id()));
    let dir = root.join("tree").join("sys");
    std::fs::create_dir_all(&dir).unwrap();
    let made = std::process::Command::new("mkfifo").arg(dir.join("pipe")).status();
    if !made.is_ok_and(|s| s.success()) {
        eprintln!("skipped: mkfifo is not available");
        std::fs::remove_dir_all(&root).ok();
        return;
    }
    let mut h = Server::new(0o3060);
    h.serve(Box::new(File::new(&root)));
    let mut c = Client::new(0o3050, 0o3060);
    c.connect(&mut h, 0);
    let nl = NEWLINE as char;
    c.command(&mut h, 10, "T1  LOGIN LISPM LISPM ");
    c.listening = Some("O0001".into());
    c.command(&mut h, 20, "T2  DATA-CONNECTION I0001 O0001");
    let r = c.command(&mut h, 30, &format!("T3 I0001 OPEN READ CHARACTER{nl}/tree/sys/pipe{nl}"));
    assert_eq!(r, "T3 I0001 ERROR WKF C Not a regular file");
    let r = c.command(&mut h, 40, &format!("T4  OPEN PROBE CHARACTER{nl}/tree/sys/pipe{nl}"));
    assert_eq!(r, "T4  ERROR WKF C Not a regular file");
    for (tid, how) in [("T5", "APPEND"), ("T6", "OVERWRITE"), ("T7", "SUPERSEDE")] {
        let r = c.command(
            &mut h,
            50,
            &format!("{tid} O0001 OPEN WRITE CHARACTER IF-EXISTS {how}{nl}/tree/sys/pipe{nl}"),
        );
        assert_eq!(r, format!("{tid} O0001 ERROR WKF C Not a regular file"), "{how}");
    }
    assert!(temporaries(&dir).is_empty(), "nothing was started");
    // A listing looks at it without reading it.
    let r = c.command(&mut h, 60, &format!("T8 I0001 DIRECTORY{nl}/tree/sys/*{nl}"));
    assert_eq!(r, "T8 I0001 DIRECTORY");
    let listing: String =
        c.down.iter().filter(|(o, _)| *o == file::CHARACTER_OP).map(|(_, d)| text(d)).collect();
    assert!(listing.contains(&format!("/tree/sys/pipe{nl}")), "{listing:?}");
    std::fs::remove_dir_all(&root).ok();
}

/// **Two control connections from one host write in one directory without
/// touching each other's temporary.** The band opens a second control
/// connection to a host when the first is busy --- a second host unit in
/// `qfile.lisp` --- and a temporary used to be named by the client's
/// address and a count kept per control connection, so the first write of
/// each connection in one directory took the same name, and making the
/// second emptied the first. A temporary is now made new under a name no
/// other write can take.
#[test]
fn two_control_connections_write_in_one_directory() {
    let root = std::env::temp_dir().join(format!("muir-chaos-two-{}", std::process::id()));
    let dir = root.join("tmp");
    std::fs::create_dir_all(&dir).unwrap();
    let mut h = Server::new(0o3060);
    h.serve(Box::new(File::new(&root)));
    let nl = NEWLINE as char;
    let mut a = Client::new(0o3050, 0o3060);
    a.connect(&mut h, 0);
    a.command(&mut h, 10, "T1  LOGIN LISPM LISPM ");
    a.listening = Some("O0001".into());
    a.command(&mut h, 20, "T2  DATA-CONNECTION I0001 O0001");
    // The same host on other indices.
    let mut b = Client::new(0o3050, 0o3060);
    b.next_index = 0o31;
    b.connect(&mut h, 1);
    b.command(&mut h, 11, "T1  LOGIN LISPM LISPM ");
    b.listening = Some("O0002".into());
    b.command(&mut h, 21, "T2  DATA-CONNECTION I0002 O0002");

    let r = a.command(&mut h, 30, &format!("T3 O0001 OPEN WRITE CHARACTER{nl}/tmp/a.text{nl}"));
    assert!(r.starts_with("T3 O0001 OPEN "), "{r:?}");
    a.send_data(&mut h, 35, file::CHARACTER_OP, b"from a");
    let r = b.command(&mut h, 31, &format!("T3 O0002 OPEN WRITE CHARACTER{nl}/tmp/b.text{nl}"));
    assert!(r.starts_with("T3 O0002 OPEN "), "{r:?}");
    assert_eq!(temporaries(&dir).len(), 2, "a temporary each: {:?}", temporaries(&dir));
    b.send_data(&mut h, 36, file::CHARACTER_OP, b"from b");
    a.send_data(&mut h, 40, file::SYNC_MARK_OP, &[]);
    b.send_data(&mut h, 41, file::SYNC_MARK_OP, &[]);
    let r = a.command(&mut h, 50, "T4 O0001 CLOSE");
    assert!(r.starts_with("T4 O0001 CLOSE "), "{r:?}");
    let r = b.command(&mut h, 51, "T4 O0002 CLOSE");
    assert!(r.starts_with("T4 O0002 CLOSE "), "{r:?}");
    assert_eq!(std::fs::read_to_string(dir.join("a.text")).unwrap(), "from a");
    assert_eq!(std::fs::read_to_string(dir.join("b.text")).unwrap(), "from b");
    assert!(temporaries(&dir).is_empty(), "{:?}", temporaries(&dir));
    std::fs::remove_dir_all(&root).ok();
}

/// **A control connection that goes away takes its temporaries with it.**
/// A write in progress is a temporary beside the real file. When the user
/// end's connection closes under it --- a CLS, or the machine rebooted ---
/// the write will never finish, and the temporary is removed instead of
/// staying in the directory as a `#muir-…#`. An `UNDATA-CONNECTION` on a
/// handle in the middle of a write ends that write the same way.
#[test]
fn a_lost_control_connection_leaves_no_temporary() {
    let root = std::env::temp_dir().join(format!("muir-chaos-lost-{}", std::process::id()));
    let dir = root.join("tmp");
    std::fs::create_dir_all(&dir).unwrap();
    let mut h = Server::new(0o3060);
    h.serve(Box::new(File::new(&root)));
    let mut c = Client::new(0o3050, 0o3060);
    c.connect(&mut h, 0);
    let nl = NEWLINE as char;
    c.command(&mut h, 10, "T1  LOGIN LISPM LISPM ");
    c.listening = Some("O0001".into());
    c.command(&mut h, 20, "T2  DATA-CONNECTION I0001 O0001");

    // UNDATA-CONNECTION in the middle of a write.
    let r = c.command(&mut h, 30, &format!("T3 O0001 OPEN WRITE CHARACTER{nl}/tmp/w.text{nl}"));
    assert!(r.starts_with("T3 O0001 OPEN "), "{r:?}");
    c.send_data(&mut h, 40, file::CHARACTER_OP, b"half");
    assert_eq!(temporaries(&dir).len(), 1);
    assert_eq!(c.command(&mut h, 50, "T4 O0001 UNDATA-CONNECTION"), "T4 O0001 UNDATA-CONNECTION");
    assert!(temporaries(&dir).is_empty(), "{:?}", temporaries(&dir));
    assert!(!dir.join("w.text").exists(), "and nothing went into place");

    // A new data connection, a write, and the control connection closes.
    c.listening = Some("O0002".into());
    assert_eq!(c.command(&mut h, 60, "T5  DATA-CONNECTION I0002 O0002"), "T5  DATA-CONNECTION");
    let r = c.command(&mut h, 70, &format!("T6 O0002 OPEN WRITE CHARACTER{nl}/tmp/w.text{nl}"));
    assert!(r.starts_with("T6 O0002 OPEN "), "{r:?}");
    c.send_data(&mut h, 80, file::CHARACTER_OP, b"half");
    assert_eq!(temporaries(&dir).len(), 1);
    let cls = c.packet(c.control.as_ref().unwrap(), op::CLS, b"Rebooted".to_vec());
    h.receive(90, &arriving(&cls));
    c.drain(&mut h, 90);
    assert_eq!(h.connections(), 0, "the data connection went with it");
    assert!(temporaries(&dir).is_empty(), "{:?}", temporaries(&dir));
    assert!(!dir.join("w.text").exists());
    std::fs::remove_dir_all(&root).ok();
}

/// **Data goes only into the write in progress.** What comes up a data
/// connection is the file being written on its output handle. Before an
/// `OPEN WRITE`, or after a `CLOSE` --- a mark the user end sent that
/// crossed the reply --- there is no file for it; it used to wait in the
/// channel, unbounded, and join the next write on that handle. It is
/// dropped. And a data packet with the `CLOSE` on its heels, no turn of
/// the server between them, used to miss the file: the close renamed the
/// temporary before the data had been looked at. It goes in first.
#[test]
fn data_goes_only_into_the_write_in_progress() {
    let root = std::env::temp_dir().join(format!("muir-chaos-stray-{}", std::process::id()));
    let dir = root.join("tmp");
    std::fs::create_dir_all(&dir).unwrap();
    let mut h = Server::new(0o3060);
    h.serve(Box::new(File::new(&root)));
    let mut c = Client::new(0o3050, 0o3060);
    c.connect(&mut h, 0);
    let nl = NEWLINE as char;
    c.command(&mut h, 10, "T1  LOGIN LISPM LISPM ");
    c.listening = Some("O0001".into());
    c.command(&mut h, 20, "T2  DATA-CONNECTION I0001 O0001");

    // Before any write: nowhere to go.
    c.send_data(&mut h, 30, file::CHARACTER_OP, b"junk");
    c.send_data(&mut h, 31, file::SYNC_MARK_OP, &[]);
    let r = c.command(&mut h, 40, &format!("T3 O0001 OPEN WRITE CHARACTER{nl}/tmp/w.text{nl}"));
    assert!(r.starts_with("T3 O0001 OPEN "), "{r:?}");
    c.send_data(&mut h, 50, file::CHARACTER_OP, b"real");

    // The last of the data, the mark and the CLOSE, back to back.
    for (t, opcode, bytes) in
        [(60, file::CHARACTER_OP, &b" tail"[..]), (61, file::SYNC_MARK_OP, &[])]
    {
        let link = c.data.as_mut().unwrap();
        let p = Packet {
            opcode,
            forward: 0,
            dest: c.server,
            dest_index: link.theirs,
            source: c.me,
            source_index: link.mine,
            number: link.number,
            ack: link.last_received,
            data: bytes.to_vec(),
        };
        link.number = link.number.wrapping_add(1);
        h.receive(t, &arriving(&p));
    }
    let r = c.command(&mut h, 62, "T4 O0001 CLOSE");
    assert!(r.starts_with("T4 O0001 CLOSE "), "{r:?}");
    assert_eq!(std::fs::read_to_string(dir.join("w.text")).unwrap(), "real tail");
    std::fs::remove_dir_all(&root).ok();
}

/// **A peer that falls silent is given up after the band's own interval.**
/// Nothing in the transport freed a connection whose other end had gone:
/// its unreceipted packets went again every half second for ever, and an
/// RFC from the same host and index was a duplicate for ever --- which is
/// what a reboot of the same band sends, its indices seeded from a clock
/// the simulator repeats. `chsncp.lisp`'s `PROBE-CONN` puts a connection
/// in `HOST-DOWN-STATE` once nothing has been received on it for
/// `HOST-DOWN-INTERVAL`, three minutes; the Chaosnet server does the same,
/// and a fresh RFC after that is a fresh connection.
#[test]
fn a_silent_peer_is_freed_after_the_host_down_interval() {
    let mut h = Server::new(0o3060);
    h.serve(Box::new(Echo));
    let me = (0o3050, 0o21);
    h.receive(0, &arriving(&rfc(me, 0o3060, 100, "ECHO")));
    let opn = next_from(&mut h, 0).expect("an OPN");
    assert_eq!((opn.opcode, opn.ack), (op::OPN, 100));
    assert_eq!(h.connections(), 1);
    // Nothing comes back. The OPN goes again to the end of the interval.
    let t = server::HOST_DOWN_NS;
    assert_eq!(next_from(&mut h, t).map(|p| p.opcode), Some(op::OPN), "still trying");
    assert_eq!(h.connections(), 1, "and still there");
    // Past it: freed, and quiet.
    assert_eq!(next_from(&mut h, t + 1), None);
    assert_eq!(h.connections(), 0, "given up");
    // The same host and index again is a new connection, not a duplicate.
    h.receive(t + 10, &arriving(&rfc(me, 0o3060, 700, "ECHO")));
    let opn = next_from(&mut h, t + 10).expect("an OPN for the new connection");
    assert_eq!((opn.opcode, opn.ack), (op::OPN, 700));
    assert_eq!(h.connections(), 1);
    // A packet from the peer starts the interval again: an STS two minutes
    // in keeps the connection past the three.
    let heard = t + 10 + 120_000_000_000;
    let mut sts = Packet {
        opcode: op::STS,
        forward: 0,
        dest: 0o3060,
        dest_index: opn.source_index,
        source: me.0,
        source_index: me.1,
        number: 701,
        ack: opn.number,
        data: Vec::new(),
    };
    sts.data.extend_from_slice(&opn.number.to_le_bytes());
    sts.data.extend_from_slice(&5u16.to_le_bytes());
    h.receive(heard, &arriving(&sts));
    assert_eq!(next_from(&mut h, t + 10 + server::HOST_DOWN_NS + 1), None);
    assert_eq!(h.connections(), 1, "heard from within the interval");
    assert_eq!(next_from(&mut h, heard + server::HOST_DOWN_NS + 1), None);
    assert_eq!(h.connections(), 0, "and not since");
}

/// **A refusal fits in a packet.** A CLS quotes its reason, and one that
/// quotes the RFC's contact name back can run past the 488 bytes a packet
/// carries (AIM-628 §3.5); the count word is twelve bits, so the frame went
/// out longer than the interface's buffer. The reason is cut to fit, from
/// the transport and from a service alike.
#[test]
fn a_refusal_fits_in_a_packet() {
    let mut h = Server::new(0o3060);
    h.serve(Box::new(Echo));
    let name = "Z".repeat(packet::MAX_DATA);
    h.receive(0, &arriving(&rfc((0o3050, 0o21), 0o3060, 1, &name)));
    let cls = next_from(&mut h, 0).expect("a refusal");
    assert_eq!((cls.opcode, cls.dest_index), (op::CLS, 0o21));
    assert!(cls.data.len() <= packet::MAX_DATA, "{} bytes of reason", cls.data.len());
    assert!(cls.to_buffer(0o3050).len() <= 8 + packet::MAX_DATA / 2 + 1, "within the buffer");
    assert!(text(&cls.data).starts_with("No server for contact name Z"), "and still the reason");
    h.receive(10, &arriving(&rfc((0o3050, 0o22), 0o3060, 2, "ECHO LONG")));
    let cls = next_from(&mut h, 10).expect("the service's refusal");
    assert_eq!((cls.opcode, cls.dest_index), (op::CLS, 0o22));
    assert!(cls.data.len() <= packet::MAX_DATA, "{} bytes of reason", cls.data.len());
    assert!(text(&cls.data).starts_with("not today, and at length "));
}

/// **The ether keeps its log only when asked.** [`muir::chaos::ether::Ether::log`]
/// holds the last [`muir::chaos::ether::LOG_CAP`] events with their packets,
/// tens of megabytes over a day's traffic, and only a test ever reads it.
#[test]
fn the_ether_keeps_its_log_only_when_asked() {
    use muir::chaos::ether::{Capture, Ether};
    let events = |keep: bool| {
        let mut e = Ether::new();
        e.keep_log(keep);
        let mut node = Capture::new(0o3060);
        node.to_send.push_back(rfc((0o3060, 1), 0o3050, 1, "TIME").to_buffer(0o3050));
        e.attach(Box::new(node));
        // Advance at the ether's own due times, as the machine does: a
        // fixed grid skips the decoder's mid-cell samples. Once the node's
        // one packet has gone and been heard, nothing more is due.
        let mut t = 0;
        for _ in 0..1_000_000 {
            e.at(t);
            match e.next_due() {
                Some(d) => t = d.max(t + 1),
                None => break,
            }
            if t >= 3_000_000 {
                break;
            }
        }
        e.log.len()
    };
    assert!(events(true) >= 2, "sent, and heard going by");
    assert_eq!(events(false), 0);
}

/// The detector's lockout is the board's: `-EDGE` presets `LOCKOUT` at
/// LMDETC 0E08, and `LOCKOUT END` clocks it clear at the end of the delay
/// chain from `GENCLK` --- the TD100NC at 0B04, the TD100NC at 0B11, the
/// TD25NC at 0A11, tapped at 100, 60 and 10 ns by the straps
/// `cadrio/iob.eco` adds. That is past the mid-cell transition and short of
/// the next cell, which is what makes it a lockout, and a decoder given a
/// cell's two edges takes one bit from them.
#[test]
fn the_lockout_ends_between_the_mid_cell_transition_and_the_next_cell() {
    assert_eq!(wire::LOCKOUT_NS, 100 + 60 + 10);
    const { assert!(wire::CELL_NS / 2 < wire::LOCKOUT_NS && wire::LOCKOUT_NS < wire::CELL_NS) };
    // Two equal bits, so the second cell has its mid-cell transition; then
    // the line idle. An edge at 125 is inside the lockout; one at 250 is
    // the next cell.
    let bits = [true, true, false];
    let changes = wire::encode(&bits);
    let mid = wire::CELL_NS + wire::CELL_NS / 2;
    assert!(changes.iter().any(|&(t, _)| t == mid), "a mid-cell edge in the second cell");
    assert_eq!(wire::decode(&changes), vec![bits.to_vec()]);
}

/// **The System 304 band calls its file and time host at 4403, and this
/// machine answers there as 4401.** [`support::CHAOS_304`] is the pair
/// every machine test runs with, and it is the band's own: asked at its
/// listener, `(send (si:parse-host "OZ") :chaos-address)` answers 2307
/// decimal, which is 4403, and `si:local-host` is `AMS-LISPM-1`, which its
/// host table puts at 4401.
///
/// What that pair buys is on the cable here: the boot puts an `RFC "TIME"`
/// on it, addressed from 4401 to 4403, and the server answers. At any
/// other pair nothing is sent at all --- 4403 is on another subnet from
/// 3050, and the band, hearing no route to it, never transmits --- and the
/// machine comes up asking for the date instead. Neither release's pair is
/// what `chaos::Config` defaults to --- that is subnet 376's and no
/// band's --- so each release's tests hand their band the pair it holds.
#[test]
fn the_304_band_reaches_the_server_at_its_own_numbers() {
    let (Some(pack), Some(root)) = (support::pack_304(), support::vendor(&["run", "file-root"]))
    else {
        return;
    };
    use muir::engine::Engine as _;
    let mut e = muir::rtl::Rtl::new(support::machine_with_pack(&pack));
    e.boot();
    let m = e.machine_mut();
    m.chaos.address = support::CHAOS_304.0;
    m.chaos.server_address = support::CHAOS_304.1;
    m.chaos.file_root = Some(root);
    m.chaos.time = Some(muir::chaos::time::TEST_UNIVERSAL);
    m.plug_chaos(0);
    m.ioboard.chaos.as_mut().unwrap().ether_mut().unwrap().keep_log(true);
    let ran = support::wait_for_the_prompt(&mut e);

    let ether = e.machine().ioboard.chaos.as_ref().unwrap().ether().unwrap();
    let packets: Vec<Packet> = ether
        .log
        .iter()
        .filter_map(|ev| match ev {
            muir::chaos::ether::Event::Sent(_, _, b) => Some(&b[..]),
            muir::chaos::ether::Event::Heard(_, f) => Some(&f.buffer[..]),
            muir::chaos::ether::Event::Collision(_) => None,
        })
        .filter_map(|b| Packet::from_buffer(b).ok().map(|(p, _)| p))
        .collect();
    eprintln!("at the prompt after {ran}: {} packets on the cable", packets.len());
    let time_rfc = packets
        .iter()
        .find(|p| p.opcode == op::RFC && p.data.starts_with(b"TIME"))
        .expect("the band asked for the time");
    assert_eq!(
        (time_rfc.source, time_rfc.dest),
        support::CHAOS_304,
        "from this machine to its file and time host"
    );
    assert!(
        packets.iter().any(|p| p.opcode == op::ANS && p.source == support::CHAOS_304.1),
        "and the server answered it"
    );
}

/// **A write is not put into place until the synchronous mark has come,
/// however early the CLOSE arrives.**
///
/// MIT's own `sys/doc/chfile.text` sets both halves of this. Of CLOSE it
/// says "a synchronous mark will be sent or awaited accordingly" ---
/// sent when reading, awaited when writing. And of the order the two
/// arrive in, its worked example of writing a file says to send "a SYNC
/// mark on the DATA connection and a CLOSE on the CONTROL connection (in
/// either order)". So a client that does everything the protocol asks of
/// it may still have its CLOSE overtake its mark, the two travelling on
/// different connections, and the server has to wait rather than take
/// the CLOSE as the end of the data.
///
/// A server that renames on the CLOSE alone puts a file into place that
/// is short of whatever had not arrived yet --- empty, if none of it
/// had. That is not the truncation `chfile.text` warns about a paragraph
/// later, which is a client's fault for not waiting for its EOF to be
/// acknowledged; this one is the server's, and it strikes a correct
/// client intermittently, on the timing of two connections.
///
/// Holding the reply back cannot hold the mark back with it: the
/// client writes the two without waiting between them ---
/// `qfile.lisp`'s `:COMMAND` sends the command packet on the control
/// connection and then, for an output stream, `(SEND STREAM
/// :WRITE-SYNCHRONOUS-MARK)`, before it waits for the response.
#[test]
fn a_write_is_not_placed_until_the_synchronous_mark_has_come() {
    let root = std::env::temp_dir().join(format!("muir-chaos-mark-{}", std::process::id()));
    let dir = root.join("tree/sys");
    std::fs::remove_dir_all(&root).ok();
    std::fs::create_dir_all(&dir).unwrap();
    let mut h = Server::new(0o3060);
    h.serve(Box::new(File::new(&root)));
    let mut c = Client::new(0o3050, 0o3060);
    c.connect(&mut h, 0);
    let nl = NEWLINE as char;
    c.command(&mut h, 10, "T1  LOGIN LISPM LISPM ");
    c.listening = Some("O0001".into());
    c.command(&mut h, 20, "T2  DATA-CONNECTION I0001 O0001");
    let r =
        c.command(&mut h, 30, &format!("T3 O0001 OPEN WRITE BINARY{nl}/tree/sys/late.qfasl{nl}"));
    assert!(r.starts_with("T3 O0001 OPEN "), "{r:?}");

    // The data, and then the CLOSE overtaking the mark.
    c.send_data(&mut h, 40, file::BINARY_OP, &[0o215, 0o12, 0, 0o377]);
    c.send_command(&mut h, 50, "T4 O0001 CLOSE");

    let real = dir.join("late.qfasl");
    let temporary = || {
        std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .any(|e| e.file_name().to_string_lossy().starts_with('#'))
    };
    assert!(!real.exists(), "the file waits for the mark, and does not go into place on the CLOSE");
    assert!(temporary(), "what has been written so far is still in the temporary");

    // The mark, and now it may go into place.
    c.send_data(&mut h, 60, file::SYNC_MARK_OP, &[]);
    let r = c.take_reply("T4").expect("the CLOSE is answered once the mark has come");
    assert!(r.starts_with("T4 O0001 CLOSE "), "{r:?}");
    assert_eq!(std::fs::read(&real).unwrap(), [0o215, 0o12, 0, 0o377], "every byte sent");
    assert!(!temporary(), "and the temporary is gone");

    std::fs::remove_dir_all(&root).ok();
}

/// **A CLOSE still waiting for its mark is answered when the transfer is
/// taken away under it.**
///
/// A write's CLOSE is held until the synchronous mark comes, so a client
/// that abandons the transfer instead of sending the mark --- an
/// `UNDATA-CONNECTION` on the handle --- would otherwise wait for a
/// reply that no longer had anything to come from. It gets `CNO`,
/// `chfile.text`'s "CLOSE on non-open channel": by the time the CLOSE
/// could be answered there was no channel left to close.
///
/// The band does not do this, and the test is here because the deferral
/// is what makes it possible to hang. Its `:REAL-CLOSE` waits for the
/// CLOSE's reply before freeing the data connection, and it only undoes
/// a connection that has gone dormant.
#[test]
fn a_close_waiting_for_its_mark_is_answered_if_the_transfer_goes_away() {
    let root = std::env::temp_dir().join(format!("muir-chaos-strand-{}", std::process::id()));
    let dir = root.join("tree/sys");
    std::fs::remove_dir_all(&root).ok();
    std::fs::create_dir_all(&dir).unwrap();
    let mut h = Server::new(0o3060);
    h.serve(Box::new(File::new(&root)));
    let mut c = Client::new(0o3050, 0o3060);
    c.connect(&mut h, 0);
    let nl = NEWLINE as char;
    c.command(&mut h, 10, "T1  LOGIN LISPM LISPM ");
    c.listening = Some("O0001".into());
    c.command(&mut h, 20, "T2  DATA-CONNECTION I0001 O0001");
    let r =
        c.command(&mut h, 30, &format!("T3 O0001 OPEN WRITE BINARY{nl}/tree/sys/gone.qfasl{nl}"));
    assert!(r.starts_with("T3 O0001 OPEN "), "{r:?}");
    c.send_data(&mut h, 40, file::BINARY_OP, &[1, 2, 3]);
    c.send_command(&mut h, 50, "T4 O0001 CLOSE");
    assert!(c.take_reply("T4").is_none(), "the CLOSE waits for the mark");

    // The mark never comes; the data connection is undone instead.
    let r = c.command(&mut h, 60, "T5 I0001 UNDATA-CONNECTION");
    assert!(r.starts_with("T5 I0001 UNDATA-CONNECTION"), "{r:?}");
    let r = c.take_reply("T4").expect("and the CLOSE is answered rather than left waiting");
    assert!(r.starts_with("T4 O0001 ERROR CNO C "), "{r:?}");
    assert!(!dir.join("gone.qfasl").exists(), "nothing went into place");
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0, "and no temporary was left");

    std::fs::remove_dir_all(&root).ok();
}

/// **A file being read and a file being written on one data connection
/// keep their own bytes.**
///
/// A data connection has two file handles and both halves may be busy at
/// once. MIT's `sys/doc/chfile.text` says what each is for --- "the input
/// file handle is used to describe the receive half of the DATA
/// connection and the output file handle is use to describe the send
/// half" --- and its `UNDATA-CONNECTION` implies "a CLOSE on each file
/// handle of the DATA connection for which there is a file transfer in
/// progress", each, so a transfer on both at the same time is the
/// protocol's own case rather than an odd one.
///
/// The band does it whenever it compiles. `sys/qcfile.lisp`'s `QC-FILE`
/// holds the source open in a `WITH-OPEN-STREAM` around the
/// `WITH-OPEN-FILE` that writes the QFASL, so the read is still open on
/// the receive half while the write runs on the send half, and it does
/// both over the one data connection it already has.
///
/// This is the test of a bug that cost an afternoon. The poll drained
/// every handle that had a transfer of any kind and handed what it found
/// to that handle; both handles share one channel, so whichever came
/// first took the lot, and when that was the input handle --- whose
/// transfer is a read, which has no use for arriving data --- the write's
/// bytes went on the floor and the rest of the poll cleared what was
/// left. The QFASL went into place empty, `make-system` would not
/// recompile it because it was newer than its source, and the band
/// stopped minutes later on `SYS: CC; LCADMC QFASL > is not a QFASL
/// file`. Three of the six QFASLs written across three runs were lost
/// that way.
///
/// **The handles are named so that the input one sorts first, and that is
/// what makes this test bite.** The poll walks the handles in order, and
/// the drain that loses the bytes is the input handle's; had the output
/// handle been walked first it would have found the write, put the bytes
/// where they belong, and this test would have passed on the broken code.
/// So `I0001` before `O0001` is not decoration: rename them to something
/// that sorts the other way and the test still passes, against a service
/// that has the bug back.
#[test]
fn a_read_and_a_write_on_one_data_connection_keep_their_own_bytes() {
    let root = std::env::temp_dir().join(format!("muir-chaos-both-{}", std::process::id()));
    let dir = root.join("tree").join("sys");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("source.lisp"), "(DEFUN F (X) X)").unwrap();

    let mut h = Server::new(0o3060);
    h.serve(Box::new(File::new(&root)));
    let mut c = Client::new(0o3050, 0o3060);
    c.connect(&mut h, 0);
    let nl = NEWLINE as char;
    c.command(&mut h, 10, "T1  LOGIN LISPM LISPM ");
    c.listening = Some("O0001".into());
    c.command(&mut h, 20, "T2  DATA-CONNECTION I0001 O0001");

    // The source on the receive half, and the QFASL on the send half
    // while that read is still open.
    let r = c.command(
        &mut h,
        30,
        &format!("T3 I0001 OPEN READ CHARACTER{nl}/tree/sys/source.lisp{nl}"),
    );
    assert!(r.starts_with("T3 I0001 OPEN "), "{r:?}");
    let r =
        c.command(&mut h, 40, &format!("T4 O0001 OPEN WRITE BINARY{nl}/tree/sys/source.qfasl{nl}"));
    assert!(r.starts_with("T4 O0001 OPEN "), "{r:?}");

    c.send_data(&mut h, 50, file::BINARY_OP, &[0o215, 0o12, 0, 0o377]);
    c.send_data(&mut h, 60, file::SYNC_MARK_OP, &[]);
    let r = c.command(&mut h, 70, "T5 O0001 CLOSE");
    assert!(r.starts_with("T5 O0001 CLOSE "), "{r:?}");
    assert_eq!(
        std::fs::read(dir.join("source.qfasl")).unwrap(),
        [0o215, 0o12, 0, 0o377],
        "the write kept every byte it sent, with a read open beside it"
    );

    // And the read is untouched by the write: its file still comes down.
    let read: Vec<u8> = c
        .down
        .iter()
        .filter(|(o, _)| *o == file::CHARACTER_OP)
        .flat_map(|(_, b)| b.clone())
        .collect();
    assert_eq!(text(&read), "(DEFUN F (X) X)", "and the read delivered its own file");

    std::fs::remove_dir_all(&root).ok();
}
