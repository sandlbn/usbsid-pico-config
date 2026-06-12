//! Integration tests that talk to a real USBSID-Pico over USB.
//!
//! **All tests in this file are gated with `#[ignore]`** so a normal
//! `cargo test` run on a machine without hardware (CI, dev box, etc.)
//! doesn't fail. Run them explicitly:
//!
//! ```text
//! # macOS — libusb requires root to claim a non-CDC USB interface
//! sudo $(which cargo) test --test real_device -- --ignored --test-threads=1 --nocapture
//!
//! # Linux — with the udev rule from README.md, no sudo needed
//! cargo test --test real_device -- --ignored --test-threads=1 --nocapture
//! ```
//!
//! `--test-threads=1` is required: every test opens the device, and only
//! one process can hold the USB interface at a time. `--nocapture` shows
//! the progress prints so you can see which field is being toggled.
//!
//! # Safety
//!
//! Every test snapshots the current `DeviceConfig` at the top, runs its
//! mutation, asserts the change took effect, then writes the snapshot
//! back. **No test ever calls `save_to_flash` or `save_and_reboot`** —
//! changes live only in RAM and are wiped at the next power-cycle even
//! if a test panics mid-run.

use usbsid_pico_config::model::{ClockRate, DeviceConfig, Preset};
use usbsid_pico_config::transport::StandaloneTransport;
use usbsid_pico_config::Device;

// Small delay we give the firmware to actually apply a freshly-written
// config before reading it back. Empirically 50 ms is plenty for plain
// SET_CONFIG triplets.
const SETTLE_MS: u64 = 50;

// Presets reshape multiple fields atomically and can trigger socket-power
// regulator changes; on some firmware revisions that takes noticeably
// longer than a SET_CONFIG. Give them more headroom.
const PRESET_SETTLE_MS: u64 = 500;

fn open() -> Device<StandaloneTransport> {
    let t = StandaloneTransport::open().expect(
        "Cannot open USBSID-Pico. Plug it in, or on macOS run the test binary with sudo. \
         On Linux ensure the udev rule from README.md is installed.",
    );
    Device::new(t)
}

/// Read the current config; sleep briefly so the device returns the freshly-
/// committed state rather than an in-flight write.
fn read_after_apply(dev: &mut Device<StandaloneTransport>) -> DeviceConfig {
    std::thread::sleep(std::time::Duration::from_millis(SETTLE_MS));
    dev.read_config_lenient().expect("read_config after apply")
}

/// Read the current config with extra settle time + a couple of retries —
/// the right thing to call after applying a preset because some presets
/// briefly take the device offline. Returns `None` if the device stays
/// unresponsive (caller can fail loudly or attempt recovery).
fn read_after_preset(dev: &mut Device<StandaloneTransport>) -> Option<DeviceConfig> {
    std::thread::sleep(std::time::Duration::from_millis(PRESET_SETTLE_MS));
    for attempt in 0..3 {
        match dev.read_config_lenient() {
            Ok(c) => return Some(c),
            Err(e) => {
                eprintln!("read_after_preset attempt {attempt} failed: {e}");
                std::thread::sleep(std::time::Duration::from_millis(250));
            }
        }
    }
    None
}

/// Push `cfg` to the device and commit it to RAM. Wraps the two-step
/// write_config + apply pattern that every mutation test uses.
fn write_and_apply(dev: &mut Device<StandaloneTransport>, cfg: &DeviceConfig) {
    dev.write_config(cfg).expect("write_config");
    dev.apply().expect("apply");
}

// ─────────────────────────────────────────────────────────────────────────────
//  ConfigGuard — RAII restore-on-drop
// ─────────────────────────────────────────────────────────────────────────────

/// Holds a snapshot of the device's original config and writes it back when
/// dropped. Critical for mutation tests: if a test panics mid-run (e.g.
/// because the device became unresponsive after a particular preset), the
/// guard's `drop` still fires and at least *tries* to leave the device in
/// the state we found it in.
///
/// Best-effort: if the device is too far gone for write_config to succeed,
/// the guard logs and gives up rather than double-panicking inside drop.
struct ConfigGuard<'a> {
    dev: &'a mut Device<StandaloneTransport>,
    original: DeviceConfig,
}

impl<'a> ConfigGuard<'a> {
    fn snapshot(dev: &'a mut Device<StandaloneTransport>) -> Self {
        let original = dev.read_config_lenient().expect("snapshot read_config");
        Self { dev, original }
    }

    fn original(&self) -> &DeviceConfig {
        &self.original
    }

    fn dev(&mut self) -> &mut Device<StandaloneTransport> {
        self.dev
    }
}

impl<'a> Drop for ConfigGuard<'a> {
    fn drop(&mut self) {
        // Give the device a moment to recover if a flaky preset left it
        // briefly unresponsive, then attempt the restore. Multiple tries
        // because the first attempt right after a hostile preset may fail.
        for attempt in 0..3 {
            if attempt > 0 {
                std::thread::sleep(std::time::Duration::from_millis(250));
            }
            if self.dev.write_config(&self.original).is_err() {
                eprintln!("ConfigGuard::drop: write_config attempt {attempt} failed");
                continue;
            }
            if self.dev.apply().is_err() {
                eprintln!("ConfigGuard::drop: apply attempt {attempt} failed");
                continue;
            }
            return;
        }
        eprintln!(
            "ConfigGuard::drop: could not restore original config. \
             Run `sudo target/debug/examples/usbsid_config reset` or power-cycle the device."
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  Read-only tests
// ─────────────────────────────────────────────────────────────────────────────

#[test]
#[ignore]
fn read_versions_match_clojure_tool() {
    let mut dev = open();
    let fw = dev.read_firmware_version().expect("read_firmware_version");
    let pcb = dev.read_pcb_version().expect("read_pcb_version");
    println!("Firmware: v{fw}");
    println!("PCB:      v{pcb}");
    assert!(!fw.is_empty(), "firmware version should not be empty");
    assert!(!pcb.is_empty(), "PCB version should not be empty");
    // Both strings include a dot per the upstream tool's validators.
    assert!(fw.contains('.'));
    assert!(pcb.contains('.'));
}

#[test]
#[ignore]
fn read_config_decodes_cleanly() {
    let mut dev = open();
    let cfg = dev.read_config().expect(
        "read_config (strict). If this fails with UnknownEnumId, your firmware uses an enum value \
         this crate doesn't know about — switch to read_config_lenient.",
    );
    println!("Clock:    {}", cfg.clock_rate.label());
    println!(
        "Socket 1: enabled={}, chip={}",
        cfg.socket1.enabled,
        cfg.socket1.chip_type.label()
    );
    println!(
        "Socket 2: enabled={}, chip={}",
        cfg.socket2.enabled,
        cfg.socket2.chip_type.label()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
//  Audio-routing flag toggles
// ─────────────────────────────────────────────────────────────────────────────
//
// Each test follows the same pattern:
//   1. Snapshot the current config.
//   2. Flip one boolean field.
//   3. write_config + apply.
//   4. Re-read; assert the flip took effect.
//   5. Write the snapshot back so the device is left in its original state.
//
// The snapshot-and-restore step uses scopeguard-style ordering rather than
// a real RAII guard — keeps the deps list at zero. If a test panics between
// the mutation and the restore, the device stays in the mutated state in
// RAM and will revert on power-cycle.

#[test]
#[ignore]
fn toggle_stereo_enabled() {
    let mut dev = open();
    let mut guard = ConfigGuard::snapshot(&mut dev);
    println!(
        "original stereo_enabled = {}",
        guard.original().stereo_enabled
    );

    let mut mutated = *guard.original();
    mutated.stereo_enabled = !mutated.stereo_enabled;
    write_and_apply(guard.dev(), &mutated);

    let after = read_after_apply(guard.dev());
    assert_eq!(
        after.stereo_enabled, mutated.stereo_enabled,
        "stereo flip didn't stick"
    );
}

#[test]
#[ignore]
fn toggle_lock_audio_switch() {
    let mut dev = open();
    let mut guard = ConfigGuard::snapshot(&mut dev);
    println!(
        "original lock_audio_switch = {}",
        guard.original().lock_audio_switch
    );

    let mut mutated = *guard.original();
    mutated.lock_audio_switch = !mutated.lock_audio_switch;
    write_and_apply(guard.dev(), &mutated);

    let after = read_after_apply(guard.dev());
    assert_eq!(after.lock_audio_switch, mutated.lock_audio_switch);
}

#[test]
#[ignore]
fn toggle_mirrored() {
    let mut dev = open();
    let mut guard = ConfigGuard::snapshot(&mut dev);
    println!("original mirrored = {}", guard.original().mirrored);

    let mut mutated = *guard.original();
    mutated.mirrored = !mutated.mirrored;
    write_and_apply(guard.dev(), &mutated);

    let after = read_after_apply(guard.dev());
    assert_eq!(after.mirrored, mutated.mirrored);
}

#[test]
#[ignore]
fn toggle_flipped() {
    let mut dev = open();
    let mut guard = ConfigGuard::snapshot(&mut dev);
    println!("original flipped = {}", guard.original().flipped);

    let mut mutated = *guard.original();
    mutated.flipped = !mutated.flipped;
    write_and_apply(guard.dev(), &mutated);

    let after = read_after_apply(guard.dev());
    assert_eq!(after.flipped, mutated.flipped);
}

#[test]
#[ignore]
fn toggle_mixed() {
    let mut dev = open();
    let mut guard = ConfigGuard::snapshot(&mut dev);
    println!("original mixed = {}", guard.original().mixed);

    let mut mutated = *guard.original();
    mutated.mixed = !mutated.mixed;
    write_and_apply(guard.dev(), &mutated);

    let after = read_after_apply(guard.dev());
    assert_eq!(after.mixed, mutated.mixed);
}

#[test]
#[ignore]
fn toggle_fmopl_enabled() {
    let mut dev = open();
    let mut guard = ConfigGuard::snapshot(&mut dev);
    println!(
        "original fmopl_enabled = {}",
        guard.original().protocols.fmopl_enabled
    );

    let mut mutated = *guard.original();
    mutated.protocols.fmopl_enabled = !mutated.protocols.fmopl_enabled;
    write_and_apply(guard.dev(), &mutated);

    let after = read_after_apply(guard.dev());
    assert_eq!(
        after.protocols.fmopl_enabled,
        mutated.protocols.fmopl_enabled
    );
}

// ─────────────────────────────────────────────────────────────────────────────
//  Clock rate
// ─────────────────────────────────────────────────────────────────────────────

#[test]
#[ignore]
fn change_clock_rate_round_trip() {
    let mut dev = open();
    let mut guard = ConfigGuard::snapshot(&mut dev);
    println!(
        "original clock_rate = {}",
        guard.original().clock_rate.label()
    );

    // Pick a target that's guaranteed different from whatever's currently set.
    let target = if guard.original().clock_rate == ClockRate::Pal {
        ClockRate::Ntsc
    } else {
        ClockRate::Pal
    };

    let mut mutated = *guard.original();
    mutated.clock_rate = target;
    write_and_apply(guard.dev(), &mutated);

    let after = read_after_apply(guard.dev());
    assert_eq!(after.clock_rate, target, "clock_rate change didn't stick");
}

// ─────────────────────────────────────────────────────────────────────────────
//  Read-only protocol toggles — CDC / WebUSB / ASID / MIDI
// ─────────────────────────────────────────────────────────────────────────────
//
// These four flags ARE returned by READ_CONFIG but the Clojure tool's
// `config->commands` writer doesn't emit them — they look firmware-managed.
// This test just confirms the values come back stably across two reads.

#[test]
#[ignore]
fn protocol_toggles_are_stable_across_reads() {
    let mut dev = open();
    let a = dev.read_config_lenient().expect("read_config");
    let b = dev.read_config_lenient().expect("read_config again");
    assert_eq!(
        a.protocols.cdc, b.protocols.cdc,
        "CDC drifted between reads"
    );
    assert_eq!(a.protocols.webusb, b.protocols.webusb, "WebUSB drifted");
    assert_eq!(a.protocols.asid, b.protocols.asid, "ASID drifted");
    assert_eq!(a.protocols.midi, b.protocols.midi, "MIDI drifted");
    println!(
        "protocols: cdc={} webusb={} asid={} midi={} fmopl={} (sidno={})",
        a.protocols.cdc,
        a.protocols.webusb,
        a.protocols.asid,
        a.protocols.midi,
        a.protocols.fmopl_enabled,
        a.protocols.fmopl_sidno
    );
}

// ─────────────────────────────────────────────────────────────────────────────
//  Preset application
// ─────────────────────────────────────────────────────────────────────────────
//
// Apply a known preset, verify the socket layout changed, restore the
// original config. NB: presets are sent as a single-byte command — they
// bypass the SET_CONFIG triplet path and atomically reshape multiple
// fields. We compare the read-back socket1.enabled / socket2.enabled to
// what the preset should produce.

#[test]
#[ignore]
fn preset_dual_both_enables_both_sockets() {
    let mut dev = open();
    let mut guard = ConfigGuard::snapshot(&mut dev);

    guard
        .dev()
        .apply_preset(Preset::DualBoth)
        .expect("apply_preset");
    let after = read_after_preset(guard.dev())
        .expect("device unresponsive after DualBoth preset (try increasing PRESET_SETTLE_MS)");
    assert!(
        after.socket1.enabled,
        "DualBoth should leave socket1 enabled"
    );
    assert!(
        after.socket2.enabled,
        "DualBoth should leave socket2 enabled"
    );
    // ConfigGuard restores on drop.
}

#[test]
#[ignore]
fn preset_single_s1_disables_socket2() {
    let mut dev = open();
    let mut guard = ConfigGuard::snapshot(&mut dev);

    guard
        .dev()
        .apply_preset(Preset::SingleS1)
        .expect("apply_preset");
    let after = read_after_preset(guard.dev())
        .expect("device unresponsive after SingleS1 preset (try increasing PRESET_SETTLE_MS)");
    assert!(after.socket1.enabled, "SingleS1 must leave socket1 enabled");
    println!(
        "SingleS1 → socket1.enabled={}, socket2.enabled={}",
        after.socket1.enabled, after.socket2.enabled
    );
    // SingleS1's documented behaviour: socket 2 is taken offline. Confirm.
    assert!(!after.socket2.enabled, "SingleS1 should disable socket2");
    // ConfigGuard restores on drop.
}

// ─────────────────────────────────────────────────────────────────────────────
//  Full write-and-restore round trip
// ─────────────────────────────────────────────────────────────────────────────
//
// Sanity check that pushing the exact config we just read back, without
// mutation, produces no detectable change. Catches the case where the
// encoder is dropping or corrupting a field on its way to SET_CONFIG.

#[test]
#[ignore]
fn write_config_is_idempotent() {
    let mut dev = open();
    let mut guard = ConfigGuard::snapshot(&mut dev);
    let before = *guard.original();
    write_and_apply(guard.dev(), &before);
    let after = read_after_apply(guard.dev());
    assert_eq!(
        after, before,
        "idempotent write produced a different config"
    );
}
