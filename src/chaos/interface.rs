// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The Unibus Chaosnet interface as the software sees it: AIM-628 §7,
//! "Hardware Programming Documentation", the version "which attaches to
//! pdp11s and Lisp Machines". The I/O board's `lm*` pages are this
//! interface; `data/CADRIO.netlist` holds them.
//!
//! To transmit: "successive 16-bit words of the packet are written into
//! the outgoing packet buffer. First the eight 16-bit words of the header
//! should be written, then exactly the number of 16-bit data words implied
//! by the byte count in the header. ... After writing the data words, the
//! last 16-bit word to be written is the cable address of the destination
//! of the packet, or 0 to broadcast it. The hardware is then told to
//! initiate transmission" --- by reading [`START`]. The hardware appends
//! the source address and the check word itself.
//!
//! To receive: "the clear-receiver bit is asserted by the program. The
//! next packet on the cable which is addressed to this node, or is
//! broadcast, will be stored into the incoming packet buffer" and Receive
//! Done set; the words read back are the packet as written, then "the
//! destination address, the source address, and the checksum."

/// Command/Status Register. "All read/write bits are initialized to zero
/// on power-up."
pub const CSR: u32 = 0o764140;
/// Read: "the network address of this interface (which is contained in a
/// set of DIP switches on the board)."
pub const MY_ADDRESS: u32 = 0o764142;
/// Write: "a word into the outgoing packet buffer. The last word written
/// is the destination address."
pub const WRITE_BUFFER: u32 = 0o764142;
/// Read: "a word from the incoming packet buffer. The last three words
/// read are the destination address, the source address, and the
/// checksum."
pub const READ_BUFFER: u32 = 0o764144;
/// Read: "the number of bits in the incoming packet buffer, minus one.
/// After the whole packet has been read out, it will contain 7777 (a
/// 12-bit minus-one)."
pub const BIT_COUNT: u32 = 0o764146;
/// Read: "initiates transmission of the packet in the outgoing packet
/// buffer. The value read is the network address of this interface. This
/// method for starting transmission may seem strange, but it makes it
/// easier for the hardware to get the source address into the packet."
pub const START: u32 = 0o764152;

/// The bits of [`CSR`], "in the usual pdp11 style", by their masks as
/// AIM-628 §7 lists them.
pub mod csr {
    /// Timer Interrupt Enable (read/write), "for the interval timer present
    /// in some versions of the interface".
    pub const TIMER_INT_ENABLE: u16 = 0o1;
    /// Loop Back (read/write). "If this bit is 1, the cable and transceiver
    /// are not used and the interface is looped back to itself. This is
    /// for maintenance."
    pub const LOOP_BACK: u16 = 0o2;
    /// Spy (read/write): receive every packet regardless of destination.
    pub const SPY: u16 = 0o4;
    /// Clear Receiver (write only). "Writing a 1 into this bit clears
    /// Receive Done and enables the receiver to receive another packet."
    pub const CLEAR_RECEIVER: u16 = 0o10;
    /// Receive Interrupt Enable (read/write).
    pub const RECEIVE_INT_ENABLE: u16 = 0o20;
    /// Transmit Interrupt Enable (read/write).
    pub const TRANSMIT_INT_ENABLE: u16 = 0o40;
    /// Transmit Abort (read only): "the last transmission was aborted, by
    /// a collision or because the receiver was busy."
    pub const TRANSMIT_ABORT: u16 = 0o100;
    /// Transmit Done (read only): "set to 1 when a transmission is
    /// completed or aborted, and cleared to 0 when a word is written into
    /// the outgoing packet buffer."
    pub const TRANSMIT_DONE: u16 = 0o200;
    /// Clear Transmitter (write only): "stops the transmitter and sets
    /// Transmit Done. This is for maintenance."
    pub const CLEAR_TRANSMITTER: u16 = 0o400;
    /// Lost Count (read only), four bits: packets "which would have been
    /// received if the incoming packet buffer had not been busy."
    pub const LOST_COUNT: u16 = 0o17000;
    /// Reset (write only): "completely resets the interface, just as at
    /// power up and Unibus Initialize."
    pub const RESET: u16 = 0o20000;
    /// CRC Error (read only). "Only valid at two times: when the incoming
    /// packet buffer contains a fresh packet, and when the packet has been
    /// completely read out of the packet buffer."
    pub const CRC_ERROR: u16 = 0o40000;
    /// Receive Done (read only): "the incoming packet buffer contains a
    /// packet."
    pub const RECEIVE_DONE: u16 = 0o100000;
}

/// The two switch bodies that hold the address, LMMYNM D10 and D12 in
/// `data/CADRIO.netlist`, and the closed-switch masks that set them to
/// `address`: `(reference, closed)` for [`crate::chip::Chip::set_switches`].
///
/// Each body is eight switches to ground against a pull-up pack: D12 has
/// `MY#0` on pin 16 up to `MY#7` on pin 9, D10 `MY#8` on 16 up to `MY#15`
/// on 9, and a closed switch grounds its bit. So bit `k` of a body's mask,
/// the switch on pin `9 + k`, is address bit `7 - k` or `15 - k`, and a
/// bit of the address that is **one is an open switch**. Whether the
/// software then reads a one for an open switch is the board's to say:
/// `the_address_switches_read_back` in `tests/chaos_netlist.rs` reads
/// [`MY_ADDRESS`] and holds it to this.
pub fn switches(address: u16) -> [(&'static str, u8); 2] {
    let closed = |byte: u8| -> u8 {
        let mut m = 0;
        for k in 0..8 {
            if byte >> (7 - k) & 1 == 0 {
                m |= 1 << k;
            }
        }
        m
    };
    [("0D12", closed(address as u8)), ("0D10", closed((address >> 8) as u8))]
}
