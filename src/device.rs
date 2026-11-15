//! High-level USBSID-Pico configuration facade.
//!
//! Wraps a [`Transport`] with operation-shaped methods (`read_config`,
//! `apply`, `save_to_flash`, `apply_preset`, …) that callers can use
//! without thinking about packet bytes.

use crate::model::{fw_flavor_from_version, DeviceConfig, FwFlavor, Preset};
use crate::protocol::{self, cfg, ConfigError, CONFIG_BUFFER_LEN};
use crate::transport::{Transport, TransportError};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DeviceError {
    #[error("transport error: {0}")]
    Transport(#[from] TransportError),
    #[error("protocol error: {0}")]
    Protocol(#[from] ConfigError),
    #[error("response too short or missing for {what}")]
    EmptyResponse { what: &'static str },
    #[error("device reported an unrecognised response for {what}: {detail}")]
    UnexpectedResponse { what: &'static str, detail: String },
}

/// High-level USBSID-Pico configuration facade.
///
/// Generic over any [`Transport`] so the same API works with the bundled
/// rusb-backed `StandaloneTransport`, a test [`crate::transport::MockTransport`],
/// or a host-side daemon bridge.
pub struct Device<T: Transport> {
    transport: T,
    fw_flavor: FwFlavor,
}

impl<T: Transport> Device<T> {
    /// Wrap a transport. Firmware flavor defaults to [`FwFlavor::V07`];
    /// call [`Self::detect_fw_flavor`] or [`Self::set_fw_flavor`] before
    /// config I/O if the target may be legacy fw.
    pub fn new(transport: T) -> Self {
        Self {
            transport,
            fw_flavor: FwFlavor::V07,
        }
    }

    pub fn transport(&self) -> &T {
        &self.transport
    }

    pub fn transport_mut(&mut self) -> &mut T {
        &mut self.transport
    }

    pub fn fw_flavor(&self) -> FwFlavor {
        self.fw_flavor
    }

    pub fn set_fw_flavor(&mut self, flavor: FwFlavor) {
        self.fw_flavor = flavor;
    }

    /// Read the firmware version and update [`Self::fw_flavor`] to match.
    pub fn detect_fw_flavor(&mut self) -> Result<String, DeviceError> {
        let ver = self.read_firmware_version()?;
        self.fw_flavor = fw_flavor_from_version(&ver);
        Ok(ver)
    }

    /// READ_CONFIG → decoded [`DeviceConfig`]. Retries 3× with 50 ms
    /// delay; each attempt also falls back to the legacy 256-byte drain.
    pub fn read_config(&mut self) -> Result<DeviceConfig, DeviceError> {
        let buf = self.read_config_frame_with_retry()?;
        Ok(protocol::decode_config_with(self.fw_flavor, &buf)?)
    }

    /// [`Self::read_config`] with unknown enum ids folded to `Unknown`
    /// (v0.7 flavor only — legacy decode is always strict).
    pub fn read_config_lenient(&mut self) -> Result<DeviceConfig, DeviceError> {
        let buf = self.read_config_frame_with_retry()?;
        Ok(protocol::decode_config_lenient_with(self.fw_flavor, &buf)?)
    }

    fn read_config_frame_with_retry(&mut self) -> Result<Vec<u8>, DeviceError> {
        let mut last_detail = String::new();
        for attempt in 0..3 {
            if attempt > 0 {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            self.transport.drain()?;
            self.transport.send(&protocol::encode_read_config())?;

            // 64B read; on error or bad framing fall through to 256B drain.
            if let Ok(buf) = self.transport.recv(CONFIG_BUFFER_LEN) {
                if let Some(frame) = protocol::try_extract_config_frame(&buf) {
                    return Ok(frame.to_vec());
                }
            }

            match self.transport.recv(CONFIG_BUFFER_LEN * 4) {
                Ok(buf) => {
                    if let Some(frame) = protocol::try_extract_config_frame(&buf) {
                        return Ok(frame.to_vec());
                    }
                    last_detail = format!("no valid frame found in {}-byte drain", buf.len());
                }
                Err(e) => {
                    last_detail = format!("recv 256B drain failed: {e}");
                }
            }
        }
        Err(DeviceError::Transport(TransportError::Io(format!(
            "read_config: gave up after 3 attempts ({last_detail})"
        ))))
    }

    /// Push a full [`DeviceConfig`] to the device (does NOT save to flash).
    /// Emits one SET_CONFIG packet per field — slow-ish (~20 packets), so
    /// callers shouldn't spam this in a tight loop.
    pub fn write_config(&mut self, cfg: &DeviceConfig) -> Result<(), DeviceError> {
        self.transport.drain()?;
        for packet in protocol::config_to_set_packets_with(self.fw_flavor, cfg) {
            self.transport.send(&packet)?;
        }
        Ok(())
    }

    /// Apply pending writes to RAM (not flash).
    pub fn apply(&mut self) -> Result<(), DeviceError> {
        self.transport.drain()?;
        self.transport.send(&protocol::encode_apply())?;
        Ok(())
    }

    /// Save to flash and reboot the device. The device will disconnect
    /// briefly — callers should expect the transport to time out on the
    /// next call until the device re-enumerates.
    pub fn save_and_reboot(&mut self) -> Result<(), DeviceError> {
        self.transport.drain()?;
        self.transport.send(&protocol::encode_save_and_reboot())?;
        Ok(())
    }

    /// Save to flash without rebooting. Preferred when continuing playback.
    pub fn save_no_reset(&mut self) -> Result<(), DeviceError> {
        self.transport.drain()?;
        self.transport.send(&protocol::encode_save_no_reset())?;
        Ok(())
    }

    /// Restore factory defaults.
    pub fn reset_to_defaults(&mut self) -> Result<(), DeviceError> {
        self.transport.drain()?;
        self.transport.send(&protocol::encode_reset_config())?;
        Ok(())
    }

    /// Re-read configuration from flash, overwriting RAM.
    pub fn reload_from_flash(&mut self) -> Result<(), DeviceError> {
        self.transport.drain()?;
        self.transport.send(&protocol::encode_reload())?;
        Ok(())
    }

    /// Apply a [`Preset`] in one shot. The device will internally
    /// reconfigure its sockets and audio routing.
    pub fn apply_preset(&mut self, p: Preset) -> Result<(), DeviceError> {
        self.transport.drain()?;
        self.transport.send(&protocol::encode_preset(p))?;
        Ok(())
    }

    /// Send CONFIG_ACK to enable socket power on v1.5+ boards. Required
    /// before sockets behave normally if `DeviceConfig::need_confirmation`
    /// is true.
    /// fw 0.9+: read the USB RX ringbuffer setting (READ_CONFIG byte 59).
    pub fn read_rx_ring(&mut self) -> Result<bool, DeviceError> {
        let buf = self.read_config_frame_with_retry()?;
        Ok(protocol::decode_rx_ring(&buf)?)
    }

    /// fw 0.9+: set the USB RX ringbuffer. The board saves and reboots
    /// (re-enumerates) when the value changes; an unchanged value is a no-op.
    pub fn set_rx_ring(&mut self, on: bool) -> Result<(), DeviceError> {
        self.transport.drain()?;
        self.transport.send(&protocol::encode_config_cmd(
            protocol::cfg::SET_RX_RING,
            on as u8,
            0,
        ))?;
        Ok(())
    }

    pub fn confirm_config(&mut self) -> Result<(), DeviceError> {
        self.transport.drain()?;
        self.transport.send(&protocol::encode_config_ack())?;
        Ok(())
    }

    /// Trigger the firmware's SID-detection routine. The device probes each
    /// configured socket, identifies whether it holds a 6581 or 8580 chip,
    /// and updates its own `socketN.sidM.type` fields accordingly. Takes
    /// ~3 seconds in the Clojure tool; the caller should sleep before
    /// reading the config back.
    ///
    /// Use this when a socket's chip_type is `Real` and you want the
    /// firmware to identify the actual silicon. Manually-set SID types are
    /// overwritten by detection — so set chip_type to a *non-`Real`* value
    /// if you want manual control.
    pub fn detect_sids(&mut self) -> Result<(), DeviceError> {
        self.transport.drain()?;
        self.transport.send(&protocol::encode_packet(
            protocol::cfg::DETECT_SIDS,
            [0, 0, 0, 0],
        ))?;
        Ok(())
    }

    /// Full hardware probe: detects both chip types (real vs clones) and
    /// the SID models in each socket, then saves the result. Takes ~3 s.
    pub fn auto_detect(&mut self) -> Result<(), DeviceError> {
        self.transport.drain()?;
        self.transport.send(&protocol::encode_packet(
            protocol::cfg::AUTO_DETECT,
            [0, 0, 0, 0],
        ))?;
        Ok(())
    }

    /// Probe for known SID-clone chips (ARMSID, SKPico, FPGASID, etc.) in
    /// each socket. Takes ~2 s.
    pub fn detect_clones(&mut self) -> Result<(), DeviceError> {
        self.transport.drain()?;
        self.transport.send(&protocol::encode_packet(
            protocol::cfg::DETECT_CLONES,
            [0, 0, 0, 0],
        ))?;
        Ok(())
    }

    /// Read the firmware version string (e.g. `"0.6.0-BETA.20250101"`).
    ///
    /// Retries up to 3 times on empty / unparseable responses with a 50 ms
    /// delay between attempts. The firmware can return garbage on the first
    /// read after connection or right after another command — the Clojure
    /// Configtool uses the same retry pattern.
    pub fn read_firmware_version(&mut self) -> Result<String, DeviceError> {
        self.read_version_with_retry("firmware version", cfg::USBSID_VERSION, |this| {
            this.transport
                .send(&protocol::encode_read_firmware_version())
        })
    }

    /// Read the PCB version string (e.g. `"1.5"`). Same retry behaviour as
    /// [`Self::read_firmware_version`].
    pub fn read_pcb_version(&mut self) -> Result<String, DeviceError> {
        self.read_version_with_retry("PCB version", cfg::US_PCB_VERSION, |this| {
            this.transport.send(&protocol::encode_read_pcb_version())
        })
    }

    /// [`Self::read_pcb_version`] + parse into [`crate::PcbVersion`].
    /// Unparseable replies fold to [`crate::PcbVersion::Unknown`].
    pub fn read_pcb_version_typed(&mut self) -> Result<crate::PcbVersion, DeviceError> {
        let raw = self.read_pcb_version()?;
        Ok(crate::parse_pcb_version(&raw)
            .map(crate::PcbVersion::from_raw)
            .unwrap_or(crate::PcbVersion::Unknown))
    }

    fn read_version_with_retry(
        &mut self,
        what: &'static str,
        expected_cmd: u8,
        send_request: impl Fn(&mut Self) -> Result<(), TransportError>,
    ) -> Result<String, DeviceError> {
        let mut last_detail = String::new();
        for attempt in 0..3 {
            if attempt > 0 {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            // Drain on every attempt so a stale or partial response from a
            // previous attempt can't be mistaken for the current one's reply.
            self.transport.drain()?;
            send_request(self)?;
            let buf = match self.transport.recv(64) {
                Ok(b) => b,
                Err(e) => {
                    last_detail = format!("recv error: {e}");
                    continue;
                }
            };
            if buf.is_empty() {
                last_detail = "empty response".into();
                continue;
            }
            if let Some(v) = protocol::decode_version_response(&buf, expected_cmd) {
                return Ok(v);
            }
            last_detail = format!(
                "echo=0x{:02X} (want 0x{:02X}), len={}, head={:02X?}",
                buf[0],
                expected_cmd,
                buf.len(),
                &buf[..buf.len().min(8)],
            );
        }
        Err(DeviceError::UnexpectedResponse {
            what,
            detail: last_detail,
        })
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::MockTransport;

    fn pal_buffer() -> Vec<u8> {
        let mut b = vec![0u8; 64];
        b[0] = 0x30;
        b[1] = 0x7F;
        b[63] = 0xFF;
        // PAL clock value: 985_248 = 0x0F_07_E0
        b[7] = 0x0F;
        b[8] = 0x07;
        b[9] = 0xE0;
        b[10] = 1; // socket1 enabled
        b[12] = 1; // chiptype = Unknown (id=1)
        b[14] = 1; // sid1 type = Na
        b[15] = 1; // sid2 type = Na
        b[20] = 1; // socket2 enabled
        b[22] = 1;
        b[24] = 1; // sid1 id=1 (low nibble)
        b[25] = 1;
        b[26] = 1;
        b[30] = 1; // led on
        b[51] = 1; // CDC on
        b[52] = 1; // WebUSB on
        b
    }

    #[test]
    fn read_config_round_trip() {
        let buf = pal_buffer();
        let transport = MockTransport::new().with_response(buf);
        let mut dev = Device::new(transport);
        let cfg = dev.read_config().unwrap();
        assert_eq!(cfg.clock_rate, crate::model::ClockRate::Pal);
        assert!(cfg.socket1.enabled);
        assert!(cfg.protocols.cdc);
    }

    #[test]
    fn apply_sends_a_single_packet() {
        let mut transport = MockTransport::new();
        let _ = transport.recv(0); // ignore
        let mut dev = Device::new(transport);
        dev.apply().unwrap();
        assert_eq!(dev.transport().sent.len(), 1);
        assert_eq!(dev.transport().sent[0], vec![0xD2, 0x31, 0, 0, 0, 0]);
    }

    #[test]
    fn save_and_reboot_packet() {
        let mut dev = Device::new(MockTransport::new());
        dev.save_and_reboot().unwrap();
        assert_eq!(dev.transport().sent[0], vec![0xD2, 0x33, 0, 0, 0, 0]);
    }

    #[test]
    fn apply_preset_dispatches_correct_command() {
        let mut dev = Device::new(MockTransport::new());
        dev.apply_preset(Preset::DualBoth).unwrap();
        assert_eq!(dev.transport().sent[0], vec![0xD2, 0x41, 0, 0, 0, 0]);
        dev.apply_preset(Preset::Mirrored).unwrap();
        assert_eq!(dev.transport().sent[1], vec![0xD2, 0x45, 0, 0, 0, 0]);
    }

    #[test]
    fn write_config_emits_many_packets() {
        let buf = pal_buffer();
        let transport = MockTransport::new().with_response(buf);
        let mut dev = Device::new(transport);
        let cfg = dev.read_config().unwrap();
        let sent_before = dev.transport().sent.len();
        dev.write_config(&cfg).unwrap();
        let sent_after = dev.transport().sent.len();
        // 1 clock + 2 sockets * 5 items + 2 LED + 4 RGB + 6 audio routing
        //   flags + 2 v1.5-only opcodes (socket_change_detect,
        //   preset_auto_detect) = 25
        assert_eq!(sent_after - sent_before, 25);
    }

    #[test]
    fn firmware_version_parse() {
        // Cmd echo + length 5 + "1.2.3"
        let resp = vec![cfg::USBSID_VERSION, 5, b'1', b'.', b'2', b'.', b'3', 0, 0];
        let mut dev = Device::new(MockTransport::new().with_response(resp));
        assert_eq!(dev.read_firmware_version().unwrap(), "1.2.3");
    }

    #[test]
    fn pcb_version_parse() {
        let resp = vec![cfg::US_PCB_VERSION, 3, b'1', b'.', b'5', 0, 0];
        let mut dev = Device::new(MockTransport::new().with_response(resp));
        assert_eq!(dev.read_pcb_version().unwrap(), "1.5");
    }

    #[test]
    fn pcb_version_typed_maps_to_enum() {
        let resp = vec![cfg::US_PCB_VERSION, 3, b'1', b'.', b'5', 0, 0];
        let mut dev = Device::new(MockTransport::new().with_response(resp));
        assert_eq!(
            dev.read_pcb_version_typed().unwrap(),
            crate::PcbVersion::V1_5
        );
    }

    // ─── Ph4 retry + legacy-drain regression tests ──────────────────

    #[test]
    fn read_config_first_attempt_recovers_after_transient_failure() {
        // First 64B read returns garbage (no valid framing), second
        // attempt returns a proper PAL buffer. Retry should succeed.
        let mut transport = MockTransport::new()
            .with_response(vec![0xAA; CONFIG_BUFFER_LEN]) // attempt 1: bad framing
            .with_response(vec![0xBB; CONFIG_BUFFER_LEN * 4]) // attempt 1: bad drain too
            .with_response(pal_buffer().to_vec()); // attempt 2: good
        let mut dev = Device::new(transport);
        let cfg = dev.read_config().unwrap();
        assert_eq!(cfg.clock_rate, crate::model::ClockRate::Pal);
    }

    #[test]
    fn read_config_legacy_drain_picks_up_second_window() {
        // Legacy firmware behaviour: single 64B recv returns something
        // wrong-sized OR unframed, the 256B drain has the valid frame
        // at the second 64-byte window.
        let mut drained = vec![0x11u8; CONFIG_BUFFER_LEN]; // garbage window 0
        drained.extend_from_slice(&pal_buffer()); // valid frame at offset 64
        drained.extend_from_slice(&vec![0x22u8; CONFIG_BUFFER_LEN * 2]); // rest is filler
        let transport = MockTransport::new()
            .with_response(vec![]) // first 64B recv returns empty → falls through
            .with_response(drained);
        let mut dev = Device::new(transport);
        let cfg = dev.read_config().unwrap();
        assert_eq!(cfg.clock_rate, crate::model::ClockRate::Pal);
    }

    #[test]
    fn read_config_gives_up_after_three_attempts() {
        // Three full attempts of unreadable data → hard error.
        let transport = MockTransport::new();
        // Six garbage responses (3 attempts × [normal + drain]).
        let mut transport = transport;
        for _ in 0..6 {
            transport = transport.with_response(vec![0xAA; CONFIG_BUFFER_LEN]);
        }
        let mut dev = Device::new(transport);
        let err = dev.read_config().unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("gave up after 3 attempts"),
            "unexpected error message: {msg}"
        );
    }
}
