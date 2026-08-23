//! INI import/export for [`DeviceConfig`], wire-compatible with the
//! Clojure Configtool's `cfg_usbsid.c`-format output.
//!
//! [`read_ini`] merges over a base config so INI-absent fields (FMOpl
//! `sidno`, v1.5+ flags, CDC/WebUSB/ASID/MIDI toggles) are preserved.

use crate::model::{ChipType, ClockRate, DeviceConfig, SidType};

/// INI chip labels, indexed by [`ChipType::id`]. Do NOT reorder — matches
/// upstream `cfg_usbsid.c`.
pub const CHIPTYPE_LABELS: &[&str] = &[
    "MOS", "Unknown", "SKPico", "ARMSID", "ARM2SID", "FPGASID", "RedipSID", "PDSID", "BackSID",
    "SIDEmu",
];

/// INI SID-type labels, indexed by [`SidType::id`].
pub const SIDTYPE_LABELS: &[&str] = &["Unknown", "N/A", "8580", "6581", "FMOpl"];

fn chip_label(c: ChipType) -> &'static str {
    CHIPTYPE_LABELS
        .get(c.id() as usize)
        .copied()
        .unwrap_or("Unknown")
}

fn sid_label(s: SidType) -> &'static str {
    SIDTYPE_LABELS
        .get(s.id() as usize)
        .copied()
        .unwrap_or("Unknown")
}

fn bool_s(v: bool) -> &'static str {
    if v {
        "True"
    } else {
        "False"
    }
}
fn dual_s(v: bool) -> &'static str {
    if v {
        "Enabled"
    } else {
        "Disabled"
    }
}
fn audio_s(v: bool) -> &'static str {
    if v {
        "Stereo"
    } else {
        "Mono"
    }
}

fn s_bool(s: &str) -> bool {
    s.trim() == "True"
}
fn s_dual(s: &str) -> bool {
    s.trim() == "Enabled"
}
fn s_audio(s: &str) -> bool {
    s.trim() == "Stereo"
}

/// Parse a chip label. Accepts historical `"Real"` (→ MOS) and `"Clone"`
/// (→ Unknown) aliases from the pre-v0.7 INI format.
fn s_chip(raw: &str) -> ChipType {
    let v = raw.trim();
    let normalised: &str = match v {
        "Real" => "MOS",
        "Clone" => "Unknown",
        other => other,
    };
    let id = CHIPTYPE_LABELS
        .iter()
        .position(|&l| l == normalised)
        .unwrap_or(1);
    ChipType::from_id(id as u8).unwrap_or(ChipType::Unknown)
}

fn s_sid(raw: &str) -> SidType {
    let v = raw.trim();
    let normalised: &str = match v {
        "MOS8580" => "8580",
        "MOS6581" => "6581",
        other => other,
    };
    let id = SIDTYPE_LABELS
        .iter()
        .position(|&l| l == normalised)
        .unwrap_or(0);
    SidType::from_id(id as u8).unwrap_or(SidType::Unknown)
}

/// Parse a clock rate given as its literal Hz value; PAL on failure.
fn s_clk(raw: &str) -> ClockRate {
    let v: u32 = raw.trim().parse().unwrap_or(985_248);
    for c in [
        ClockRate::Default,
        ClockRate::Pal,
        ClockRate::Ntsc,
        ClockRate::Drean,
        ClockRate::Ntsc2,
    ] {
        if c.hz() == v {
            return c;
        }
    }
    ClockRate::Pal
}

/// Serialise a `DeviceConfig` to Configtool-compatible INI. `fw_version`
/// is echoed into `[General] version = ...`; empty → sentinel default.
pub fn write_ini(cfg: &DeviceConfig, fw_version: &str) -> String {
    let version = if fw_version.trim().is_empty() {
        "v0.0.0-DEFAULT.19000101".to_string()
    } else {
        fw_version.to_string()
    };
    let s1 = &cfg.socket1;
    let s2 = &cfg.socket2;
    let led = &cfg.led;
    let rgb = &cfg.rgb_led;

    let chip_list = CHIPTYPE_LABELS.join(", ");
    let sid_list = SIDTYPE_LABELS.join(", ");
    let sid_to_use_display = rgb.sid_to_use.max(1);

    format!(
        "[General]\n\
         ; Version is for reference only\n\
         version = {version}\n\
         ; Possible clockrates: 1000000, 985248, 1022727, 1023440, 1022730\n\
         clock_rate = {clock_hz}\n\
         ; Possible options: False, True\n\
         lock_clockrate = {lock_clockrate}\n\
         ; Possible options: False, True\n\
         external_clock = {external_clock}\n\
         \n\
         [socketOne]\n\
         ; Possible options: False, True\n\
         enabled = {s1_enabled}\n\
         ; Possible options: Disabled, Enabled\n\
         dualsid = {s1_dualsid}\n\
         ; Possible chiptypes: {chip_list}\n\
         chiptype = {s1_chip}\n\
         ; Possible sidtypes: {sid_list}\n\
         sid1type = {s1_sid1}\n\
         sid2type = {s1_sid2}\n\
         \n\
         [socketTwo]\n\
         ; Possible options: False, True\n\
         enabled = {s2_enabled}\n\
         ; Possible options: Disabled, Enabled\n\
         dualsid = {s2_dualsid}\n\
         ; Possible chiptypes: {chip_list}\n\
         chiptype = {s2_chip}\n\
         ; Possible sidtypes: {sid_list}\n\
         sid1type = {s2_sid1}\n\
         sid2type = {s2_sid2}\n\
         \n\
         [Features]\n\
         ; Possible options: False, True\n\
         mirrored = {mirrored}\n\
         flipped  = {flipped}\n\
         mixed    = {mixed}\n\
         \n\
         [LED]\n\
         ; Possible options: False, True\n\
         enabled = {led_enabled}\n\
         ; Possible options: False, True\n\
         idle_breathe = {led_breathe}\n\
         \n\
         [RGBLED]\n\
         ; Possible options: False, True\n\
         enabled = {rgb_enabled}\n\
         ; Possible options: False, True\n\
         idle_breathe = {rgb_breathe}\n\
         ; Possible brightness values 0 ~ 255\n\
         brightness = {rgb_brightness}\n\
         ; Possible sids to use are 1, 2, 3 or 4\n\
         sid_to_use = {sid_to_use}\n\
         \n\
         [FMOPL]\n\
         ; Possible options: False, True\n\
         enabled = {fmopl_enabled}\n\
         \n\
         [Audioswitch]\n\
         ; Possible options: Mono, Stereo\n\
         set_to = {audio_set_to}\n\
         ; Possible options: False, True\n\
         lock_audio_switch = {lock_audio}\n\
         \n",
        clock_hz = cfg.clock_rate.hz(),
        lock_clockrate = bool_s(cfg.lock_clockrate),
        external_clock = bool_s(cfg.external_clock),
        s1_enabled = bool_s(s1.enabled),
        s1_dualsid = dual_s(s1.dualsid),
        s1_chip = chip_label(s1.chip_type),
        s1_sid1 = sid_label(s1.sid1.kind),
        s1_sid2 = sid_label(s1.sid2.kind),
        s2_enabled = bool_s(s2.enabled),
        s2_dualsid = dual_s(s2.dualsid),
        s2_chip = chip_label(s2.chip_type),
        s2_sid1 = sid_label(s2.sid1.kind),
        s2_sid2 = sid_label(s2.sid2.kind),
        mirrored = bool_s(cfg.mirrored),
        flipped = bool_s(cfg.flipped),
        mixed = bool_s(cfg.mixed),
        led_enabled = bool_s(led.enabled),
        led_breathe = bool_s(led.idle_breathe),
        rgb_enabled = bool_s(rgb.enabled),
        rgb_breathe = bool_s(rgb.idle_breathe),
        rgb_brightness = rgb.brightness,
        sid_to_use = sid_to_use_display,
        fmopl_enabled = bool_s(cfg.protocols.fmopl_enabled),
        audio_set_to = audio_s(cfg.stereo_enabled),
        lock_audio = bool_s(cfg.lock_audio_switch),
    )
}

/// Parsed INI as `(section, key) -> value`.
struct ParsedIni {
    kv: std::collections::HashMap<(String, String), String>,
}

impl ParsedIni {
    fn parse(ini: &str) -> Self {
        let mut kv = std::collections::HashMap::new();
        let mut section = String::new();
        for raw_line in ini.lines() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
                continue;
            }
            if line.starts_with('[') && line.ends_with(']') {
                section = line[1..line.len() - 1].trim().to_string();
                continue;
            }
            if let Some(eq) = line.find('=') {
                let key = line[..eq].trim().to_string();
                let value = line[eq + 1..].trim().to_string();
                kv.insert((section.clone(), key), value);
            }
        }
        Self { kv }
    }

    fn get(&self, section: &str, key: &str) -> Option<&str> {
        self.kv
            .get(&(section.to_string(), key.to_string()))
            .map(|s| s.as_str())
    }
}

/// Extract `[General] version = ...`, or `None` if absent.
pub fn read_ini_version(ini: &str) -> Option<String> {
    ParsedIni::parse(ini)
        .get("General", "version")
        .map(|s| s.to_string())
}

/// Merge INI over `base`: INI-present fields overwrite, INI-absent fields
/// pass through unchanged. Never fails — unknown labels fold to
/// `Unknown`, unparseable numbers to PAL / brightness 127.
pub fn read_ini(ini: &str, base: &DeviceConfig) -> DeviceConfig {
    let parsed = ParsedIni::parse(ini);
    let mut cfg = *base;

    if let Some(v) = parsed.get("General", "clock_rate") {
        cfg.clock_rate = s_clk(v);
    }
    if let Some(v) = parsed.get("General", "lock_clockrate") {
        cfg.lock_clockrate = s_bool(v);
    }
    if let Some(v) = parsed.get("General", "external_clock") {
        cfg.external_clock = s_bool(v);
    }

    if let Some(v) = parsed.get("socketOne", "enabled") {
        cfg.socket1.enabled = s_bool(v);
    }
    if let Some(v) = parsed.get("socketOne", "dualsid") {
        cfg.socket1.dualsid = s_dual(v);
    }
    if let Some(v) = parsed.get("socketOne", "chiptype") {
        cfg.socket1.chip_type = s_chip(v);
    }
    if let Some(v) = parsed.get("socketOne", "sid1type") {
        cfg.socket1.sid1.kind = s_sid(v);
    }
    if let Some(v) = parsed.get("socketOne", "sid2type") {
        cfg.socket1.sid2.kind = s_sid(v);
    }

    if let Some(v) = parsed.get("socketTwo", "enabled") {
        cfg.socket2.enabled = s_bool(v);
    }
    if let Some(v) = parsed.get("socketTwo", "dualsid") {
        cfg.socket2.dualsid = s_dual(v);
    }
    if let Some(v) = parsed.get("socketTwo", "chiptype") {
        cfg.socket2.chip_type = s_chip(v);
    }
    if let Some(v) = parsed.get("socketTwo", "sid1type") {
        cfg.socket2.sid1.kind = s_sid(v);
    }
    if let Some(v) = parsed.get("socketTwo", "sid2type") {
        cfg.socket2.sid2.kind = s_sid(v);
    }

    if let Some(v) = parsed.get("Features", "mirrored") {
        cfg.mirrored = s_bool(v);
    }
    // Historic compat: earlier tool wrote mirrored under socketTwo/act_as_one.
    if let Some(v) = parsed.get("socketTwo", "act_as_one") {
        cfg.mirrored = s_bool(v);
    }
    if let Some(v) = parsed.get("Features", "flipped") {
        cfg.flipped = s_bool(v);
    }
    if let Some(v) = parsed.get("Features", "mixed") {
        cfg.mixed = s_bool(v);
    }

    if let Some(v) = parsed.get("LED", "enabled") {
        cfg.led.enabled = s_bool(v);
    }
    if let Some(v) = parsed.get("LED", "idle_breathe") {
        cfg.led.idle_breathe = s_bool(v);
    }
    if let Some(v) = parsed.get("RGBLED", "enabled") {
        cfg.rgb_led.enabled = s_bool(v);
    }
    if let Some(v) = parsed.get("RGBLED", "idle_breathe") {
        cfg.rgb_led.idle_breathe = s_bool(v);
    }
    if let Some(v) = parsed.get("RGBLED", "brightness") {
        if let Ok(n) = v.trim().parse::<u32>() {
            cfg.rgb_led.brightness = n.min(255) as u8;
        }
    }
    if let Some(v) = parsed.get("RGBLED", "sid_to_use") {
        if let Ok(n) = v.trim().parse::<i32>() {
            cfg.rgb_led.sid_to_use = if (1..=4).contains(&n) { n as i8 } else { -1 };
        }
    }

    if let Some(v) = parsed.get("FMOPL", "enabled") {
        cfg.protocols.fmopl_enabled = s_bool(v);
    }
    if let Some(v) = parsed.get("Audioswitch", "set_to") {
        cfg.stereo_enabled = s_audio(v);
    }
    if let Some(v) = parsed.get("Audioswitch", "lock_audio_switch") {
        cfg.lock_audio_switch = s_bool(v);
    }

    cfg
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{LedConfig, ProtocolToggles, RgbLedConfig, SidSlot, SocketConfig};

    fn fixture() -> DeviceConfig {
        DeviceConfig {
            need_confirmation: true,
            disable_changedetect: true,
            preset_auto_detect: true,
            last_preset: 7,
            lock_clockrate: true,
            external_clock: false,
            clock_rate: ClockRate::Pal,
            socket1: SocketConfig {
                enabled: true,
                dualsid: false,
                chip_type: ChipType::ArmSid,
                sid1: SidSlot {
                    id: 0,
                    addr: SidSlot::addr_for_id(0),
                    kind: SidType::Mos6581,
                },
                sid2: SidSlot {
                    id: 1,
                    addr: SidSlot::addr_for_id(1),
                    kind: SidType::Na,
                },
            },
            socket2: SocketConfig {
                enabled: true,
                dualsid: true,
                chip_type: ChipType::SkPico,
                sid1: SidSlot {
                    id: 2,
                    addr: SidSlot::addr_for_id(2),
                    kind: SidType::Mos8580,
                },
                sid2: SidSlot {
                    id: 3,
                    addr: SidSlot::addr_for_id(3),
                    kind: SidType::Mos8580,
                },
            },
            led: LedConfig {
                enabled: true,
                idle_breathe: false,
            },
            rgb_led: RgbLedConfig {
                enabled: true,
                idle_breathe: true,
                brightness: 200,
                sid_to_use: 2,
            },
            protocols: ProtocolToggles {
                cdc: true,
                webusb: true,
                asid: false,
                midi: false,
                fmopl_enabled: true,
                fmopl_sidno: 3,
            },
            stereo_enabled: true,
            lock_audio_switch: true,
            mirrored: true,
            flipped: false,
            mixed: true,
        }
    }

    #[test]
    fn write_ini_contains_expected_labels() {
        let ini = write_ini(&fixture(), "1.2.3");
        // Header line
        assert!(ini.contains("version = 1.2.3"), "version line missing");
        // Chip labels use the INI table (MOS/ARMSID/SKPico) not the enum
        // labels (Real SID / ARMSID / SKPico).
        assert!(ini.contains("chiptype = ARMSID"), "socket1 chip");
        assert!(ini.contains("chiptype = SKPico"), "socket2 chip");
        // SID labels use INI form (6581 / 8580 / N/A).
        assert!(ini.contains("sid1type = 6581"), "socket1 sid1");
        assert!(ini.contains("sid2type = N/A"), "socket1 sid2");
        // Enabled/Disabled for dualsid; True/False for booleans.
        assert!(ini.contains("dualsid = Enabled"), "socket2 dualsid");
        assert!(ini.contains("dualsid = Disabled"), "socket1 dualsid");
        assert!(ini.contains("mirrored = True"));
        // Audioswitch = Stereo (not True).
        assert!(ini.contains("set_to = Stereo"));
        // Empty version substitutes the DEFAULT sentinel.
        let ini2 = write_ini(&fixture(), "");
        assert!(ini2.contains("version = v0.0.0-DEFAULT."));
    }

    #[test]
    fn ini_roundtrip_preserves_readable_fields() {
        let orig = fixture();
        let ini = write_ini(&orig, "1.2.3");
        let parsed = read_ini(&ini, &orig);
        // Fields the INI covers must round-trip byte-for-byte.
        assert_eq!(parsed.clock_rate, orig.clock_rate);
        assert_eq!(parsed.lock_clockrate, orig.lock_clockrate);
        assert_eq!(parsed.external_clock, orig.external_clock);
        assert_eq!(parsed.socket1.chip_type, orig.socket1.chip_type);
        assert_eq!(parsed.socket1.sid1.kind, orig.socket1.sid1.kind);
        assert_eq!(parsed.socket2.dualsid, orig.socket2.dualsid);
        assert_eq!(parsed.mirrored, orig.mirrored);
        assert_eq!(parsed.flipped, orig.flipped);
        assert_eq!(parsed.mixed, orig.mixed);
        assert_eq!(parsed.led, orig.led);
        assert_eq!(parsed.rgb_led, orig.rgb_led);
        assert_eq!(parsed.protocols.fmopl_enabled, orig.protocols.fmopl_enabled);
        assert_eq!(parsed.stereo_enabled, orig.stereo_enabled);
        assert_eq!(parsed.lock_audio_switch, orig.lock_audio_switch);
    }

    #[test]
    fn ini_does_not_touch_uncovered_v15_fields() {
        // Preserve v1.5+ fields and read-only protocol toggles that don't
        // appear in the INI.
        let orig = fixture();
        let ini = write_ini(&orig, "");
        let mut base = fixture();
        base.need_confirmation = false; // start with different values
        base.disable_changedetect = false;
        base.preset_auto_detect = false;
        base.last_preset = 0;
        base.protocols.fmopl_sidno = 99;
        let out = read_ini(&ini, &base);
        assert_eq!(out.need_confirmation, false, "must preserve base");
        assert_eq!(out.disable_changedetect, false);
        assert_eq!(out.preset_auto_detect, false);
        assert_eq!(out.last_preset, 0);
        assert_eq!(out.protocols.fmopl_sidno, 99);
    }

    #[test]
    fn read_ini_alias_real_and_mos_sid_labels() {
        let ini = "[socketOne]\n\
                   chiptype = Real\n\
                   sid1type = MOS8580\n\
                   sid2type = MOS6581\n";
        let base = fixture();
        let out = read_ini(ini, &base);
        assert_eq!(out.socket1.chip_type, ChipType::Real);
        assert_eq!(out.socket1.sid1.kind, SidType::Mos8580);
        assert_eq!(out.socket1.sid2.kind, SidType::Mos6581);
    }

    #[test]
    fn read_ini_version_extracts_or_returns_none() {
        assert_eq!(
            read_ini_version("[General]\nversion = 1.2.3\n"),
            Some("1.2.3".to_string())
        );
        assert_eq!(read_ini_version("[General]\n"), None);
        assert_eq!(read_ini_version(""), None);
    }

    #[test]
    fn read_ini_socket_two_act_as_one_compat() {
        // Old writer put mirrored under socketTwo/act_as_one.
        let ini = "[socketTwo]\nact_as_one = True\n";
        let mut base = fixture();
        base.mirrored = false;
        let out = read_ini(ini, &base);
        assert!(out.mirrored);
    }
}
