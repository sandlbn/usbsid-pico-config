//! Logical data model for a USBSID-Pico device's configuration.
//!
//! Enums and structs here mirror what the user sees in the Configtool UI;
//! they intentionally hide the wire-format bit-packing (e.g. the way socket
//! 1's two SID-id slots are squeezed into the high and low nibbles of a
//! single byte at offset 13). Those details live in [`crate::protocol`].

// ─────────────────────────────────────────────────────────────────────────────
//  Clock rates
// ─────────────────────────────────────────────────────────────────────────────

/// SID master clock the device drives onto its socket pins.
///
/// `Pal` (≈ 0.985 MHz) is the historical default for European C64s; `Ntsc`
/// (≈ 1.023 MHz) for North American ones. `Drean` and `Ntsc2` are minor
/// regional variants. `Default` lets the firmware pick (currently 1 MHz).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClockRate {
    Default,
    Pal,
    Ntsc,
    Drean,
    Ntsc2,
}

impl ClockRate {
    /// 4-bit id used in `SET_CONFIG` for section 0x0 ("clock").
    pub fn id(self) -> u8 {
        match self {
            Self::Default => 0,
            Self::Pal => 1,
            Self::Ntsc => 2,
            Self::Drean => 3,
            Self::Ntsc2 => 4,
        }
    }

    /// Inverse of [`Self::id`]. Returns `None` for unknown ids.
    pub fn from_id(id: u8) -> Option<Self> {
        Some(match id {
            0 => Self::Default,
            1 => Self::Pal,
            2 => Self::Ntsc,
            3 => Self::Drean,
            4 => Self::Ntsc2,
            _ => return None,
        })
    }

    /// Human-readable label (mirrors the Clojure tool's UI strings).
    pub fn label(self) -> &'static str {
        match self {
            Self::Default => "DEFAULT (1.000 MHz)",
            Self::Pal => "PAL (0.985 MHz)",
            Self::Ntsc => "NTSC (1.023 MHz)",
            Self::Drean => "DREAN (1.023 MHz)",
            Self::Ntsc2 => "NTSC2 (1.023 MHz)",
        }
    }

    /// Approximate clock value in Hz, as the firmware stores it (READ_CONFIG
    /// bytes 7..=9, big-endian 24-bit). Useful for displaying the literal
    /// frequency next to the label.
    pub fn hz(self) -> u32 {
        match self {
            Self::Default => 1_000_000,
            Self::Pal => 985_248,
            Self::Ntsc => 1_022_727,
            Self::Drean => 1_023_440,
            Self::Ntsc2 => 1_022_730,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  Chip & SID types
// ─────────────────────────────────────────────────────────────────────────────

/// What's physically installed in a USBSID-Pico socket. `Real` means a
/// genuine MOS SID; the other variants are clones / FPGA implementations
/// the firmware can recognise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChipType {
    Real,
    Unknown,
    SkPico,
    ArmSid,
    Arm2Sid,
    FpgaSid,
    RedipSid,
    PdSid,
    BackSid,
    SidEmu,
}

impl ChipType {
    pub fn id(self) -> u8 {
        match self {
            Self::Real => 0,
            Self::Unknown => 1,
            Self::SkPico => 2,
            Self::ArmSid => 3,
            Self::Arm2Sid => 4,
            Self::FpgaSid => 5,
            Self::RedipSid => 6,
            Self::PdSid => 7,
            Self::BackSid => 8,
            Self::SidEmu => 9,
        }
    }

    pub fn from_id(id: u8) -> Option<Self> {
        Some(match id {
            0 => Self::Real,
            1 => Self::Unknown,
            2 => Self::SkPico,
            3 => Self::ArmSid,
            4 => Self::Arm2Sid,
            5 => Self::FpgaSid,
            6 => Self::RedipSid,
            7 => Self::PdSid,
            8 => Self::BackSid,
            9 => Self::SidEmu,
            _ => return None,
        })
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Real => "Real SID",
            Self::Unknown => "Unknown",
            Self::SkPico => "SKPico",
            Self::ArmSid => "ARMSID",
            Self::Arm2Sid => "ARM2SID",
            Self::FpgaSid => "FPGASID",
            Self::RedipSid => "RedipSID",
            Self::PdSid => "PDSID",
            Self::BackSid => "BackSID",
            Self::SidEmu => "SIDEmu",
        }
    }
}

/// Per-slot SID model (a socket can hold one or two SIDs depending on chip).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SidType {
    Unknown,
    /// Not applicable — slot is empty / not used (e.g. socket configured as single-SID).
    Na,
    Mos8580,
    Mos6581,
    /// FMOpl audio chip (some clones expose this in place of a SID).
    FmOpl,
}

impl SidType {
    pub fn id(self) -> u8 {
        match self {
            Self::Unknown => 0,
            Self::Na => 1,
            Self::Mos8580 => 2,
            Self::Mos6581 => 3,
            Self::FmOpl => 4,
        }
    }

    pub fn from_id(id: u8) -> Option<Self> {
        Some(match id {
            0 => Self::Unknown,
            1 => Self::Na,
            2 => Self::Mos8580,
            3 => Self::Mos6581,
            4 => Self::FmOpl,
            _ => return None,
        })
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Unknown => "Unknown",
            Self::Na => "N/A",
            Self::Mos8580 => "MOS8580",
            Self::Mos6581 => "MOS6581",
            Self::FmOpl => "FMOpl",
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  Socket configuration
// ─────────────────────────────────────────────────────────────────────────────

/// One SID slot within a socket. `id` is the SID's index on the device bus
/// (0..3, with 4-bit width), `addr` is the C64-side base ($D400, $D420, …)
/// derived from `id`, and `kind` is the chip model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SidSlot {
    pub id: u8,
    pub addr: u8,
    pub kind: SidType,
}

impl SidSlot {
    /// Convert a SID id (0..3, or 0xFF for "absent") to its base register
    /// offset. Matches the Clojure tool's `sid-addr` helper.
    pub fn addr_for_id(id: u8) -> u8 {
        match id {
            0 => 0x00,
            1 => 0x20,
            2 => 0x40,
            3 => 0x60,
            _ => 0xFF,
        }
    }
}

/// One of the two physical sockets on the USBSID-Pico board.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SocketConfig {
    pub enabled: bool,
    /// True when the socket holds a clone chip presenting *two* SIDs.
    pub dualsid: bool,
    pub chip_type: ChipType,
    pub sid1: SidSlot,
    pub sid2: SidSlot,
}

// ─────────────────────────────────────────────────────────────────────────────
//  LED + RGB LED
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LedConfig {
    pub enabled: bool,
    pub idle_breathe: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RgbLedConfig {
    pub enabled: bool,
    pub idle_breathe: bool,
    pub brightness: u8,
    /// Which SID drives the LED colour. `-1` = off, `1..=4` = that SID.
    pub sid_to_use: i8,
}

impl Default for RgbLedConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            idle_breathe: false,
            brightness: 127,
            sid_to_use: -1,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  Protocol toggles + audio routing
// ─────────────────────────────────────────────────────────────────────────────

/// Firmware-side protocol toggles (CDC, WebUSB, ASID, MIDI, FMOpl).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ProtocolToggles {
    pub cdc: bool,
    pub webusb: bool,
    pub asid: bool,
    pub midi: bool,
    pub fmopl_enabled: bool,
    /// Which SID slot the FMOpl audio is routed through (0=unknown, 1..=4,
    /// 255=disabled). See the Clojure tool's `fmopl-sid-options`.
    pub fmopl_sidno: u8,
}

// ─────────────────────────────────────────────────────────────────────────────
//  Top-level DeviceConfig
// ─────────────────────────────────────────────────────────────────────────────

/// The full logical configuration of a USBSID-Pico device.
///
/// Produced by [`crate::protocol::decode_config`] from a 64-byte READ_CONFIG
/// buffer; consumed by [`crate::protocol::config_to_set_commands`] to
/// generate the SET_CONFIG triplets that push it back to the device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceConfig {
    /// True on v1.5+ boards that need an explicit CONFIG_ACK after first
    /// connection before they enable socket power rails.
    pub need_confirmation: bool,
    /// Diagnostic: socket change-detection has been disabled in firmware.
    pub disable_changedetect: bool,

    pub lock_clockrate: bool,
    pub external_clock: bool,
    pub clock_rate: ClockRate,

    pub socket1: SocketConfig,
    pub socket2: SocketConfig,

    pub led: LedConfig,
    pub rgb_led: RgbLedConfig,
    pub protocols: ProtocolToggles,

    pub stereo_enabled: bool,
    pub lock_audio_switch: bool,
    pub mirrored: bool,
    pub flipped: bool,
    pub mixed: bool,
}

// ─────────────────────────────────────────────────────────────────────────────
//  Presets
// ─────────────────────────────────────────────────────────────────────────────

/// Preset commands send a single configuration "macro" the firmware applies
/// atomically. Listed in the order the Clojure tool exposes them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Preset {
    SingleS1,
    SingleS2,
    DualBoth,
    DualS1,
    DualS2,
    TripleS1,
    TripleS2,
    Quad,
    Mirrored,
    MirroredDual,
    DualFlipped,
    QuadFlipped,
    QuadMixed,
    QuadFlipMixed,
}

impl Preset {
    /// Human-readable label.
    pub fn label(self) -> &'static str {
        match self {
            Self::SingleS1 => "SINGLE SID (Socket 1)",
            Self::SingleS2 => "SINGLE SID (Socket 2)",
            Self::DualBoth => "DUAL SID (S1 + S2)",
            Self::DualS1 => "DUAL SID (Socket 1 only)",
            Self::DualS2 => "DUAL SID (Socket 2 only)",
            Self::TripleS1 => "TRIPLE (Dual S1 + Single S2)",
            Self::TripleS2 => "TRIPLE (Single S1 + Dual S2)",
            Self::Quad => "QUAD SID (4 chips)",
            Self::Mirrored => "MIRRORED (S2 = S1)",
            Self::MirroredDual => "MIRRORED DUAL",
            Self::DualFlipped => "DUAL FLIPPED",
            Self::QuadFlipped => "QUAD FLIPPED",
            Self::QuadMixed => "QUAD MIXED",
            Self::QuadFlipMixed => "QUAD FLIP+MIX",
        }
    }
}
