//! USB transport abstraction.
//!
//! The [`Transport`] trait is intentionally tiny: two methods, send and
//! receive raw bytes. That keeps the protocol layer free of any I/O
//! coupling and lets host applications (notably [Phosphor](https://github.com/sandlbn/Phosphor))
//! plug in their own daemon-bridge, mock transport for tests, or whatever
//! they need without having to fight a more elaborate abstraction.
//!
//! For stand-alone CLI use, the [`StandaloneTransport`] (behind the
//! `rusb-transport` feature) opens the USBSID-Pico directly via `rusb`.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum TransportError {
    #[error("USB I/O error: {0}")]
    Io(String),
    #[error("device not found (VID={vid:#06X}, PID={pid:#06X})")]
    DeviceNotFound { vid: u16, pid: u16 },
    #[error("device not open")]
    NotOpen,
    #[error("timeout after {ms} ms")]
    Timeout { ms: u32 },
    #[error("short read: got {got} bytes, expected {expected}")]
    ShortRead { got: usize, expected: usize },
}

/// Wire-byte transport for the USBSID-Pico configuration protocol.
///
/// Implementations send a full 6-byte config packet via [`send`] and read
/// the device's response (variable length: 1 byte ACK, 64 bytes for
/// READ_CONFIG, ~32 bytes for version queries) via [`recv`].
///
/// All methods are blocking. If you need async, wrap the calls in a
/// `spawn_blocking` task on the host side.
///
/// [`send`]: Transport::send
/// [`recv`]: Transport::recv
pub trait Transport {
    /// Send `bytes` to the device's bulk-OUT endpoint. Must be exactly one
    /// 6-byte packet (the protocol crate produces nothing else), but the
    /// trait doesn't enforce that — implementations forward the bytes as-is.
    fn send(&mut self, bytes: &[u8]) -> Result<(), TransportError>;

    /// Read up to `len` bytes from the bulk-IN endpoint. The returned `Vec`
    /// may be shorter than `len` if the device sends a partial response.
    /// Implementations should apply a reasonable timeout (~250 ms) so a
    /// missing device doesn't hang forever.
    fn recv(&mut self, len: usize) -> Result<Vec<u8>, TransportError>;

    /// Best-effort flush of any bytes the device has already queued on the
    /// bulk-IN endpoint. Used before each new command to prevent leftover
    /// data from a previous response (USB bulk transfers can leave trailing
    /// zero-length packets or oversized payloads) leaking into the next
    /// read and making it look like the wrong command echoed back.
    ///
    /// Implementations should use a very short timeout (~5-20 ms) so this
    /// returns quickly on an idle endpoint. The default does nothing —
    /// transports that don't need it (e.g. mocks, IPC bridges) can leave it.
    fn drain(&mut self) -> Result<(), TransportError> {
        Ok(())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  StandaloneTransport — direct rusb backend
// ─────────────────────────────────────────────────────────────────────────────

/// USB IDs for the USBSID-Pico hardware.
pub const VENDOR_ID: u16 = 0xCAFE;
pub const PRODUCT_ID: u16 = 0x4011;

/// Bulk OUT endpoint (host → device).
pub const EP_OUT: u8 = 0x02;
/// Bulk IN endpoint (device → host).
pub const EP_IN: u8 = 0x82;

/// The USB interface index for normal control / config traffic.
pub const INTERFACE: u8 = 1;

#[cfg(feature = "rusb-transport")]
pub use standalone::StandaloneTransport;

#[cfg(feature = "rusb-transport")]
mod standalone {
    use super::*;
    use rusb::{Context, DeviceHandle, UsbContext};
    use std::time::Duration;

    /// Default I/O timeout. Configured high enough that the longest commands
    /// (AUTO_DETECT can take ~3 s; we cap shorter so a stuck device fails
    /// loudly) still complete. Callers can override per-command.
    pub const DEFAULT_TIMEOUT_MS: u32 = 1000;

    /// Direct `rusb`-backed USB transport. Opens the USBSID-Pico device and
    /// holds its handle for the lifetime of the struct.
    ///
    /// Intended for stand-alone CLI / tool use. Host applications that
    /// already own the USB device (e.g. through a daemon bridge) should
    /// implement [`Transport`] themselves and avoid this type.
    pub struct StandaloneTransport {
        handle: DeviceHandle<Context>,
        timeout: Duration,
    }

    impl StandaloneTransport {
        /// Open the first USBSID-Pico found on the system.
        pub fn open() -> Result<Self, TransportError> {
            Self::open_with_timeout(DEFAULT_TIMEOUT_MS)
        }

        /// Open with a custom timeout (milliseconds).
        pub fn open_with_timeout(timeout_ms: u32) -> Result<Self, TransportError> {
            let ctx = Context::new().map_err(|e| TransportError::Io(format!("rusb init: {e}")))?;
            let handle = ctx.open_device_with_vid_pid(VENDOR_ID, PRODUCT_ID).ok_or(
                TransportError::DeviceNotFound {
                    vid: VENDOR_ID,
                    pid: PRODUCT_ID,
                },
            )?;
            // Detach any kernel driver before claiming the interface (no-op on
            // platforms that don't support it).
            #[cfg(not(target_os = "windows"))]
            {
                let _ = handle.set_auto_detach_kernel_driver(true);
            }
            handle
                .claim_interface(INTERFACE)
                .map_err(|e| TransportError::Io(format!("claim_interface({INTERFACE}): {e}")))?;
            Ok(Self {
                handle,
                timeout: Duration::from_millis(timeout_ms as u64),
            })
        }
    }

    impl Drop for StandaloneTransport {
        fn drop(&mut self) {
            let _ = self.handle.release_interface(INTERFACE);
        }
    }

    impl Transport for StandaloneTransport {
        fn send(&mut self, bytes: &[u8]) -> Result<(), TransportError> {
            let n = self
                .handle
                .write_bulk(EP_OUT, bytes, self.timeout)
                .map_err(|e| match e {
                    rusb::Error::Timeout => TransportError::Timeout {
                        ms: self.timeout.as_millis() as u32,
                    },
                    other => TransportError::Io(format!("write_bulk: {other}")),
                })?;
            if n != bytes.len() {
                return Err(TransportError::Io(format!(
                    "short write: {n}/{} bytes",
                    bytes.len()
                )));
            }
            Ok(())
        }

        fn recv(&mut self, len: usize) -> Result<Vec<u8>, TransportError> {
            let mut buf = vec![0u8; len];
            let n = self
                .handle
                .read_bulk(EP_IN, &mut buf, self.timeout)
                .map_err(|e| match e {
                    rusb::Error::Timeout => TransportError::Timeout {
                        ms: self.timeout.as_millis() as u32,
                    },
                    other => TransportError::Io(format!("read_bulk: {other}")),
                })?;
            buf.truncate(n);
            Ok(buf)
        }

        fn drain(&mut self) -> Result<(), TransportError> {
            // Up to 8 short reads, each with a 10 ms timeout, until the
            // endpoint is idle. The cap exists so a stuck device can't make
            // drain() spin forever; in practice we expect 0-1 reads.
            let drain_timeout = Duration::from_millis(10);
            let mut scratch = [0u8; 64];
            for _ in 0..8 {
                match self.handle.read_bulk(EP_IN, &mut scratch, drain_timeout) {
                    Ok(0) => break,
                    Ok(_) => continue, // there was data — keep draining
                    Err(rusb::Error::Timeout) => break, // nothing pending — done
                    Err(other) => {
                        return Err(TransportError::Io(format!("drain read_bulk: {other}")))
                    }
                }
            }
            Ok(())
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  Mock transport for tests
// ─────────────────────────────────────────────────────────────────────────────

/// Test helper: a transport that records every `send` and returns canned
/// `recv` responses in FIFO order. Public so downstream test suites can
/// reuse it.
pub struct MockTransport {
    pub sent: Vec<Vec<u8>>,
    pub responses: std::collections::VecDeque<Vec<u8>>,
}

impl MockTransport {
    pub fn new() -> Self {
        Self {
            sent: Vec::new(),
            responses: std::collections::VecDeque::new(),
        }
    }

    pub fn with_response(mut self, bytes: Vec<u8>) -> Self {
        self.responses.push_back(bytes);
        self
    }

    pub fn push_response(&mut self, bytes: Vec<u8>) {
        self.responses.push_back(bytes);
    }
}

impl Default for MockTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl Transport for MockTransport {
    fn send(&mut self, bytes: &[u8]) -> Result<(), TransportError> {
        self.sent.push(bytes.to_vec());
        Ok(())
    }

    fn recv(&mut self, len: usize) -> Result<Vec<u8>, TransportError> {
        match self.responses.pop_front() {
            Some(b) => Ok(b.into_iter().take(len).collect()),
            None => Err(TransportError::Io(
                "MockTransport: no canned response".into(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_records_sends() {
        let mut t = MockTransport::new();
        t.send(&[1, 2, 3]).unwrap();
        t.send(&[4, 5, 6]).unwrap();
        assert_eq!(t.sent, vec![vec![1, 2, 3], vec![4, 5, 6]]);
    }

    #[test]
    fn mock_returns_queued_responses() {
        let mut t = MockTransport::new()
            .with_response(vec![0xAA, 0xBB])
            .with_response(vec![0xCC]);
        assert_eq!(t.recv(4).unwrap(), vec![0xAA, 0xBB]);
        assert_eq!(t.recv(1).unwrap(), vec![0xCC]);
        assert!(t.recv(1).is_err());
    }
}
