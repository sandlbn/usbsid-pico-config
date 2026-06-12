//! Stand-alone CLI for poking a USBSID-Pico's configuration. Useful for
//! validating the crate against real hardware without involving Phosphor.
//!
//! Run with:
//!
//! ```text
//! cargo run --example usbsid_config -- info       # read fw + pcb + config
//! cargo run --example usbsid_config -- read       # dump full config
//! cargo run --example usbsid_config -- clock pal
//! cargo run --example usbsid_config -- preset dual-both
//! cargo run --example usbsid_config -- apply      # commit pending writes
//! cargo run --example usbsid_config -- save       # write to flash (no reset)
//! cargo run --example usbsid_config -- save-reboot
//! cargo run --example usbsid_config -- reset      # restore defaults
//! cargo run --example usbsid_config -- reload     # reload from flash
//! cargo run --example usbsid_config -- confirm    # CONFIG_ACK (v1.5+ boards)
//! cargo run --example usbsid_config -- list-presets
//! cargo run --example usbsid_config -- list-clocks
//! ```
//!
//! Requires the `rusb-transport` feature (enabled by default). On Linux
//! you may need a udev rule to grant your user access to the USBSID-Pico:
//!
//! ```text
//! SUBSYSTEM=="usb", ATTR{idVendor}=="cafe", ATTR{idProduct}=="4011", MODE="0660", TAG+="uaccess"
//! ```
//!
//! Drop that in `/etc/udev/rules.d/99-usbsid-pico.rules` and `sudo udevadm
//! control --reload-rules && sudo udevadm trigger`.

use std::env;
use std::process::ExitCode;

use usbsid_pico_config::model::{ChipType, ClockRate, Preset, SidType};
use usbsid_pico_config::transport::{StandaloneTransport, Transport};
use usbsid_pico_config::Device;

fn print_usage() {
    let bin = "usbsid_config";
    eprintln!("Usage: {bin} <command> [args...]");
    eprintln!();
    eprintln!("Commands:");
    eprintln!("  info             Read firmware version + PCB version + full config");
    eprintln!("  read             Print full config in a human-readable form");
    eprintln!("  clock <rate>     Set clock rate. <rate> = default | pal | ntsc | drean | ntsc2");
    eprintln!(
        "  preset <name>    Apply a preset. <name> = single-s1, dual-both, mirrored, quad, etc."
    );
    eprintln!("                   (Use `list-presets` to see all options.)");
    eprintln!("  apply            Commit pending writes to RAM");
    eprintln!("  save             Save to flash, keep running");
    eprintln!("  save-reboot      Save to flash and reboot the device");
    eprintln!("  reset            Restore factory defaults");
    eprintln!("  reload           Reload config from flash");
    eprintln!("  confirm          Send CONFIG_ACK (v1.5+ board first-boot handshake)");
    eprintln!("  list-presets     Print all preset names");
    eprintln!("  list-clocks      Print all clock-rate names");
    eprintln!();
    eprintln!("Hardware detection (blocks while firmware probes; ~2-3 s each):");
    eprintln!(
        "  detect-sids      Identify 6581 vs 8580 in each socket (overwrites manual sid types)"
    );
    eprintln!("  detect-clones    Identify clone chips (ARMSID, SKPico, FPGASID, …)");
    eprintln!("  auto-detect      Both of the above, atomic");
    eprintln!();
    eprintln!("Diagnostics:");
    eprintln!("  raw-config       Dump the 64-byte READ_CONFIG response as hex");
    eprintln!("  raw-socket       Dump the READ_SOCKETCFG response as hex");
    eprintln!();
    eprintln!("Socket fine-tuning (writes single SET_CONFIG fields):");
    eprintln!("  socket <N> enable [true|false]              Enable / disable socket N (1 or 2)");
    eprintln!("  socket <N> dualsid [true|false]             Toggle dual-SID mode");
    eprintln!("  socket <N> chip <type>                      Set chip type (real|unknown|skpico|");
    eprintln!("                                              armsid|arm2sid|fpgasid|redipsid|");
    eprintln!("                                              pdsid|backsid|sidemu)");
    eprintln!("  socket <N> sid1 <type>                      Set SID 1 model");
    eprintln!("  socket <N> sid2 <type>                      Set SID 2 model");
    eprintln!(
        "                                              <type>: unknown|na|mos8580|mos6581|fmopl"
    );
    eprintln!("  socket <N> show                             Show current socket N state");
    eprintln!();
    eprintln!("Audio routing toggles:");
    eprintln!("  audio stereo [true|false]                   Set stereo-enabled flag");
    eprintln!("  audio mirrored [true|false]");
    eprintln!("  audio flipped [true|false]");
    eprintln!("  audio mixed [true|false]");
    eprintln!("  audio lock [true|false]                     lock_audio_switch");
    eprintln!();
    eprintln!("All `socket` / `audio` / `clock` writes update RAM. Run `save` to persist.");
}

fn parse_preset(s: &str) -> Option<Preset> {
    Some(match s.to_ascii_lowercase().as_str() {
        "single-s1" => Preset::SingleS1,
        "single-s2" => Preset::SingleS2,
        "dual-both" => Preset::DualBoth,
        "dual-s1" => Preset::DualS1,
        "dual-s2" => Preset::DualS2,
        "triple-s1" => Preset::TripleS1,
        "triple-s2" => Preset::TripleS2,
        "quad" => Preset::Quad,
        "mirrored" => Preset::Mirrored,
        "mirrored-dual" => Preset::MirroredDual,
        "dual-flipped" => Preset::DualFlipped,
        "quad-flipped" => Preset::QuadFlipped,
        "quad-mixed" => Preset::QuadMixed,
        "quad-flipmix" | "quad-flip-mix" => Preset::QuadFlipMixed,
        _ => return None,
    })
}

fn all_presets() -> &'static [(&'static str, Preset)] {
    &[
        ("single-s1", Preset::SingleS1),
        ("single-s2", Preset::SingleS2),
        ("dual-both", Preset::DualBoth),
        ("dual-s1", Preset::DualS1),
        ("dual-s2", Preset::DualS2),
        ("triple-s1", Preset::TripleS1),
        ("triple-s2", Preset::TripleS2),
        ("quad", Preset::Quad),
        ("mirrored", Preset::Mirrored),
        ("mirrored-dual", Preset::MirroredDual),
        ("dual-flipped", Preset::DualFlipped),
        ("quad-flipped", Preset::QuadFlipped),
        ("quad-mixed", Preset::QuadMixed),
        ("quad-flipmix", Preset::QuadFlipMixed),
    ]
}

fn parse_chip(s: &str) -> Option<ChipType> {
    Some(match s.to_ascii_lowercase().as_str() {
        "real" | "real-sid" | "real_sid" => ChipType::Real,
        "unknown" => ChipType::Unknown,
        "skpico" => ChipType::SkPico,
        "armsid" => ChipType::ArmSid,
        "arm2sid" => ChipType::Arm2Sid,
        "fpgasid" => ChipType::FpgaSid,
        "redipsid" => ChipType::RedipSid,
        "pdsid" => ChipType::PdSid,
        "backsid" => ChipType::BackSid,
        "sidemu" => ChipType::SidEmu,
        _ => return None,
    })
}

fn parse_sid_type(s: &str) -> Option<SidType> {
    Some(match s.to_ascii_lowercase().as_str() {
        "unknown" => SidType::Unknown,
        "na" | "n/a" | "none" => SidType::Na,
        "mos8580" | "8580" => SidType::Mos8580,
        "mos6581" | "6581" => SidType::Mos6581,
        "fmopl" => SidType::FmOpl,
        _ => return None,
    })
}

fn parse_bool(s: &str) -> Option<bool> {
    Some(match s.to_ascii_lowercase().as_str() {
        "true" | "1" | "on" | "yes" | "enable" | "enabled" => true,
        "false" | "0" | "off" | "no" | "disable" | "disabled" => false,
        _ => return None,
    })
}

fn parse_clock(s: &str) -> Option<ClockRate> {
    Some(match s.to_ascii_lowercase().as_str() {
        "default" => ClockRate::Default,
        "pal" => ClockRate::Pal,
        "ntsc" => ClockRate::Ntsc,
        "drean" => ClockRate::Drean,
        "ntsc2" => ClockRate::Ntsc2,
        _ => return None,
    })
}

fn all_clocks() -> &'static [ClockRate] {
    &[
        ClockRate::Default,
        ClockRate::Pal,
        ClockRate::Ntsc,
        ClockRate::Drean,
        ClockRate::Ntsc2,
    ]
}

fn open() -> Result<Device<StandaloneTransport>, String> {
    let t = StandaloneTransport::open().map_err(|e| format!("Cannot open USBSID-Pico: {e}"))?;
    Ok(Device::new(t))
}

/// Render a single "Label : Value" row, padded to 16 chars on the label
/// side so columns line up regardless of label length.
fn row(label: &str, value: &str) {
    println!("  {label:<18} {value}");
}

fn fmt_enabled(b: bool) -> &'static str {
    if b {
        "Enabled"
    } else {
        "Disabled"
    }
}

fn fmt_yes_no(b: bool) -> &'static str {
    if b {
        "True"
    } else {
        "False"
    }
}

fn print_config(dev: &mut Device<StandaloneTransport>) -> Result<(), String> {
    let cfg = dev
        .read_config_lenient()
        .map_err(|e| format!("read_config: {e}"))?;

    // ── General ─────────────────────────────────────────────────────────
    println!();
    println!("General:");
    row(
        "SID Clockrate",
        &format!("{} Hz ({})", cfg.clock_rate.hz(), cfg.clock_rate.label()),
    );
    row("Clockrate locked", fmt_yes_no(cfg.lock_clockrate));
    row("External clock", fmt_yes_no(cfg.external_clock));
    row("Onboard LED", fmt_enabled(cfg.led.enabled));
    row("Onboard LED Breathing", fmt_enabled(cfg.led.idle_breathe));
    row("RGB LED", fmt_enabled(cfg.rgb_led.enabled));
    row("RGB LED Breathing", fmt_enabled(cfg.rgb_led.idle_breathe));
    row("RGB LED Brightness", &cfg.rgb_led.brightness.to_string());
    row(
        "RGB LED SID to use",
        &(if cfg.rgb_led.sid_to_use < 0 {
            "Off".to_string()
        } else {
            cfg.rgb_led.sid_to_use.to_string()
        }),
    );

    // ── Sockets ─────────────────────────────────────────────────────────
    for (name, sock) in [("Socket One", &cfg.socket1), ("Socket Two", &cfg.socket2)] {
        println!();
        println!("{name}:");
        row("Enabled", fmt_yes_no(sock.enabled));
        row("Chip type", sock.chip_type.label());
        row(
            "Clone type",
            if sock.chip_type == usbsid_pico_config::ChipType::Real {
                "None"
            } else {
                sock.chip_type.label()
            },
        );
        row("Dual SID", fmt_enabled(sock.dualsid));
        row("SID One", sock.sid1.kind.label());
        row("SID Two", sock.sid2.kind.label());
    }

    // ── Audio routing flags ─────────────────────────────────────────────
    println!();
    println!("Audio routing:");
    row("Lock audio sw", fmt_enabled(cfg.lock_audio_switch));
    row("Mirrored", fmt_enabled(cfg.mirrored));
    row("Flipped", fmt_enabled(cfg.flipped));
    row("Mixed", fmt_enabled(cfg.mixed));

    // ── Protocols (matches the web tool's column) ───────────────────────
    println!();
    println!("Protocols:");
    row("CDC", fmt_enabled(cfg.protocols.cdc));
    row("WebUSB", fmt_enabled(cfg.protocols.webusb));
    row("ASID", fmt_enabled(cfg.protocols.asid));
    row("MIDI", fmt_enabled(cfg.protocols.midi));
    row("FMOpl", fmt_enabled(cfg.protocols.fmopl_enabled));
    row("FMOpl SID", &cfg.protocols.fmopl_sidno.to_string());
    row("Audio", if cfg.stereo_enabled { "Stereo" } else { "Mono" });

    println!();
    if cfg.need_confirmation {
        println!("⚠️  This board needs CONFIG_ACK. Run: usbsid_config confirm");
    }
    if cfg.disable_changedetect {
        println!("⚠️  Socket change-detection has been disabled in firmware.");
    }
    Ok(())
}

fn run() -> Result<(), String> {
    let mut args = env::args().skip(1);
    let cmd = match args.next() {
        Some(c) => c,
        None => {
            print_usage();
            return Err("no command given".into());
        }
    };

    match cmd.as_str() {
        "list-presets" => {
            for (name, p) in all_presets() {
                println!("{name:14} {}", p.label());
            }
            Ok(())
        }
        "list-clocks" => {
            for c in all_clocks() {
                println!(
                    "{:8} {} ({} Hz)",
                    format!("{c:?}").to_lowercase(),
                    c.label(),
                    c.hz()
                );
            }
            Ok(())
        }
        "info" => {
            let mut dev = open()?;
            let fw = dev
                .read_firmware_version()
                .map_err(|e| format!("fw: {e}"))?;
            let pcb = dev.read_pcb_version().map_err(|e| format!("pcb: {e}"))?;
            println!("Firmware:          v{fw}");
            println!("PCB:               v{pcb}");
            println!();
            print_config(&mut dev)
        }
        "read" => print_config(&mut open()?),
        "clock" => {
            let arg = args.next().ok_or("clock: missing rate argument")?;
            let rate = parse_clock(&arg).ok_or_else(|| {
                format!("clock: unknown rate '{arg}'. Try one of: default, pal, ntsc, drean, ntsc2")
            })?;
            let mut dev = open()?;
            // Set via SET_CONFIG section CLOCK (we re-use the existing field-write
            // path by reading current config, mutating clock_rate, and writing back).
            let mut cfg = dev
                .read_config_lenient()
                .map_err(|e| format!("read: {e}"))?;
            cfg.clock_rate = rate;
            dev.write_config(&cfg).map_err(|e| format!("write: {e}"))?;
            dev.apply().map_err(|e| format!("apply: {e}"))?;
            println!(
                "Clock set to {} ({} Hz) and applied (in RAM only — run `save` to persist).",
                rate.label(),
                rate.hz()
            );
            Ok(())
        }
        "preset" => {
            let arg = args.next().ok_or("preset: missing name argument")?;
            let p = parse_preset(&arg)
                .ok_or_else(|| format!("preset: unknown name '{arg}'. Try `list-presets`."))?;
            let mut dev = open()?;
            dev.apply_preset(p)
                .map_err(|e| format!("apply_preset: {e}"))?;
            println!("Applied preset: {}", p.label());
            Ok(())
        }
        "apply" => {
            open()?.apply().map_err(|e| format!("apply: {e}"))?;
            println!("Applied pending writes (RAM only).");
            Ok(())
        }
        "save" => {
            open()?.save_no_reset().map_err(|e| format!("save: {e}"))?;
            println!("Configuration saved to flash (device still running).");
            Ok(())
        }
        "save-reboot" => {
            open()?
                .save_and_reboot()
                .map_err(|e| format!("save-reboot: {e}"))?;
            println!("Configuration saved to flash; device rebooting. Reconnect when ready.");
            Ok(())
        }
        "reset" => {
            open()?
                .reset_to_defaults()
                .map_err(|e| format!("reset: {e}"))?;
            println!("Configuration reset to factory defaults.");
            Ok(())
        }
        "reload" => {
            open()?
                .reload_from_flash()
                .map_err(|e| format!("reload: {e}"))?;
            println!("Configuration reloaded from flash.");
            Ok(())
        }
        "confirm" => {
            open()?
                .confirm_config()
                .map_err(|e| format!("confirm: {e}"))?;
            println!("CONFIG_ACK sent. Socket power should now be enabled (v1.5+ boards).");
            Ok(())
        }
        "socket" => {
            let socket_arg = args
                .next()
                .ok_or("socket: missing socket number (1 or 2)")?;
            let socket_num = socket_arg
                .parse::<u8>()
                .map_err(|_| format!("socket: '{socket_arg}' is not 1 or 2"))?;
            if socket_num != 1 && socket_num != 2 {
                return Err(format!("socket: must be 1 or 2 (got {socket_num})"));
            }
            let field = args
                .next()
                .ok_or("socket: missing field (enable, dualsid, chip, sid1, sid2, show)")?;
            run_socket(socket_num, &field, args.next())
        }
        "audio" => {
            let field = args
                .next()
                .ok_or("audio: missing field (stereo, mirrored, flipped, mixed, lock)")?;
            run_audio(&field, args.next())
        }
        "detect-sids" => {
            let mut dev = open()?;
            println!("Running SID detection (this takes ~3 seconds)…");
            dev.detect_sids().map_err(|e| format!("detect-sids: {e}"))?;
            std::thread::sleep(std::time::Duration::from_millis(3200));
            print_config(&mut dev)
        }
        "detect-clones" => {
            let mut dev = open()?;
            println!("Running clone detection (this takes ~2 seconds)…");
            dev.detect_clones()
                .map_err(|e| format!("detect-clones: {e}"))?;
            std::thread::sleep(std::time::Duration::from_millis(2200));
            print_config(&mut dev)
        }
        "raw-socket" => {
            use usbsid_pico_config::protocol;
            let mut dev = open()?;
            dev.transport_mut()
                .drain()
                .map_err(|e| format!("drain: {e}"))?;
            dev.transport_mut()
                .send(&protocol::encode_packet(
                    protocol::cfg::READ_SOCKETCFG,
                    [0, 0, 0, 0],
                ))
                .map_err(|e| format!("send: {e}"))?;
            let buf = dev
                .transport_mut()
                .recv(64)
                .map_err(|e| format!("recv: {e}"))?;
            println!("READ_SOCKETCFG response ({} bytes):", buf.len());
            for chunk_start in (0..buf.len()).step_by(16) {
                let end = (chunk_start + 16).min(buf.len());
                let hex: Vec<String> = buf[chunk_start..end]
                    .iter()
                    .map(|b| format!("{b:02X}"))
                    .collect();
                println!("  {chunk_start:02X}: {}", hex.join(" "));
            }
            Ok(())
        }
        "raw-config" => {
            let mut dev = open()?;
            // Reach under the Device API to send READ_CONFIG and grab the
            // raw 64-byte response. We use the underlying Transport since
            // Device only exposes the parsed view.
            use usbsid_pico_config::protocol;
            dev.transport_mut()
                .drain()
                .map_err(|e| format!("drain: {e}"))?;
            dev.transport_mut()
                .send(&protocol::encode_read_config())
                .map_err(|e| format!("send: {e}"))?;
            let buf = dev
                .transport_mut()
                .recv(protocol::CONFIG_BUFFER_LEN)
                .map_err(|e| format!("recv: {e}"))?;
            print_raw_buffer(&buf);
            Ok(())
        }
        "auto-detect" => {
            let mut dev = open()?;
            println!("Running auto-detect (this takes ~3 seconds)…");
            dev.auto_detect().map_err(|e| format!("auto-detect: {e}"))?;
            std::thread::sleep(std::time::Duration::from_millis(3500));
            print_config(&mut dev)
        }
        "-h" | "--help" | "help" => {
            print_usage();
            Ok(())
        }
        other => {
            print_usage();
            Err(format!("unknown command '{other}'"))
        }
    }
}

fn run_socket(num: u8, field: &str, value: Option<String>) -> Result<(), String> {
    let mut dev = open()?;
    let mut cfg = dev
        .read_config_lenient()
        .map_err(|e| format!("read_config: {e}"))?;

    // Borrow the right socket once; same pattern for both 1 and 2.
    {
        let sock = if num == 1 {
            &mut cfg.socket1
        } else {
            &mut cfg.socket2
        };
        match field.to_ascii_lowercase().as_str() {
            "show" => {
                println!("Socket {num}:");
                println!("  enabled:   {}", sock.enabled);
                println!("  dualsid:   {}", sock.dualsid);
                println!("  chip type: {}", sock.chip_type.label());
                println!(
                    "  SID 1:     id={} addr=${:02X} {}",
                    sock.sid1.id,
                    sock.sid1.addr,
                    sock.sid1.kind.label()
                );
                println!(
                    "  SID 2:     id={} addr=${:02X} {}",
                    sock.sid2.id,
                    sock.sid2.addr,
                    sock.sid2.kind.label()
                );
                return Ok(());
            }
            "enable" | "enabled" => {
                let v = value
                    .as_deref()
                    .and_then(parse_bool)
                    .ok_or("socket: missing or invalid bool value (true/false)")?;
                sock.enabled = v;
            }
            "dualsid" => {
                let v = value
                    .as_deref()
                    .and_then(parse_bool)
                    .ok_or("socket: missing or invalid bool value (true/false)")?;
                sock.dualsid = v;
            }
            "chip" | "chip_type" | "chiptype" => {
                let s = value.ok_or("socket: missing chip type argument")?;
                sock.chip_type = parse_chip(&s).ok_or_else(|| format!(
                    "socket: unknown chip type '{s}'. Try real, unknown, skpico, armsid, arm2sid, fpgasid, redipsid, pdsid, backsid, sidemu"
                ))?;
            }
            "sid1" => {
                let s = value.ok_or("socket: missing SID 1 type argument")?;
                sock.sid1.kind = parse_sid_type(&s).ok_or_else(|| {
                    format!(
                        "socket: unknown SID type '{s}'. Try unknown, na, mos8580, mos6581, fmopl"
                    )
                })?;
            }
            "sid2" => {
                let s = value.ok_or("socket: missing SID 2 type argument")?;
                sock.sid2.kind = parse_sid_type(&s).ok_or_else(|| {
                    format!(
                        "socket: unknown SID type '{s}'. Try unknown, na, mos8580, mos6581, fmopl"
                    )
                })?;
            }
            other => {
                return Err(format!(
                    "socket: unknown field '{other}'. Try enable, dualsid, chip, sid1, sid2, show."
                ))
            }
        }
    }

    dev.write_config(&cfg)
        .map_err(|e| format!("write_config: {e}"))?;
    dev.apply().map_err(|e| format!("apply: {e}"))?;
    println!("Socket {num} updated (RAM only — run `save` to persist).");
    Ok(())
}

fn run_audio(field: &str, value: Option<String>) -> Result<(), String> {
    let mut dev = open()?;
    let mut cfg = dev
        .read_config_lenient()
        .map_err(|e| format!("read_config: {e}"))?;

    let v = value
        .as_deref()
        .and_then(parse_bool)
        .ok_or("audio: missing or invalid bool value (true/false)")?;

    match field.to_ascii_lowercase().as_str() {
        "stereo" | "stereo_enabled" | "stereo-enabled" => cfg.stereo_enabled = v,
        "mirrored" => cfg.mirrored = v,
        "flipped" => cfg.flipped = v,
        "mixed" => cfg.mixed = v,
        "lock" | "lock_audio" | "lock-audio" | "lock_audio_switch" => {
            cfg.lock_audio_switch = v;
        }
        other => {
            return Err(format!(
                "audio: unknown field '{other}'. Try stereo, mirrored, flipped, mixed, lock."
            ))
        }
    }

    dev.write_config(&cfg)
        .map_err(|e| format!("write_config: {e}"))?;
    dev.apply().map_err(|e| format!("apply: {e}"))?;
    println!("Audio.{field} = {v} (RAM only — run `save` to persist).");
    Ok(())
}

/// Pretty-print a 64-byte READ_CONFIG buffer as hex with byte-offset
/// annotations matching what we know about the layout.
fn print_raw_buffer(buf: &[u8]) {
    println!("Raw READ_CONFIG buffer ({} bytes):", buf.len());
    for chunk_start in (0..buf.len()).step_by(16) {
        let end = (chunk_start + 16).min(buf.len());
        let chunk = &buf[chunk_start..end];
        let hex: Vec<String> = chunk.iter().map(|b| format!("{b:02X}")).collect();
        println!("  {chunk_start:02X}: {}", hex.join(" "));
    }
    println!();
    println!("Known offsets (from Clojure tool):");
    println!("   0    initiator (must be 0x30)");
    println!("   1    verification marker (0x7F)");
    println!("   2-3  need_confirmation, disable_changedetect");
    println!("   5-9  lock_clockrate, external_clock, clock value (3 bytes)");
    println!("  10-15 socket 1: enabled, dualsid, chip, id-pack, sid1_type, sid2_type");
    println!(
        "  20-26 socket 2: enabled, dualsid, [reserved=22], chip, id-pack, sid1_type, sid2_type"
    );
    println!("  30-31 LED");
    println!("  40-43 RGB LED");
    println!("  51-56 CDC, WebUSB, ASID, MIDI, FMOpl, FMOpl SID");
    println!("  57-58 stereo_enabled, lock_audio_switch");
    println!("  60    bit0=mirrored, bit1=flipped, bit2=mixed");
    println!("  63    terminator (0xFF)");
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
