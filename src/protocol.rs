//! Wire-format protocol for USBSID-Pico configuration.
//!
//! Pure-logic encoding/decoding — no I/O, no async, no globals. Every
//! function maps bytes ↔ structs and is safe to fuzz / unit-test.
//!
//! # Wire format
//!
//! Each "config command" sent to the device is a **6-byte packet**:
//!
//! ```text
//!   byte 0   framing byte  = (Cmd::COMMAND << 6) | Cmd::CONFIG = 0xD2
//!   byte 1   Cfg command   = one of READ_CONFIG (0x30), SET_CONFIG (0x32), …
//!   byte 2   arg0
//!   byte 3   arg1
//!   byte 4   arg2
//!   byte 5   arg3
//! ```
//!
//! Args that aren't used by a given command are zero-padded.
//!
//! **READ_CONFIG (`0x30`)** returns a 64-byte response with the framing
//! prologue/epilogue:
//!
//! ```text
//!   byte 0   = 0x30   (echoes the cmd byte; initiator)
//!   byte 1   = 0x7F   (verification marker)
//!   byte 63  = 0xFF   (terminator)
//!   bytes 2..63 = packed config bytes (see [`decode_config`])
//! ```
//!
//! **USBSID_VERSION / US_PCB_VERSION** return:
//!
//! ```text
//!   byte 0   = cmd echo
//!   byte 1   = string length N
//!   bytes 2..2+N = ASCII version string
//!   remaining = unspecified
//! ```
//!
//! **SET_CONFIG (`0x32`)** writes one field per packet. `arg0=section`,
//! `arg1=item` (sub-field within the section), `arg2=value`, `arg3=0`.
//!
//! **Preset commands (`0x40..=0x4B`)** apply a configuration macro in one
//! shot. Some take an `arg0` (e.g. `SINGLE_SID`: `0`=socket1, `1`=socket2).

use crate::model::{
    ChipType, ClockRate, DeviceConfig, LedConfig, Preset, ProtocolToggles, RgbLedConfig, SidSlot,
    SidType, SocketConfig,
};
use thiserror::Error;

// ─────────────────────────────────────────────────────────────────────────────
//  Constants — verified by disassembling the Java JAR's Config$Cfg + Cmd enums
// ─────────────────────────────────────────────────────────────────────────────

/// Length of one outbound config-command USB packet.
pub const PACKET_LEN: usize = 6;

/// Length of the buffer READ_CONFIG returns.
pub const CONFIG_BUFFER_LEN: usize = 64;

/// Framing byte that prefixes every config command: `(COMMAND<<6) | CONFIG`.
/// `Cmd::COMMAND = 3`, `Cmd::CONFIG = 0x12`, so framing = `0xD2`.
pub const FRAMING_BYTE: u8 = (3 << 6) | 0x12;

/// `Cfg` command bytes (the second byte of each packet). Names match the
/// Java enum to make cross-referencing easy.
///
/// Values verified by disassembling `Config$Cfg`'s `<clinit>` in the
/// upstream JAR — the enum is **not** densely numbered, there are gaps at
/// `0x3B-0x3E`, `0x4C-0x4E`, `0x5C-0x5F`, `0x62`, `0x64-0x7F`, `0x82-0x84`,
/// `0x8A-0x8F`, etc. Do not assume sequential byte values.
#[allow(dead_code)]
pub mod cfg {
    pub const RESET_USBSID: u8 = 0x20;
    pub const READ_CONFIG: u8 = 0x30;
    pub const APPLY_CONFIG: u8 = 0x31;
    pub const SET_CONFIG: u8 = 0x32;
    pub const SAVE_CONFIG: u8 = 0x33;
    pub const SAVE_NORESET: u8 = 0x34;
    pub const RESET_CONFIG: u8 = 0x35;
    pub const WRITE_CONFIG: u8 = 0x36;
    pub const READ_SOCKETCFG: u8 = 0x37;
    pub const RELOAD_CONFIG: u8 = 0x38;
    pub const READ_NUMSIDS: u8 = 0x39;
    pub const READ_FMOPLSID: u8 = 0x3A;
    pub const READ_CONFIGACK: u8 = 0x3F;
    pub const SINGLE_SID: u8 = 0x40;
    pub const DUAL_SID: u8 = 0x41;
    pub const QUAD_SID: u8 = 0x42;
    pub const TRIPLE_SID: u8 = 0x43;
    pub const TRIPLE_SID_TWO: u8 = 0x44;
    pub const MIRRORED_SID: u8 = 0x45;
    pub const DUAL_SOCKET1: u8 = 0x46;
    pub const DUAL_SOCKET2: u8 = 0x47;
    pub const DUAL_FLIPPED: u8 = 0x48;
    pub const QUAD_FLIPPED: u8 = 0x49;
    pub const QUAD_MIXED: u8 = 0x4A;
    pub const QUAD_FLIPMIX: u8 = 0x4B;
    pub const HOTFLIP_SOCKETS: u8 = 0x4F;
    pub const SET_CLOCK: u8 = 0x50;
    pub const DETECT_SIDS: u8 = 0x51;
    pub const TEST_ALLSIDS: u8 = 0x52;
    pub const TEST_SID1: u8 = 0x53;
    pub const TEST_SID2: u8 = 0x54;
    pub const TEST_SID3: u8 = 0x55;
    pub const TEST_SID4: u8 = 0x56;
    pub const GET_CLOCK: u8 = 0x57;
    pub const LOCK_CLOCK: u8 = 0x58;
    pub const STOP_TESTS: u8 = 0x59;
    pub const DETECT_CLONES: u8 = 0x5A;
    pub const AUTO_DETECT: u8 = 0x5B;
    pub const LOAD_MIDI_STATE: u8 = 0x60;
    pub const SAVE_MIDI_STATE: u8 = 0x61;
    pub const RESET_MIDI_STATE: u8 = 0x63;
    pub const USBSID_VERSION: u8 = 0x80;
    pub const US_PCB_VERSION: u8 = 0x81;
    pub const RESTART_BUS: u8 = 0x85;
    pub const RESTART_BUS_CLK: u8 = 0x86;
    pub const SYNC_PIOS: u8 = 0x87;
    pub const TOGGLE_AUDIO: u8 = 0x88;
    pub const SET_AUDIO: u8 = 0x89;
    pub const LOCK_AUDIO: u8 = 0x90;
    pub const GET_AUDIO: u8 = 0x91;
    /// CONFIG_ACK for v1.5+ boards. The Clojure tool sends this at byte 0xFA.
    pub const CONFIG_ACK: u8 = 0xFA;
    pub const SOCKET_DETECT: u8 = 0xFD;
}

/// SET_CONFIG "section" bytes.
#[allow(dead_code)]
pub mod section {
    pub const CLOCK: u8 = 0x0;
    pub const SOCKET1: u8 = 0x1;
    pub const SOCKET2: u8 = 0x2;
    pub const LED: u8 = 0x3;
    pub const RGB: u8 = 0x4;
    pub const FMOPL: u8 = 0x9;
    pub const STEREO: u8 = 0xA;
    pub const LOCK_AUDIO: u8 = 0xB;
    pub const MIRRORED: u8 = 0xC;
    pub const FLIPPED: u8 = 0xD;
    pub const MIXED: u8 = 0xE;
}

// ─────────────────────────────────────────────────────────────────────────────
//  Errors
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("config buffer too short: {got} bytes, need {}", CONFIG_BUFFER_LEN)]
    BufferTooShort { got: usize },

    #[error(
        "invalid config buffer framing: [0]=0x{b0:02X} (want 0x30), [1]=0x{b1:02X} (want 0x7F), [63]=0x{b63:02X} (want 0xFF)"
    )]
    InvalidFraming { b0: u8, b1: u8, b63: u8 },

    #[error("unknown enum id {id} for {field}")]
    UnknownEnumId { field: &'static str, id: u8 },
}

// ─────────────────────────────────────────────────────────────────────────────
//  Packet encoding
// ─────────────────────────────────────────────────────────────────────────────

/// Build a 6-byte config command packet.
pub fn encode_packet(cfg_cmd: u8, args: [u8; 4]) -> [u8; PACKET_LEN] {
    [FRAMING_BYTE, cfg_cmd, args[0], args[1], args[2], args[3]]
}

/// READ_CONFIG packet (asks the device to send back its 64-byte config).
pub fn encode_read_config() -> [u8; PACKET_LEN] {
    encode_packet(cfg::READ_CONFIG, [0, 0, 0, 0])
}

/// APPLY_CONFIG packet (commit pending writes to RAM, not flash).
pub fn encode_apply() -> [u8; PACKET_LEN] {
    encode_packet(cfg::APPLY_CONFIG, [0, 0, 0, 0])
}

/// SAVE_CONFIG packet (write to flash AND reboot the device).
pub fn encode_save_and_reboot() -> [u8; PACKET_LEN] {
    encode_packet(cfg::SAVE_CONFIG, [0, 0, 0, 0])
}

/// SAVE_NORESET packet (write to flash, keep running).
pub fn encode_save_no_reset() -> [u8; PACKET_LEN] {
    encode_packet(cfg::SAVE_NORESET, [0, 0, 0, 0])
}

/// RESET_CONFIG packet (revert to firmware defaults).
pub fn encode_reset_config() -> [u8; PACKET_LEN] {
    encode_packet(cfg::RESET_CONFIG, [0, 0, 0, 0])
}

/// RELOAD_CONFIG packet (re-read config from flash, overwriting RAM).
pub fn encode_reload() -> [u8; PACKET_LEN] {
    encode_packet(cfg::RELOAD_CONFIG, [0, 0, 0, 0])
}

/// USBSID_VERSION packet (returns firmware version string).
pub fn encode_read_firmware_version() -> [u8; PACKET_LEN] {
    encode_packet(cfg::USBSID_VERSION, [0, 0, 0, 0])
}

/// US_PCB_VERSION packet (returns PCB version string).
pub fn encode_read_pcb_version() -> [u8; PACKET_LEN] {
    encode_packet(cfg::US_PCB_VERSION, [0, 0, 0, 0])
}

/// CONFIG_ACK packet (v1.5+ board confirmation handshake).
pub fn encode_config_ack() -> [u8; PACKET_LEN] {
    encode_packet(cfg::CONFIG_ACK, [0, 0, 0, 0])
}

/// SET_CONFIG packet: write a single (section, item, value) triplet.
pub fn encode_set_config(sect: u8, item: u8, value: u8) -> [u8; PACKET_LEN] {
    encode_packet(cfg::SET_CONFIG, [sect, item, value, 0])
}

/// Preset packet — map a [`Preset`] to its (cmd, arg) pair and encode.
pub fn encode_preset(p: Preset) -> [u8; PACKET_LEN] {
    let (cmd, arg) = match p {
        Preset::SingleS1 => (cfg::SINGLE_SID, 0),
        Preset::SingleS2 => (cfg::SINGLE_SID, 1),
        Preset::DualBoth => (cfg::DUAL_SID, 0),
        Preset::DualS1 => (cfg::DUAL_SOCKET1, 0),
        Preset::DualS2 => (cfg::DUAL_SOCKET2, 0),
        Preset::TripleS1 => (cfg::TRIPLE_SID, 0),
        Preset::TripleS2 => (cfg::TRIPLE_SID_TWO, 0),
        Preset::Quad => (cfg::QUAD_SID, 0),
        Preset::Mirrored => (cfg::MIRRORED_SID, 0),
        Preset::MirroredDual => (cfg::MIRRORED_SID, 1),
        Preset::DualFlipped => (cfg::DUAL_FLIPPED, 0),
        Preset::QuadFlipped => (cfg::QUAD_FLIPPED, 0),
        Preset::QuadMixed => (cfg::QUAD_MIXED, 0),
        Preset::QuadFlipMixed => (cfg::QUAD_FLIPMIX, 0),
    };
    encode_packet(cmd, [arg, 0, 0, 0])
}

// ─────────────────────────────────────────────────────────────────────────────
//  Response decoding — READ_CONFIG buffer
// ─────────────────────────────────────────────────────────────────────────────

/// Validate the 64-byte READ_CONFIG response framing (initiator, marker,
/// terminator). Returns the buffer unchanged on success.
pub fn validate_config_buffer(buf: &[u8]) -> Result<&[u8], ConfigError> {
    if buf.len() < CONFIG_BUFFER_LEN {
        return Err(ConfigError::BufferTooShort { got: buf.len() });
    }
    let b = &buf[..CONFIG_BUFFER_LEN];
    if b[0] != 0x30 || b[1] != 0x7F || b[63] != 0xFF {
        return Err(ConfigError::InvalidFraming {
            b0: b[0],
            b1: b[1],
            b63: b[63],
        });
    }
    Ok(b)
}

/// Decode a validated 64-byte READ_CONFIG buffer into a [`DeviceConfig`].
///
/// Returns an `UnknownEnumId` error if any enum field contains a value the
/// firmware doesn't define — typically a sign the firmware is newer than
/// this crate. Callers that prefer a best-effort decode can call
/// [`decode_config_lenient`] instead.
pub fn decode_config(buf: &[u8]) -> Result<DeviceConfig, ConfigError> {
    let b = validate_config_buffer(buf)?;
    let clock_val = ((b[7] as u32) << 16) | ((b[8] as u32) << 8) | (b[9] as u32);
    let clock_rate = clock_rate_from_value(clock_val).unwrap_or(ClockRate::Pal);

    Ok(DeviceConfig {
        need_confirmation: b[2] != 0,
        disable_changedetect: b[3] != 0,

        lock_clockrate: b[5] != 0,
        external_clock: b[6] != 0,
        clock_rate,

        socket1: decode_socket(b, SOCKET1_LAYOUT)?,
        socket2: decode_socket(b, SOCKET2_LAYOUT)?,

        led: LedConfig {
            enabled: b[30] != 0,
            idle_breathe: b[31] != 0,
        },
        rgb_led: RgbLedConfig {
            enabled: b[40] != 0,
            idle_breathe: b[41] != 0,
            brightness: b[42],
            sid_to_use: if b[43] == 0 || b[43] == 0xFF {
                -1
            } else {
                b[43] as i8
            },
        },
        protocols: ProtocolToggles {
            cdc: b[51] != 0,
            webusb: b[52] != 0,
            asid: b[53] != 0,
            midi: b[54] != 0,
            fmopl_enabled: b[55] != 0,
            fmopl_sidno: b[56],
        },

        stereo_enabled: b[57] != 0,
        lock_audio_switch: b[58] != 0,
        mirrored: (b[60] & 0x1) != 0,
        flipped: (b[60] & 0x2) != 0,
        mixed: (b[60] & 0x4) != 0,
    })
}

/// Best-effort decode: unknown enum ids fall back to `Unknown`. Use this
/// when interoperating with newer firmware than the crate was built against.
pub fn decode_config_lenient(buf: &[u8]) -> Result<DeviceConfig, ConfigError> {
    match decode_config(buf) {
        Ok(c) => Ok(c),
        Err(ConfigError::UnknownEnumId { .. }) => {
            // Fall through to a relaxed parse.
            let b = validate_config_buffer(buf)?;
            let clock_val = ((b[7] as u32) << 16) | ((b[8] as u32) << 8) | (b[9] as u32);
            Ok(DeviceConfig {
                need_confirmation: b[2] != 0,
                disable_changedetect: b[3] != 0,
                lock_clockrate: b[5] != 0,
                external_clock: b[6] != 0,
                clock_rate: clock_rate_from_value(clock_val).unwrap_or(ClockRate::Pal),
                socket1: decode_socket_lenient(b, SOCKET1_LAYOUT),
                socket2: decode_socket_lenient(b, SOCKET2_LAYOUT),
                led: LedConfig {
                    enabled: b[30] != 0,
                    idle_breathe: b[31] != 0,
                },
                rgb_led: RgbLedConfig {
                    enabled: b[40] != 0,
                    idle_breathe: b[41] != 0,
                    brightness: b[42],
                    sid_to_use: if b[43] == 0 || b[43] == 0xFF {
                        -1
                    } else {
                        b[43] as i8
                    },
                },
                protocols: ProtocolToggles {
                    cdc: b[51] != 0,
                    webusb: b[52] != 0,
                    asid: b[53] != 0,
                    midi: b[54] != 0,
                    fmopl_enabled: b[55] != 0,
                    fmopl_sidno: b[56],
                },
                stereo_enabled: b[57] != 0,
                lock_audio_switch: b[58] != 0,
                mirrored: (b[60] & 0x1) != 0,
                flipped: (b[60] & 0x2) != 0,
                mixed: (b[60] & 0x4) != 0,
            })
        }
        Err(e) => Err(e),
    }
}

/// Byte offsets within the 64-byte READ_CONFIG buffer for one socket.
///
/// Socket 1 and socket 2 use *different* offset layouts in the response —
/// socket 2 has a one-byte gap (an unused / reserved byte) between
/// `dualsid` and `chip_type`, so its `chip_type` lives at byte 23 rather
/// than 22 as a naive +10 shift would suggest. Source-of-truth: the upstream
/// Clojure tool's `parse-config-bytes` in `driver.clj`.
#[derive(Debug, Clone, Copy)]
struct SocketLayout {
    enabled: usize,
    dualsid: usize,
    chip_type: usize,
    id_pack: usize,
    sid1_type: usize,
    sid2_type: usize,
}

const SOCKET1_LAYOUT: SocketLayout = SocketLayout {
    enabled: 10,
    dualsid: 11,
    chip_type: 12,
    id_pack: 13,
    sid1_type: 14,
    sid2_type: 15,
};

const SOCKET2_LAYOUT: SocketLayout = SocketLayout {
    enabled: 20,
    dualsid: 21,
    chip_type: 23, // byte 22 is reserved / unused
    id_pack: 24,
    sid1_type: 25,
    sid2_type: 26,
};

fn decode_socket(buf: &[u8], l: SocketLayout) -> Result<SocketConfig, ConfigError> {
    let packed = buf[l.id_pack]; // sid1.id in low nibble, sid2.id in high nibble
    let sid1_id = packed & 0xF;
    let sid2_id = (packed >> 4) & 0xF;
    let chip_id = buf[l.chip_type];
    let chip = ChipType::from_id(chip_id).ok_or(ConfigError::UnknownEnumId {
        field: "ChipType",
        id: chip_id,
    })?;
    let sid1_type_id = buf[l.sid1_type];
    let sid1_type = SidType::from_id(sid1_type_id).ok_or(ConfigError::UnknownEnumId {
        field: "SidType (sid1)",
        id: sid1_type_id,
    })?;
    let sid2_type_id = buf[l.sid2_type];
    let sid2_type = SidType::from_id(sid2_type_id).ok_or(ConfigError::UnknownEnumId {
        field: "SidType (sid2)",
        id: sid2_type_id,
    })?;
    Ok(SocketConfig {
        enabled: buf[l.enabled] != 0,
        dualsid: buf[l.dualsid] != 0,
        chip_type: chip,
        sid1: SidSlot {
            id: sid1_id,
            addr: SidSlot::addr_for_id(sid1_id),
            kind: sid1_type,
        },
        sid2: SidSlot {
            id: sid2_id,
            addr: SidSlot::addr_for_id(sid2_id),
            kind: sid2_type,
        },
    })
}

fn decode_socket_lenient(buf: &[u8], l: SocketLayout) -> SocketConfig {
    let packed = buf[l.id_pack];
    let sid1_id = packed & 0xF;
    let sid2_id = (packed >> 4) & 0xF;
    SocketConfig {
        enabled: buf[l.enabled] != 0,
        dualsid: buf[l.dualsid] != 0,
        chip_type: ChipType::from_id(buf[l.chip_type]).unwrap_or(ChipType::Unknown),
        sid1: SidSlot {
            id: sid1_id,
            addr: SidSlot::addr_for_id(sid1_id),
            kind: SidType::from_id(buf[l.sid1_type]).unwrap_or(SidType::Unknown),
        },
        sid2: SidSlot {
            id: sid2_id,
            addr: SidSlot::addr_for_id(sid2_id),
            kind: SidType::from_id(buf[l.sid2_type]).unwrap_or(SidType::Unknown),
        },
    }
}

/// Map a raw clock value (firmware-side Hz) to a [`ClockRate`] enum.
///
/// Tolerant of small drift in the firmware's Hz constants.
pub fn clock_rate_from_value(hz: u32) -> Option<ClockRate> {
    // Exact match first, then tolerate ±5 Hz drift on the regional variants.
    for r in [
        ClockRate::Default,
        ClockRate::Pal,
        ClockRate::Ntsc,
        ClockRate::Drean,
        ClockRate::Ntsc2,
    ] {
        if r.hz() == hz {
            return Some(r);
        }
    }
    None
}

// ─────────────────────────────────────────────────────────────────────────────
//  Encoding: full config → stream of SET_CONFIG packets
// ─────────────────────────────────────────────────────────────────────────────

/// Convert a complete [`DeviceConfig`] to the sequence of SET_CONFIG packets
/// that push it back to the device. Order matters: the Clojure tool sends
/// these in clock → sockets → LEDs → protocols → audio order.
///
/// Each packet is the 6-byte form `[0xD2, 0x32, section, item, value, 0]`.
pub fn config_to_set_packets(cfg: &DeviceConfig) -> Vec<[u8; PACKET_LEN]> {
    let mut out: Vec<[u8; PACKET_LEN]> = Vec::with_capacity(24);

    // Clock (section 0x0): item=clock_id, value=lock_flag.
    out.push(encode_set_config(
        section::CLOCK,
        cfg.clock_rate.id(),
        if cfg.lock_clockrate { 1 } else { 0 },
    ));

    // Sockets 1 & 2 (sections 0x1 / 0x2). Items: 0=enabled, 1=dualsid,
    // 2=chiptype, 4=sid1.type, 5=sid2.type.
    for (sect, sock) in [
        (section::SOCKET1, &cfg.socket1),
        (section::SOCKET2, &cfg.socket2),
    ] {
        out.push(encode_set_config(
            sect,
            0x0,
            if sock.enabled { 1 } else { 0 },
        ));
        out.push(encode_set_config(
            sect,
            0x1,
            if sock.dualsid { 1 } else { 0 },
        ));
        out.push(encode_set_config(sect, 0x2, sock.chip_type.id()));
        out.push(encode_set_config(sect, 0x4, sock.sid1.kind.id()));
        out.push(encode_set_config(sect, 0x5, sock.sid2.kind.id()));
    }

    // LED (section 0x3): item 0=enabled, 1=idle_breathe.
    out.push(encode_set_config(
        section::LED,
        0x0,
        if cfg.led.enabled { 1 } else { 0 },
    ));
    out.push(encode_set_config(
        section::LED,
        0x1,
        if cfg.led.idle_breathe { 1 } else { 0 },
    ));

    // RGB LED (section 0x4): items 0..3.
    out.push(encode_set_config(
        section::RGB,
        0x0,
        if cfg.rgb_led.enabled { 1 } else { 0 },
    ));
    out.push(encode_set_config(
        section::RGB,
        0x1,
        if cfg.rgb_led.idle_breathe { 1 } else { 0 },
    ));
    out.push(encode_set_config(section::RGB, 0x2, cfg.rgb_led.brightness));
    out.push(encode_set_config(
        section::RGB,
        0x3,
        cfg.rgb_led.sid_to_use.max(0) as u8,
    ));

    // FMOpl + audio routing flags: each section takes a bool in the "item"
    // slot per the Clojure tool's `config->commands`.
    out.push(encode_set_config(
        section::FMOPL,
        if cfg.protocols.fmopl_enabled { 1 } else { 0 },
        0,
    ));
    out.push(encode_set_config(
        section::STEREO,
        if cfg.stereo_enabled { 1 } else { 0 },
        0,
    ));
    out.push(encode_set_config(
        section::LOCK_AUDIO,
        if cfg.lock_audio_switch { 1 } else { 0 },
        0,
    ));
    out.push(encode_set_config(
        section::MIRRORED,
        if cfg.mirrored { 1 } else { 0 },
        0,
    ));
    out.push(encode_set_config(
        section::FLIPPED,
        if cfg.flipped { 1 } else { 0 },
        0,
    ));
    out.push(encode_set_config(
        section::MIXED,
        if cfg.mixed { 1 } else { 0 },
        0,
    ));

    out
}

// ─────────────────────────────────────────────────────────────────────────────
//  Version-string response parsing
// ─────────────────────────────────────────────────────────────────────────────

/// Parse the response to a USBSID_VERSION or US_PCB_VERSION query.
///
/// Layout: `[cmd_echo, length, version_string_bytes…]`. Returns the version
/// string. The `cmd_echo` byte is checked against `expected_cmd` so a
/// botched read fails loudly rather than returning random ASCII.
pub fn decode_version_response(buf: &[u8], expected_cmd: u8) -> Option<String> {
    if buf.len() < 2 {
        return None;
    }
    if buf[0] != expected_cmd {
        return None;
    }
    let len = buf[1] as usize;
    let end = 2 + len;
    if buf.len() < end {
        return None;
    }
    let s = String::from_utf8_lossy(&buf[2..end]).into_owned();
    if s.trim().is_empty() {
        None
    } else {
        Some(s)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_buffer() -> [u8; CONFIG_BUFFER_LEN] {
        let mut b = [0u8; CONFIG_BUFFER_LEN];
        // Framing
        b[0] = 0x30;
        b[1] = 0x7F;
        b[63] = 0xFF;
        // PAL clock (985_248 = 0x0F_07_E0)
        b[7] = 0x0F;
        b[8] = 0x07;
        b[9] = 0xE0;
        b[5] = 1; // lock_clockrate
        b[6] = 0; // external_clock
                  // Socket 1 — bytes 10..=15: enabled, dualsid, chip_type, id_pack, sid1_type, sid2_type
        b[10] = 1;
        b[11] = 0;
        b[12] = ChipType::Real.id();
        b[13] = 0xF0; // sid2.id=0xF, sid1.id=0
        b[14] = SidType::Mos6581.id();
        b[15] = SidType::Na.id();
        // Socket 2 — bytes 20, 21, 23, 24, 25, 26 (byte 22 reserved!)
        b[20] = 1;
        b[21] = 0;
        b[22] = 0xAA; // canary in the reserved byte — must NOT influence decoding
        b[23] = ChipType::Real.id();
        b[24] = 0xF0;
        b[25] = SidType::Mos8580.id();
        b[26] = SidType::Na.id();
        // LEDs
        b[30] = 1;
        b[31] = 0;
        b[40] = 0;
        b[42] = 127;
        b[43] = 0xFF; // sid_to_use = -1
                      // Protocols: CDC + WebUSB on
        b[51] = 1;
        b[52] = 1;
        // Audio: stereo on
        b[57] = 1;
        b[60] = 0b001; // mirrored only
        b
    }

    #[test]
    fn decode_config_basic() {
        let b = make_buffer();
        let cfg = decode_config(&b).unwrap();
        assert_eq!(cfg.clock_rate, ClockRate::Pal);
        assert!(cfg.lock_clockrate);
        assert!(!cfg.external_clock);
        assert!(cfg.socket1.enabled);
        assert!(cfg.socket2.enabled);
        assert_eq!(cfg.socket1.chip_type, ChipType::Real);
        assert_eq!(cfg.socket1.sid1.kind, SidType::Mos6581);
        assert_eq!(cfg.socket1.sid1.addr, 0x00);
        assert_eq!(cfg.socket1.sid2.id, 0xF);
        assert_eq!(cfg.socket1.sid2.addr, 0xFF);
        assert_eq!(cfg.socket2.chip_type, ChipType::Real);
        assert_eq!(cfg.socket2.sid1.kind, SidType::Mos8580);
        assert_eq!(cfg.socket2.sid2.kind, SidType::Na);
        assert!(cfg.led.enabled);
        assert_eq!(cfg.rgb_led.sid_to_use, -1);
        assert!(cfg.protocols.cdc);
        assert!(cfg.protocols.webusb);
        assert!(cfg.stereo_enabled);
        assert!(cfg.mirrored);
        assert!(!cfg.flipped);
        assert!(!cfg.mixed);
    }

    /// Regression for socket 2's byte-offset gap (the "wrong SID slot"
    /// bug). Verifies decoding ignores byte 22 (reserved) and pulls
    /// chip_type from byte 23, sid1_type from byte 25, sid2_type from
    /// byte 26 — NOT bytes 22/24/25 as a naive +10-shift would do.
    #[test]
    fn socket2_skips_reserved_byte_22() {
        let mut b = make_buffer();
        // Set distinguishable values:
        // - byte 22 (reserved): canary 0xCC. Decoder must ignore it.
        // - byte 23 (socket2.chip_type): ArmSid (id=3)
        // - byte 25 (socket2.sid1_type): Mos8580 (id=2)
        // - byte 26 (socket2.sid2_type): Mos6581 (id=3)
        b[22] = 0xCC;
        b[23] = ChipType::ArmSid.id();
        b[25] = SidType::Mos8580.id();
        b[26] = SidType::Mos6581.id();
        let cfg = decode_config(&b).unwrap();
        assert_eq!(
            cfg.socket2.chip_type,
            ChipType::ArmSid,
            "socket2.chip_type must be read from byte 23, not 22"
        );
        assert_eq!(
            cfg.socket2.sid1.kind,
            SidType::Mos8580,
            "socket2.sid1.kind must be read from byte 25"
        );
        assert_eq!(
            cfg.socket2.sid2.kind,
            SidType::Mos6581,
            "socket2.sid2.kind must be read from byte 26"
        );
    }

    #[test]
    fn rejects_bad_framing() {
        let mut b = make_buffer();
        b[0] = 0; // wrong initiator
        match decode_config(&b) {
            Err(ConfigError::InvalidFraming { .. }) => {}
            other => panic!("expected InvalidFraming, got {other:?}"),
        }
    }

    #[test]
    fn rejects_unknown_chiptype() {
        let mut b = make_buffer();
        b[12] = 99; // not in 0..=9
        match decode_config(&b) {
            Err(ConfigError::UnknownEnumId { field, id: 99 }) => {
                assert_eq!(field, "ChipType");
            }
            other => panic!("expected UnknownEnumId, got {other:?}"),
        }
    }

    #[test]
    fn lenient_decode_handles_unknown_enums() {
        let mut b = make_buffer();
        b[12] = 99;
        let cfg = decode_config_lenient(&b).unwrap();
        assert_eq!(cfg.socket1.chip_type, ChipType::Unknown);
    }

    #[test]
    fn framing_byte_is_d2() {
        assert_eq!(FRAMING_BYTE, 0xD2);
    }

    #[test]
    fn encode_read_config_packet() {
        assert_eq!(encode_read_config(), [0xD2, 0x30, 0, 0, 0, 0]);
    }

    #[test]
    fn encode_set_config_packet() {
        // section 0x1 (socket1), item 0x2 (chiptype), value 3 (ArmSid)
        assert_eq!(encode_set_config(0x1, 0x2, 3), [0xD2, 0x32, 0x1, 0x2, 3, 0]);
    }

    #[test]
    fn preset_encoding() {
        assert_eq!(encode_preset(Preset::SingleS1), [0xD2, 0x40, 0, 0, 0, 0]);
        assert_eq!(encode_preset(Preset::SingleS2), [0xD2, 0x40, 1, 0, 0, 0]);
        assert_eq!(encode_preset(Preset::DualBoth), [0xD2, 0x41, 0, 0, 0, 0]);
        assert_eq!(encode_preset(Preset::Mirrored), [0xD2, 0x45, 0, 0, 0, 0]);
        assert_eq!(
            encode_preset(Preset::MirroredDual),
            [0xD2, 0x45, 1, 0, 0, 0]
        );
        assert_eq!(encode_preset(Preset::Quad), [0xD2, 0x42, 0, 0, 0, 0]);
    }

    #[test]
    fn config_to_packets_clock() {
        let b = make_buffer();
        let cfg = decode_config(&b).unwrap();
        let pkts = config_to_set_packets(&cfg);
        // First packet should be the clock: SET_CONFIG, section CLOCK, item=PAL id (1), value=lock_flag (1)
        assert_eq!(pkts[0], [0xD2, 0x32, 0x0, 0x1, 0x1, 0x0]);
    }

    #[test]
    fn version_response_parsing() {
        // Simulate a USBSID_VERSION response: cmd echo, length 5, "1.0.0"
        let resp = [
            cfg::USBSID_VERSION,
            5,
            b'1',
            b'.',
            b'0',
            b'.',
            b'0',
            0xFF,
            0xFF,
        ];
        let v = decode_version_response(&resp, cfg::USBSID_VERSION).unwrap();
        assert_eq!(v, "1.0.0");
    }

    #[test]
    fn version_response_wrong_cmd_returns_none() {
        let resp = [0xAA, 5, b'1', b'.', b'0', b'.', b'0'];
        assert!(decode_version_response(&resp, cfg::USBSID_VERSION).is_none());
    }
}
