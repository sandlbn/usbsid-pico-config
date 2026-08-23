//! USBSID-Pico configuration protocol in Rust.
//!
//! Talks to the USBSID-Pico hardware to read and write its device-side
//! configuration: chip routing per socket, clock rate (PAL/NTSC/etc.),
//! audio routing modes, LED settings, firmware version, etc.
//!
//! Reimplementation of the protocol layer from the upstream Clojure
//! [USBSID-Pico Configtool](https://github.com/LouDnl/USBSID-Pico-Configtool),
//! which in turn used a vendored Java USB driver. Wire-compatible with the
//! same firmware revisions (≥ v0.4 confirmed; older revisions may lack
//! the CONFIG_ACK handshake).
//!
//! # Layering
//!
//! The crate is split into:
//!
//! - [`model`]: the logical [`DeviceConfig`] tree plus the [`Preset`],
//!   [`ClockRate`], [`ChipType`] and [`SidType`] enums.
//! - [`protocol`]: byte-level command encoding + 64-byte config-buffer
//!   decoding. Stateless. Easy to fuzz / unit-test.
//! - [`transport`]: the [`Transport`] trait the [`Device`] calls into,
//!   plus an optional `rusb`-backed [`StandaloneTransport`] for CLI use.
//! - [`device`]: the high-level [`Device<T>`] facade — `read_config()`,
//!   `apply()`, `save_to_flash()`, `apply_preset()`, etc.
//!
//! # Choosing a transport
//!
//! - **Stand-alone CLI / tool**: enable the default `rusb-transport` feature
//!   and use [`transport::StandaloneTransport::open`].
//! - **Embedded in a larger app that already owns the USB device** (e.g.
//!   Phosphor, which routes USB through a macOS LaunchDaemon): set
//!   `default-features = false` and implement [`Transport`] yourself.
//!   The trait surface is intentionally tiny — `send` + `recv` — so the
//!   host app keeps full control of session management, retries, and IPC.

pub mod device;
#[cfg(feature = "fpgasid")]
pub mod fpgasid;
pub mod ini_io;
pub mod model;
pub mod protocol;
pub mod transport;

pub use device::Device;
#[cfg(feature = "fpgasid")]
pub use fpgasid::{parse_diag as parse_fpgasid_diag, FpgaSidDiag, FpgaSidError, FpgaSidSlotConfig};
pub use ini_io::{read_ini, read_ini_version, write_ini};
pub use model::{
    flat_chip_to_legacy, fw_capabilities, fw_flavor_from_version, legacy_to_flat_chip,
    parse_pcb_version, Capabilities, ChipType, ClockRate, DeviceConfig, FwFlavor, LedConfig,
    PcbVersion, Preset, ProtocolToggles, RgbLedConfig, SidSlot, SidType, SocketConfig,
};
pub use protocol::{ConfigError, CONFIG_BUFFER_LEN};
pub use transport::{Transport, TransportError};
