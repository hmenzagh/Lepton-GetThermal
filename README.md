# Lepton-GetThermal

> **Warning:** This entire application was coded by AI agents (Claude Code). Review the code accordingly before using it in any critical context.

A macOS thermal camera viewer for the FLIR Lepton module via PureThermal USB adapter. Built with Tauri v2, Rust, and React.

Lepton-GetThermal bypasses AVFoundation — which only exposes pre-processed BGRA frames — to stream raw **Y16 radiometric data** directly over IOKit isochronous USB. This preserves the full 16-bit thermal values needed for accurate temperature measurement and custom processing.

## Features

- **Raw Y16 streaming** — Direct IOKit USB isochronous transfers from PureThermal (160×120 @ 9 fps)
- **Color palettes** — Iron Black (FLIR-style), Rainbow, Grayscale
- **Polarity** — White-hot / Black-hot toggle
- **Isotherm overlay** — Configurable temperature threshold with striped visual overlay
- **Neural network upscaling** — ESPCN 3× super-resolution via embedded ONNX model (160×120 → 480×360)
- **Radiometry** — Spot temperature readout in °C/°F with interactive spotmeter ROI
- **Min/Max markers** — Visual pins showing hottest and coldest pixel locations
- **Flat-field correction** — Trigger shutter recalibration
- **Screenshot capture** — Save frames as PNG

## Prerequisites

- **macOS 13+** (Apple Silicon or Intel)
- **Node.js** (v18+)
- **Rust** (stable toolchain)
- **FLIR Lepton** module with **PureThermal** USB adapter (VID `1e4e`, PID `0100`)

## Getting started

```bash
# Install frontend dependencies
npm install

# Run in development mode
npm run tauri dev

# Build for production
npm run tauri build
```

The production build outputs:
- `src-tauri/target/release/bundle/macos/Lepton-GetThermal.app`
- `src-tauri/target/release/bundle/dmg/Lepton-GetThermal_0.1.0_aarch64.dmg`

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│  React Frontend                                         │
│  ┌──────────┐  ┌──────────┐  ┌───────────────────────┐  │
│  │ Control  │  │  Video   │  │ Overlays (markers,    │  │
│  │  Panel   │  │  Canvas  │  │ spotmeter, isotherm)  │  │
│  └────┬─────┘  └────▲─────┘  └───────────────────────┘  │
│       │  Tauri       │ base64 RGBA                       │
│       │  invoke()    │ "thermal-frame" event              │
├───────┼──────────────┼───────────────────────────────────┤
│  Rust Backend        │                                   │
│       │         ┌────┴──────────────┐                    │
│       ▼         │ Processing Pipeline│                   │
│  ┌─────────┐    │  Y16 → auto-gain  │                   │
│  │ Lepton  │    │  → [invert]       │                   │
│  │   SDK   │    │  → [upscale ONNX] │                   │
│  │  (UVC)  │    │  → colorize (LUT) │                   │
│  └────┬────┘    │  → [isotherm]     │                   │
│       │         │  → RGBA           │                   │
│       │         └────▲──────────────┘                    │
│       │              │ raw Y16 frames                    │
│       │         ┌────┴────────────┐                      │
│       └────────►│  IOKit USB      │                      │
│                 │  (isochronous)  │                      │
│                 └────────┬────────┘                      │
└──────────────────────────┼───────────────────────────────┘
                           │
                    PureThermal USB
                     FLIR Lepton
```

### Processing pipeline

Each frame follows this path:

1. **Auto-gain** — Normalize 16-bit Y16 to 8-bit grayscale (linear min/max stretch)
2. **Invert** — Flip grayscale if polarity is black-hot
3. **Upscale** *(optional)* — ESPCN 3× neural network super-resolution
4. **Colorize** — Apply 256-entry RGB palette LUT
5. **Isotherm** *(optional)* — Overlay red/white stripes on pixels above threshold

### Why IOKit instead of AVFoundation?

PureThermal exposes two video formats via UVC: BGRA (processed by the device's internal AGC) and Y16 (raw 16-bit radiometric). AVFoundation only surfaces the BGRA format. To get raw thermal data for accurate temperature measurement and custom processing, Lepton-GetThermal talks to the device directly via IOKit isochronous USB transfers.

## Project structure

```
src/                        # React frontend
├── components/             # UI components (canvas, controls, overlays)
├── hooks/                  # useCamera (Tauri commands), useFrameStream (events)
└── lib/types.ts            # TypeScript interfaces

src-tauri/                  # Rust backend
├── src/
│   ├── camera/             # Lepton SDK (UVC extension units), frame acquisition
│   ├── commands/           # Tauri command handlers (stream, controls)
│   ├── processing/         # Auto-gain, colorization, palettes, ONNX upscaling
│   ├── usb_stream.rs       # IOKit isochronous USB streaming
│   ├── usb_helper.c        # IOKit C wrapper (FFI)
│   ├── uvc_descriptors.rs  # USB descriptor parsing
│   └── uvc_payload.rs      # UVC payload reassembly
└── models/
    └── super_resolution.onnx  # Embedded ESPCN 3× model
```

## Tech stack

| Layer | Technology |
|-------|-----------|
| Framework | Tauri v2 |
| Frontend | React 18, TypeScript, Vite |
| Backend | Rust (2021 edition) |
| USB | IOKit (via C FFI) |
| ML inference | ONNX Runtime (`ort` crate) |
| Camera SDK | FLIR Lepton UVC extension units |

## License

All rights reserved.
