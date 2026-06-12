# usbsid-pico-config

Rust crate to read and write [USBSID-Pico](https://github.com/LouDnl/USBSID-Pico) hardware configuration over USB.

## Features

- Read/write the device's 64-byte configuration buffer
- Built-in presets (single / dual / triple / quad / mirrored / flipped / mixed)
- Clock rate selection (PAL / NTSC / DREAN / NTSC2 / default)
- Chip-type routing per socket (real SID + 8 clone models)
- Firmware + PCB version readout
- Hardware SID detection (`detect-sids`, `auto-detect`, `detect-clones`)

## Usage

```rust
use usbsid_pico_config::{Device, Preset};
use usbsid_pico_config::transport::StandaloneTransport;

let mut dev = Device::new(StandaloneTransport::open()?);

let cfg = dev.read_config()?;
println!("clock = {}", cfg.clock_rate.label());

dev.apply_preset(Preset::DualBoth)?;
dev.save_no_reset()?;
```

For host applications that already own the USB session (e.g. through a privileged daemon), disable the default feature and implement `Transport` yourself:

```toml
usbsid-pico-config = { version = "0.1", default-features = false }
```

## CLI

```sh
cargo run --example usbsid_config -- info             # firmware + PCB + config
cargo run --example usbsid_config -- read             # config only
cargo run --example usbsid_config -- preset dual-both
cargo run --example usbsid_config -- clock pal
cargo run --example usbsid_config -- socket 1 chip real
cargo run --example usbsid_config -- socket 2 sid1 mos8580
cargo run --example usbsid_config -- audio stereo true
cargo run --example usbsid_config -- detect-sids
cargo run --example usbsid_config -- save             # persist to flash
cargo run --example usbsid_config -- reset            # factory defaults
cargo run --example usbsid_config -- raw-config       # hex dump
```

`cargo run --example usbsid_config -- --help` lists every subcommand.

### Linux permissions

```
# /etc/udev/rules.d/99-usbsid-pico.rules
SUBSYSTEM=="usb", ATTR{idVendor}=="cafe", ATTR{idProduct}=="4011", MODE="0660", TAG+="uaccess"
```

```sh
sudo udevadm control --reload-rules && sudo udevadm trigger
```

### macOS

Run the example with `sudo` — libusb can't claim the USBSID-Pico's vendor interface without root.

## Protocol

The USBSID-Pico exposes configuration over USB bulk transfers on endpoint 0x02 (OUT) / 0x82 (IN), interface 1.

Each outbound packet is 6 bytes:

```
byte 0   framing      0xD2  = (Cmd::COMMAND << 6) | Cmd::CONFIG
byte 1   command      e.g. 0x30 READ_CONFIG, 0x32 SET_CONFIG, 0x80 USBSID_VERSION
byte 2-5 arguments    command-specific
```

`READ_CONFIG` returns a 64-byte buffer; framing is `[0]=0x30`, `[1]=0x7F`, `[63]=0xFF`. Decoding is documented inline in [`src/protocol.rs`](src/protocol.rs); the source-of-truth for byte offsets is the Clojure-based [USBSID-Pico-Configtool](https://github.com/LouDnl/USBSID-Pico-Configtool), specifically its `driver.clj` and the bundled `usbsid-usb-driver-library-java-1.1.jar`.

`SET_CONFIG` writes one field per packet via `(section, item, value)` triplets. Sections: 0x0 clock, 0x1/0x2 sockets, 0x3 LED, 0x4 RGB, 0x9 FMOpl, 0xA stereo, 0xB lock_audio_switch, 0xC mirrored, 0xD flipped, 0xE mixed.

Command bytes ≥ 0x40 cover presets, hardware detection, version queries, and CONFIG_ACK. The full enum is in [`src/protocol.rs`](src/protocol.rs).

## Tested against

- USBSID-Pico firmware **0.7.0**, PCB **v1.3**

Other firmware revisions should work for the common path. Unknown enum values surface as `ConfigError::UnknownEnumId`; call `read_config_lenient()` to map them to `Unknown` instead of erroring.

## Status

- Read path stable, all unit tests passing (21/21)
- Write path validated end-to-end on real hardware via the integration suite at [`tests/real_device.rs`](tests/real_device.rs)
- `READ_SOCKETCFG`, `AUTO_DETECT`, `DETECT_CLONES`, MIDI / audio-toggle commands wired but not exhaustively tested - use at your own risk

## License

MIT OR Apache-2.0
