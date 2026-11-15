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
    /// v1.5+: needs `CONFIG_ACK` before socket power rails come up.
    pub need_confirmation: bool,
    /// v1.5+: socket change-detect disabled. (SET_CONFIG section `0xF`.)
    pub disable_changedetect: bool,
    /// v1.5+: preset-autodetect (byte 4 bit 7). SET_CONFIG section `0x10`.
    pub preset_auto_detect: bool,
    /// v1.5+: last-applied preset id (byte 4 bits 0-6). READ-ONLY.
    pub last_preset: u8,

    pub lock_clockrate: bool,
    /// READ_CONFIG byte 6 bit 0.
    pub external_clock: bool,
    /// fw 0.9+: clock link (byte 6 bit 1). READ-ONLY here; set with
    /// `LINK_CLOCK` (0x84), which saves and reboots the board.
    pub clock_link: bool,
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

// ─── PCB revision + capability gating ────────────────────────────────────

/// PCB revision decoded from `US_PCB_VERSION` (e.g. `"1.5"`).
///
/// [`raw`](Self::raw) returns Clojure's folded integer form (`"1.5"` → 15)
/// for numeric feature comparisons; [`fw_capabilities`] is the preferred
/// API for gating.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PcbVersion {
    V1_0,
    V1_3,
    V1_4,
    V1_5,
    /// Anything newer than v1.5, stored as the folded integer.
    Newer(u32),
    Unknown,
}

impl PcbVersion {
    pub fn raw(self) -> u32 {
        match self {
            Self::V1_0 => 10,
            Self::V1_3 => 13,
            Self::V1_4 => 14,
            Self::V1_5 => 15,
            Self::Newer(v) => v,
            Self::Unknown => 0,
        }
    }

    pub fn from_raw(raw: u32) -> Self {
        match raw {
            0 => Self::Unknown,
            r if r >= 16 => Self::Newer(r),
            15 => Self::V1_5,
            14 => Self::V1_4,
            13 => Self::V1_3,
            _ => Self::V1_0,
        }
    }
}

/// Parse `"1.5"` / `"v1.5"` / `"1.5.0"` into Clojure's folded integer form
/// (`major * 10 + min(minor, 9)`). `None` on blank / unparseable input.
pub fn parse_pcb_version(s: &str) -> Option<u32> {
    let trimmed = s.trim().trim_start_matches(|c: char| c == 'v' || c == 'V');
    if trimmed.is_empty() {
        return None;
    }
    let mut parts = trimmed.split('.');
    let major: u32 = parts.next()?.parse().ok()?;
    let minor: u32 = parts.next().map_or(Some(0), |m| m.parse().ok())?;
    Some(major * 10 + minor.min(9))
}

/// Feature flags derived from the PCB revision — mirrors Clojure
/// `fw-capabilities`. Every field is `false` for `PcbVersion::Unknown`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Capabilities {
    pub audio_switch: bool,
    pub presets_v15: bool,
    pub autodetect: bool,
    pub mixed: bool,
    pub lock_audio_switch: bool,
    pub external_clock: bool,
    pub rgb_led: bool,
    pub fpga_diag: bool,
}

/// Firmware wire-format flavor.
///
/// `V07` is v0.7.0–v0.7.4; `Legacy` is v0.5/v0.6 (byte 22 = mirrored,
/// bytes 12/13 + 23/24 = split `{chiptype, clonetype}`, sections
/// `0xC..=0x10` absent).
///
/// `V075` (v0.7.5–v0.8.x): READ_CONFIG byte 3 / SET_CONFIG section `0xF`
/// changed meaning from "disable change detection" to
/// `socket_change_detect` (1 = enabled). `V09` (v0.9+): additionally byte 59
/// = USB RX ringbuffer. The [`DeviceConfig::disable_changedetect`] field
/// keeps its meaning across flavors; the codec inverts the wire bit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FwFlavor {
    #[default]
    V07,
    Legacy,
    V075,
    V09,
}

impl FwFlavor {
    /// Byte 3 / section 0xF carries `socket_change_detect` (1 = on).
    pub fn change_detect_is_enable_bit(self) -> bool {
        matches!(self, FwFlavor::V075 | FwFlavor::V09)
    }
}

/// `Legacy` below 0.7, `V07` for 0.7.0–0.7.4, `V075` for 0.7.5–0.8.x,
/// `V09` for 0.9 and newer. Accepts build suffixes such as
/// `"0.9.0-20261113"`. Unparseable input defaults to `V07`.
pub fn fw_flavor_from_version(s: &str) -> FwFlavor {
    let trimmed = s.trim().trim_start_matches(|c: char| c == 'v' || c == 'V');
    let num = |p: Option<&str>| -> Option<u32> {
        let p = p?;
        let digits: String = p.chars().take_while(|c| c.is_ascii_digit()).collect();
        digits.parse().ok()
    };
    let mut parts = trimmed.split('.');
    let major = match num(parts.next()) {
        Some(m) => m,
        None => return FwFlavor::V07,
    };
    let minor = num(parts.next()).unwrap_or(0);
    let patch = num(parts.next()).unwrap_or(0);
    match (major, minor, patch) {
        (0, m, _) if m < 7 => FwFlavor::Legacy,
        (0, 7, p) if p < 5 => FwFlavor::V07,
        (0, 7, _) | (0, 8, _) => FwFlavor::V075,
        _ => FwFlavor::V09,
    }
}

/// Legacy `{chiptype, clonetype}` byte pair → flat [`ChipType`].
/// Mirrors Clojure `config_model.clj:97 legacy->flat-chip`.
pub fn legacy_to_flat_chip(chiptype: u8, clonetype: u8) -> ChipType {
    match chiptype {
        0 => ChipType::Real,
        2 => ChipType::Unknown,
        1 => match clonetype {
            2 => ChipType::SkPico,
            3 => ChipType::ArmSid,
            4 => ChipType::FpgaSid,
            5 => ChipType::RedipSid,
            _ => ChipType::Unknown,
        },
        _ => ChipType::Unknown,
    }
}

/// Inverse of [`legacy_to_flat_chip`]. `None` for chips with no legacy
/// mapping (`Arm2Sid`, `PdSid`, `BackSid`, `SidEmu`) — legacy encoder
/// silently drops those writes.
pub fn flat_chip_to_legacy(chip: ChipType) -> Option<(u8, u8)> {
    match chip {
        ChipType::Real => Some((0, 0)),
        ChipType::Unknown => Some((2, 0)),
        ChipType::SkPico => Some((1, 2)),
        ChipType::ArmSid => Some((1, 3)),
        ChipType::FpgaSid => Some((1, 4)),
        ChipType::RedipSid => Some((1, 5)),
        _ => None,
    }
}

/// Capability set for a PCB revision. Thresholds: 13 → audio-switch;
/// 15 → v1.5+ bundle (presets, autodetect, mixed, external clock, RGB
/// LED, FPGASID diag).
pub fn fw_capabilities(pcb_ver: u32) -> Capabilities {
    let v13 = pcb_ver >= 13;
    let v15 = pcb_ver >= 15;
    Capabilities {
        audio_switch: v13,
        lock_audio_switch: v13,
        presets_v15: v15,
        autodetect: v15,
        mixed: v15,
        external_clock: v15,
        rgb_led: v15,
        fpga_diag: v15,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_pcb_version_common() {
        assert_eq!(parse_pcb_version("1.5"), Some(15));
        assert_eq!(parse_pcb_version("1.3"), Some(13));
        assert_eq!(parse_pcb_version("1.0"), Some(10));
        assert_eq!(parse_pcb_version("2.0"), Some(20));
    }

    #[test]
    fn parse_pcb_version_v_prefix_and_whitespace() {
        assert_eq!(parse_pcb_version("v1.5"), Some(15));
        assert_eq!(parse_pcb_version("V1.4"), Some(14));
        assert_eq!(parse_pcb_version("  1.5  "), Some(15));
    }

    #[test]
    fn parse_pcb_version_three_component_takes_only_major_minor() {
        // Real firmware never returns three-component versions, but be liberal.
        assert_eq!(parse_pcb_version("1.5.0"), Some(15));
        assert_eq!(parse_pcb_version("1.5.99"), Some(15));
    }

    #[test]
    fn parse_pcb_version_lone_major() {
        assert_eq!(parse_pcb_version("1"), Some(10));
        assert_eq!(parse_pcb_version("2"), Some(20));
    }

    #[test]
    fn parse_pcb_version_garbage_returns_none() {
        assert_eq!(parse_pcb_version(""), None);
        assert_eq!(parse_pcb_version("   "), None);
        assert_eq!(parse_pcb_version("banana"), None);
        assert_eq!(parse_pcb_version("1.x"), None);
    }

    #[test]
    fn pcb_version_roundtrip() {
        for (raw, expected) in [
            (0, PcbVersion::Unknown),
            (10, PcbVersion::V1_0),
            (12, PcbVersion::V1_0),
            (13, PcbVersion::V1_3),
            (14, PcbVersion::V1_4),
            (15, PcbVersion::V1_5),
            (16, PcbVersion::Newer(16)),
        ] {
            let v = PcbVersion::from_raw(raw);
            assert_eq!(v, expected, "from_raw({raw})");
        }
        // Fold-preserving for known variants.
        assert_eq!(PcbVersion::V1_5.raw(), 15);
        assert_eq!(PcbVersion::V1_3.raw(), 13);
    }

    #[test]
    fn fw_flavor_from_version_boundary() {
        assert_eq!(fw_flavor_from_version("0.5.0"), FwFlavor::Legacy);
        assert_eq!(fw_flavor_from_version("0.6.4"), FwFlavor::Legacy);
        assert_eq!(fw_flavor_from_version("0.7.0"), FwFlavor::V07);
        assert_eq!(fw_flavor_from_version("0.7.4"), FwFlavor::V07);
        // Byte 3 became socket_change_detect in 0.7.5.
        assert_eq!(fw_flavor_from_version("0.7.5"), FwFlavor::V075);
        assert_eq!(fw_flavor_from_version("0.8.0"), FwFlavor::V075);
        assert_eq!(fw_flavor_from_version("0.9.0-20261113"), FwFlavor::V09);
        assert_eq!(fw_flavor_from_version("1.0.0"), FwFlavor::V09);
        // Unparseable defaults to V07 — current is the safe assumption.
        assert_eq!(fw_flavor_from_version(""), FwFlavor::V07);
        assert_eq!(fw_flavor_from_version("garbage"), FwFlavor::V07);
        // Optional leading 'v' tolerated.
        assert_eq!(fw_flavor_from_version("v0.6.4"), FwFlavor::Legacy);
    }

    #[test]
    fn legacy_chip_mapping_roundtrip() {
        // Chips WITH a legacy mapping should survive to_legacy → from_legacy.
        for chip in [
            ChipType::Real,
            ChipType::Unknown,
            ChipType::SkPico,
            ChipType::ArmSid,
            ChipType::FpgaSid,
            ChipType::RedipSid,
        ] {
            let (ct, cl) = flat_chip_to_legacy(chip).expect("has legacy mapping");
            assert_eq!(legacy_to_flat_chip(ct, cl), chip, "chip = {chip:?}");
        }
        // Chips WITHOUT a legacy mapping return None from the encoder.
        for chip in [
            ChipType::Arm2Sid,
            ChipType::PdSid,
            ChipType::BackSid,
            ChipType::SidEmu,
        ] {
            assert_eq!(
                flat_chip_to_legacy(chip),
                None,
                "chip {chip:?} should have no legacy mapping"
            );
        }
        // Unrecognised legacy clonetype bytes collapse to Unknown.
        assert_eq!(legacy_to_flat_chip(1, 99), ChipType::Unknown);
        assert_eq!(legacy_to_flat_chip(9, 9), ChipType::Unknown);
    }

    #[test]
    fn fw_capabilities_boundaries() {
        // Unknown / <1.3: nothing.
        let none = fw_capabilities(0);
        assert!(!none.audio_switch && !none.presets_v15 && !none.rgb_led);

        // v1.3: audio switch only.
        let v13 = fw_capabilities(13);
        assert!(v13.audio_switch);
        assert!(v13.lock_audio_switch);
        assert!(!v13.presets_v15);
        assert!(!v13.rgb_led);

        // v1.4: still just the v1.3 stuff.
        let v14 = fw_capabilities(14);
        assert!(v14.audio_switch && !v14.presets_v15);

        // v1.5: full bundle.
        let v15 = fw_capabilities(15);
        assert!(v15.audio_switch && v15.presets_v15 && v15.autodetect);
        assert!(v15.mixed && v15.external_clock && v15.rgb_led && v15.fpga_diag);

        // Newer inherits v1.5 bundle.
        let v16 = fw_capabilities(16);
        assert!(v16.presets_v15 && v16.rgb_led);
    }
}
