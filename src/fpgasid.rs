//! FPGASID diagnostic readback (feature `fpgasid`). Port of Clojure
//! `sid/fpgasid.clj` — issues a READ_CLONECHIP for the FPGASID at a
//! given SID base address and decodes the 64-byte response.

use crate::protocol::{encode_packet, PACKET_LEN};
use crate::transport::TransportError;

pub const FPGASID_BUFFER_LEN: usize = 64;

pub const READ_CLONECHIP: u8 = 0xA0;
pub const FPGASID_READ_ID: u8 = 0xA5;

/// Framing bytes: `[buf[0], buf[1], buf[62], buf[63]]` must equal
/// `[FPGASID_READ_ID, VERIFICATION_BYTE, END_BYTE, TERMINATION_BYTE]`.
pub const VERIFICATION_BYTE: u8 = 0x7F;
pub const END_BYTE: u8 = 0x8F;
pub const TERMINATION_BYTE: u8 = 0xFF;

pub const SLOTS: [&str; 2] = ["A", "B"];

pub const EXTIN_SOURCE_LABELS: [&str; 4] =
    ["analog input", "disabled", "other SID", "digifix (8580)"];
pub const READBACK_LABELS: [&str; 4] = [
    "bitrotation 6581",
    "always read value",
    "always read $00",
    "bitrotation 8580",
];
pub const REGDELAY_LABELS: [&str; 2] = ["6581", "8580"];
pub const MIXEDWAVE_LABELS: [&str; 2] = ["6581", "8580"];
pub const CRUNCHYDAC_LABELS: [&str; 2] = ["on (6581)", "off (8580)"];
pub const FILTERMODE_LABELS: [&str; 2] = ["6581", "8580"];
pub const OUTPUTMODE_LABELS: [&str; 2] = ["stereo", "mono mix"];
pub const SID2ADDR_LABELS: [&str; 5] = ["$d400", "$de00", "$d500", "$d420", ""];

/// Decoded diagnostic block for one SID.
#[derive(Debug, Clone)]
pub struct FpgaSidSlotConfig {
    pub slot: &'static str,
    pub sidno: u8,
    pub output_mode: &'static str,
    /// `[primary, addr1, addr2, addr3]` — the four candidate SID2 slots.
    pub sid2_addr: [&'static str; 4],
    pub extin_source: &'static str,
    pub readback: &'static str,
    pub reg_delay: &'static str,
    pub mixed_wave: &'static str,
    pub crunchy_dac: &'static str,
    pub filter_mode: &'static str,
    pub digifix: u8,
}

#[derive(Debug, Clone)]
pub struct FpgaSidDiag {
    pub idnum: u16,
    pub cpld: u8,
    pub fpga: u8,
    pub pca: u8,
    pub unique_id: [u8; 8],
    /// Clock period in microseconds (decoded from bytes 12-13).
    pub frequency_us: f32,
    pub select_pins: u8,
    pub idxa: u8,
    pub idxb: u8,
    pub flta: u8,
    pub fltb: u8,
    /// Four per-SID diagnostic blocks in `[slotA/SID1, slotA/SID2,
    /// slotB/SID1, slotB/SID2]` order — matches Clojure `sid1..sid4`.
    pub per_sid: [FpgaSidSlotConfig; 4],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FpgaSidError {
    WrongLength(usize),
    /// Framing bytes didn't match — wrong chip or corrupted read.
    VerificationFailed {
        got: [u8; 4],
        expected: [u8; 4],
    },
}

impl std::fmt::Display for FpgaSidError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WrongLength(n) => write!(
                f,
                "FPGASID diag buffer wrong length ({n} != {FPGASID_BUFFER_LEN})"
            ),
            Self::VerificationFailed { got, expected } => write!(
                f,
                "FPGASID diag framing mismatch: got {got:02X?}, expected {expected:02X?}"
            ),
        }
    }
}

impl std::error::Error for FpgaSidError {}

impl From<FpgaSidError> for TransportError {
    fn from(e: FpgaSidError) -> Self {
        TransportError::Io(e.to_string())
    }
}

/// READ_CLONECHIP packet for the FPGASID at SID base `addr`.
pub fn encode_read_fpgasid(addr: u8) -> [u8; PACKET_LEN] {
    encode_packet(READ_CLONECHIP, [FPGASID_READ_ID, addr, 0, 0])
}

/// Parse a 64-byte FPGASID readback. Port of Clojure `parse-read-buffer`.
pub fn parse_diag(buf: &[u8]) -> Result<FpgaSidDiag, FpgaSidError> {
    if buf.len() < FPGASID_BUFFER_LEN {
        return Err(FpgaSidError::WrongLength(buf.len()));
    }
    let expected = [
        FPGASID_READ_ID,
        VERIFICATION_BYTE,
        END_BYTE,
        TERMINATION_BYTE,
    ];
    let got = [buf[0], buf[1], buf[62], buf[63]];
    if got != expected {
        return Err(FpgaSidError::VerificationFailed { got, expected });
    }

    let id_hi = buf[2];
    let id_lo = buf[3];
    let idnum = ((id_hi as u16) << 8) | id_lo as u16;
    let freq_r = ((buf[12] as u32) << 8) | buf[13] as u32;
    // ((freq_r / 10) * 12.5) / 1000 = freq_r * 12.5 / 10_000 μs
    let frequency_us = (freq_r as f32) * 12.5 / 10_000.0;

    let unique_id: [u8; 8] = [
        buf[14], buf[15], buf[16], buf[17], buf[18], buf[19], buf[20], buf[21],
    ];

    // Per-SID triples: slot A / SID1 (22-24), SID2 (25-27); slot B / SID1
    // (28-30), SID2 (31-33).
    let per_sid = [
        parse_sid_block(SLOTS[0], 1, buf[22], buf[23], buf[24]),
        parse_sid_block(SLOTS[0], 2, buf[25], buf[26], buf[27]),
        parse_sid_block(SLOTS[1], 1, buf[28], buf[29], buf[30]),
        parse_sid_block(SLOTS[1], 2, buf[31], buf[32], buf[33]),
    ];

    Ok(FpgaSidDiag {
        idnum,
        cpld: buf[4],
        fpga: buf[5],
        pca: buf[6],
        select_pins: buf[7],
        idxa: buf[8],
        idxb: buf[9],
        flta: buf[10],
        fltb: buf[11],
        frequency_us,
        unique_id,
        per_sid,
    })
}

fn parse_sid_block(slot: &'static str, sidno: u8, zero: u8, one: u8, two: u8) -> FpgaSidSlotConfig {
    let extin = ((two & 0xC0) >> 6) as usize;
    let readb = ((two & 0x30) >> 4) as usize;
    let regd = ((two & 0x08) >> 3) as usize;
    let mixw = ((two & 0x04) >> 2) as usize;
    let crunch = ((two & 0x02) >> 1) as usize;
    let fltr = (two & 0x01) as usize;
    let outp = ((one & 0x08) >> 3) as usize;

    // SID2-address selection encodes 3 candidate bits in `one`:
    // bit 2 → addr1 index 1 (`$de00`) else 4 (`""`, disabled)
    // bit 1 → addr2 index 2 (`$d500`) else 4
    // bit 0 → addr3 index 3 (`$d420`) else 4
    // If all three are 4 (all disabled) → primary is `$d400` (index 0),
    // otherwise primary falls through to index 4 (disabled).
    let addr1 = if ((one & 0x04) >> 2) == 1 { 1 } else { 4 };
    let addr2 = if ((one & 0x02) >> 1) == 1 { 2 } else { 4 };
    let addr3 = if (one & 0x01) == 1 { 3 } else { 4 };
    let addr_primary = if addr1 + addr2 + addr3 == 12 { 0 } else { 4 };

    FpgaSidSlotConfig {
        slot,
        sidno,
        output_mode: OUTPUTMODE_LABELS[outp],
        sid2_addr: [
            SID2ADDR_LABELS[addr_primary],
            SID2ADDR_LABELS[addr1],
            SID2ADDR_LABELS[addr2],
            SID2ADDR_LABELS[addr3],
        ],
        extin_source: EXTIN_SOURCE_LABELS[extin],
        readback: READBACK_LABELS[readb],
        reg_delay: REGDELAY_LABELS[regd],
        mixed_wave: MIXEDWAVE_LABELS[mixw],
        crunchy_dac: CRUNCHYDAC_LABELS[crunch],
        filter_mode: FILTERMODE_LABELS[fltr],
        digifix: zero,
    }
}

impl<T: crate::transport::Transport> crate::device::Device<T> {
    /// Read + parse the FPGASID diagnostic at SID base `addr`.
    pub fn read_fpgasid_diag(
        &mut self,
        addr: u8,
    ) -> Result<FpgaSidDiag, crate::device::DeviceError> {
        self.transport_mut().drain()?;
        self.transport_mut().send(&encode_read_fpgasid(addr))?;
        let buf = self.transport_mut().recv(FPGASID_BUFFER_LEN)?;
        parse_diag(&buf)
            .map_err(|e| crate::device::DeviceError::Transport(TransportError::Io(e.to_string())))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Fixture from Clojure `dev-buffer` (`fpgasid.clj:118`), rewritten
    /// as `u8` (source uses signed literals).
    fn dev_buffer() -> [u8; FPGASID_BUFFER_LEN] {
        [
            0xA5, 0x7F, 0xF5, 0x1D, 0x05, 0x0A, 0x14, 0x14, 0x00, 0x00, 0x00, 0x00, 0x03, 0x2D,
            0x80, 0x98, 0x02, 0x14, 0x89, 0xFF, 0x65, 0x00, 0x52, 0x14, 0x01, 0xDF, 0x65, 0x00,
            0x52, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x8F, 0xFF,
        ]
    }

    #[test]
    fn parse_dev_buffer_verifies() {
        let d = parse_diag(&dev_buffer()).expect("dev buffer must verify");
        assert_eq!(d.idnum, 0xF51D);
        assert_eq!(d.cpld, 0x05);
        assert_eq!(d.fpga, 0x0A);
        assert_eq!(d.pca, 0x14);
        assert_eq!(d.select_pins, 0x14);
        assert_eq!(d.idxa, 0x00);
        assert_eq!(d.idxb, 0x00);
        assert_eq!(d.flta, 0x00);
        assert_eq!(d.fltb, 0x00);
    }

    #[test]
    fn parse_dev_buffer_frequency_matches_clojure_formula() {
        // Clojure formula: ((freq_r / 10) * 12.5) / 1000
        // buf[12] = 3, buf[13] = 45 → 0x032D = 813
        // → (813 / 10) * 12.5 / 1000 = 1.01625 μs
        let d = parse_diag(&dev_buffer()).unwrap();
        assert!(
            (d.frequency_us - 1.01625).abs() < 0.001,
            "frequency_us = {}",
            d.frequency_us
        );
    }

    #[test]
    fn per_sid_blocks_present_and_labelled() {
        let d = parse_diag(&dev_buffer()).unwrap();
        assert_eq!(d.per_sid[0].slot, "A");
        assert_eq!(d.per_sid[0].sidno, 1);
        assert_eq!(d.per_sid[1].slot, "A");
        assert_eq!(d.per_sid[1].sidno, 2);
        assert_eq!(d.per_sid[2].slot, "B");
        assert_eq!(d.per_sid[2].sidno, 1);
        assert_eq!(d.per_sid[3].slot, "B");
        assert_eq!(d.per_sid[3].sidno, 2);
        // Every per-SID entry must resolve every label — no empty strings
        // except in the sid2_addr candidate slots, which use `""` as the
        // "disabled" sentinel by design.
        for sid in &d.per_sid {
            assert!(!sid.output_mode.is_empty());
            assert!(!sid.extin_source.is_empty());
            assert!(!sid.readback.is_empty());
        }
    }

    #[test]
    fn wrong_length_rejected() {
        match parse_diag(&[0u8; 32]) {
            Err(FpgaSidError::WrongLength(n)) => assert_eq!(n, 32),
            other => panic!("expected WrongLength(32), got {other:?}"),
        }
    }

    #[test]
    fn framing_verification_reports_expected_bytes() {
        let mut buf = dev_buffer();
        buf[1] = 0xAA; // corrupt VERIFICATION_BYTE
        match parse_diag(&buf) {
            Err(FpgaSidError::VerificationFailed { got, expected }) => {
                assert_eq!(got[1], 0xAA);
                assert_eq!(
                    expected,
                    [
                        FPGASID_READ_ID,
                        VERIFICATION_BYTE,
                        END_BYTE,
                        TERMINATION_BYTE
                    ]
                );
            }
            other => panic!("expected VerificationFailed, got {other:?}"),
        }
    }

    #[test]
    fn encode_read_fpgasid_packet_shape() {
        let pkt = encode_read_fpgasid(0x00);
        // Framing byte + READ_CLONECHIP + FPGASID_READ_ID + addr + pad + pad
        assert_eq!(pkt[0], 0xD2);
        assert_eq!(pkt[1], READ_CLONECHIP);
        assert_eq!(pkt[2], FPGASID_READ_ID);
        assert_eq!(pkt[3], 0x00);
    }
}
