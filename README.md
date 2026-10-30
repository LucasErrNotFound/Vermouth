<div align="center">

<img src="docs/images/Vermouth.png" alt="Vermouth logo" width="256" />

# Vermouth

*A secret makes a woman, woman*

A native desktop workspace for CCTV and IP cameras.

[Overview](#overview) · [Showcase](#showcase) · [Themes](#themes) · [Getting started](#getting-started) · [Build from source](#build-from-source)

</div>

---

> [!NOTE]
> **Vermouth is under active development.** The current focus is live viewing and camera management on Windows. Some pages and capabilities are still works in progress; this README distinguishes working functionality from planned features.

## Overview

Vermouth brings network camera monitoring into a clean, responsive native desktop interface. Add an authorized RTSP camera, manage multiple feeds, move between a camera grid and a focused view, and access controls appropriate to the connected device.

Rather than tying the experience to one camera brand or a cloud account, Vermouth is being built around standard camera protocols, local-first workflows, and clearly communicated connection states. Its interface uses GPUI-Kit, while the current video pipeline uses FFmpeg for RTSP decoding.

## Main screenshot

The Live View workspace keeps the camera feeds at the center, with navigation and contextual controls available when needed.

> **No screenshot yet.** A screenshot of the actual running application will be added here.

## Showcase

### Live View

Watch multiple streams in an adaptive grid, select a camera, and switch to a focused view. The application is designed to adapt to window resizing rather than rely on a single fixed desktop resolution.

> **Demo capture pending.**

<!--
![Vermouth Live View demonstration](docs/gifs/live-view.gif)
-->

### Camera management

Add, edit, connect, disconnect, retry, and remove configured RTSP cameras. The Cameras page also includes best-effort private-network discovery to help identify possible camera services; finding a device does not guarantee a usable video stream.

> **Demo capture pending.**

<!--
![Vermouth camera management demonstration](docs/gifs/camera-management.gif)
-->

### Camera controls

For supported, authorized cameras, Vermouth exposes ONVIF-based pan, tilt, and zoom controls. Availability and behavior depend on the particular device, its configuration, and the permissions supplied to the app.

> **Demo capture pending.**

<!--
![Vermouth ONVIF camera controls](docs/gifs/camera-controls.gif)
-->

### Appearance and workspace

Switch between light and dark appearances, select a theme family, adjust interface density, and work with the collapsible navigation sidebar.

> **Demo capture pending.**

<!--
![Vermouth theme and workspace customization](docs/gifs/appearance.gif)
-->

## Themes

Vermouth uses GPUI-Kit's theme system for consistent native controls and a customizable interface. Available palettes depend on the theme assets included with the build.

### Light mode

<table>
  <tr>
    <td align="center" width="50%">
      <strong>Default Light</strong><br />
      <em>Screenshot capture pending</em>
      <!-- <img src="docs/images/themes/light/default-light.png" alt="Vermouth in Default Light" width="480" /> -->
    </td>
    <td align="center" width="50%">
      <strong>Everforest Light</strong><br />
      <em>Screenshot capture pending</em>
      <!-- <img src="docs/images/themes/light/everforest-light.png" alt="Vermouth in Everforest Light" width="480" /> -->
    </td>
  </tr>
</table>

### Dark mode

<table>
  <tr>
    <td align="center" width="50%">
      <strong>Default Dark</strong><br />
      <em>Screenshot capture pending</em>
      <!-- <img src="docs/images/themes/dark/default-dark.png" alt="Vermouth in Default Dark" width="480" /> -->
    </td>
    <td align="center" width="50%">
      <strong>Catppuccin Mocha</strong><br />
      <em>Screenshot capture pending</em>
      <!-- <img src="docs/images/themes/dark/catppuccin-mocha.png" alt="Vermouth in Catppuccin Mocha" width="480" /> -->
    </td>
  </tr>
</table>

Additional theme families in the application include Ayu, Flexoki, Gruvbox, Solarized, and others. The precise list and light/dark variants are determined by the installed theme catalog.

## Features

- **Native desktop interface:** Rust and GPUI-Kit, without a browser-based frontend.
- **RTSP live streaming:** display compatible, user-authorized IP camera feeds through an FFmpeg-based media pipeline.
- **Multi-camera monitoring:** adaptive grid layouts, camera selection, and focused viewing.
- **Camera management:** add, edit, persist, connect, disconnect, retry, and remove camera entries.
- **Local-network discovery:** identify candidate services on private networks, with explicit compatibility limitations.
- **ONVIF PTZ controls:** camera-dependent movement controls when supported and permitted by the device.
- **Connection feedback:** distinguish connecting, live, stale, disconnected, and error conditions.
- **Configurable appearance:** light/dark modes, theme families, interface density, and collapsible navigation.
- **Performance-conscious presentation:** bounded frame handoff and reduced presentation work when Live View is hidden, as the navigation integration is completed.

### Planned, not yet advertised as complete

- Local snapshots and recording management.
- Recorded-video playback and retention controls.
- More comprehensive diagnostics and redacted exports.
- Wider camera compatibility and verified support beyond Windows.

## Getting started

At this development stage, **building from source is the documented way to run Vermouth**. See [Build from source](#build-from-source).

1. Launch Vermouth and open **Cameras** from the sidebar.
2. Select **Add RTSP camera** to enter a camera name and a valid RTSP endpoint, or use the network scan to discover possible devices before configuring a stream.
3. Save the camera and connect using access you are authorized to use.
4. Open **Live View** to monitor the feed. Select a camera to use the focused layout.
5. If the camera exposes supported ONVIF PTZ capabilities, use the available movement controls. Otherwise, those features may be unavailable.
6. Use **Disconnect** when you want to stop the stream, or **Remove** to delete a saved camera entry from Vermouth. Removing an entry does not change the camera itself.

A discovered device, an open network port, an authenticated endpoint, and a successfully decoded live stream are different things. Vermouth should not label a camera **Live** until actual video frames are received.

### Camera compatibility

| Capability | Current direction |
| --- | --- |
| RTSP | Primary video transport; actual playback depends on stream format and FFmpeg decoding support. |
| ONVIF discovery | Best-effort identification of devices and advertised services. |
| ONVIF PTZ | Supported on compatible, authorized cameras; not guaranteed by discovery alone. |
| Vendor-specific / proprietary transports | Not a general compatibility promise. |
| Recording and playback | Planned; not currently presented as a finished feature. |

## Platform notes

| Platform | Status |
| --- | --- |
| **Windows 10/11** | Primary development target. |
| **Linux** | Architectural goal; not yet a verified release target. |
| **macOS** | Not currently a verified release target. |

An accessible camera stream, a working graphics environment, and **FFmpeg available on the system PATH** are required for the current RTSP pipeline.

## Build from source

### Prerequisites

- [Rust and Cargo](https://www.rust-lang.org/tools/install/) using a toolchain compatible with the repository's `Cargo.lock`.
- On Windows, the **MSVC** Rust target and Visual Studio Build Tools with **Desktop development with C++** and a Windows SDK.
- [FFmpeg](https://ffmpeg.org/download.html), including `ffmpeg.exe` accessible from your terminal's `PATH`.
- Network access to an RTSP endpoint you are authorized to use.

Confirm the tools are available:

```powershell
rustc --version
cargo --version
ffmpeg -version
```

### Windows

Clone the repository after publishing it, replacing `OWNER` with its actual GitHub owner:

```powershell
git clone https://github.com/OWNER/vermouth.git
cd vermouth
```

Run the development build:

```powershell
cargo run --locked
```

Or build and launch an optimized executable:

```powershell
cargo build --release --locked
.\target\release\vermouth.exe
```

The current stream implementation starts FFmpeg as an external process. Building `vermouth.exe` does **not** automatically bundle FFmpeg. Distribution packaging and license compliance for external media dependencies must be addressed separately before publishing installers.

## Troubleshooting

| Problem | What to check |
| --- | --- |
| Camera stays offline | Confirm the device is powered, reachable, and configured to expose a compatible RTSP stream. |
| Connection or authentication fails | Check the RTSP endpoint, port, username, password, and camera-side permissions. |
| Camera is discovered but video does not play | Discovery only suggests a candidate device; it does not establish RTSP compatibility or valid credentials. |
| Stream connects but no video appears | Run `ffmpeg -version`, verify the stream independently using an authorized test player, and inspect the app's error state. |
| Video becomes stale or reconnects | Check Wi-Fi/LAN stability, camera load, and whether the RTSP endpoint is still available. |
| PTZ is unavailable | The camera may not expose compatible ONVIF PTZ services, or the supplied credentials may lack movement permissions. |
| FFmpeg is not found | Install FFmpeg and ensure the `ffmpeg` command resolves from the same environment that launches Vermouth. |
| Camera grid looks different after resizing | Vermouth adjusts the layout to the available window space; return to a larger window or use the focused view. |

When reporting a bug, include the operating system, steps to reproduce, and a **redacted** error message. Never share real camera credentials, private stream URLs, or images from private feeds in a public issue.

## Development

Vermouth follows a feature-oriented Rust application structure. The native shell owns page navigation, while camera services and view state have distinct responsibilities.

| Location | Responsibility |
| --- | --- |
| `src/app/` | Application startup, shell, native title bar, and navigation. |
| `src/features/live_view/` | Camera grid, focused viewing, frame presentation, and contextual controls. |
| `src/features/cameras/` | Camera list, configuration forms, and discovery workflows. |
| `src/features/settings/` | Appearance, themes, density, and preferences. |
| `src/features/recordings/` | Recording UI and workflows as they are implemented. |
| `src/features/diagnostics/` | Diagnostics UI and later connection inspection tools. |
| `src/camera/` | Camera state, RTSP stream sessions, lifecycle, and coordination. |
| `src/persistence/` | Saved cameras and application preferences. |
| `assets/` | Icons and bundled appearance resources. |
| `docs/` | Project specifications, security guidance, architecture, and testing notes. |

For implementation details, see [the documentation index](docs/README.md), [architecture notes](docs/ARCHITECTURE.md), [camera protocol notes](docs/CAMERA_PROTOCOLS.md), and [security/privacy baseline](docs/SECURITY_PRIVACY.md).

### Code quality

Run the project's checks from the repository root:

```powershell
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
```

A successful compile is not proof of camera compatibility. Test real connections, stream teardown, PTZ stop behavior, and responsive layouts on the operating systems and devices actually supported by the release.

## Security and privacy

Vermouth is intended for cameras and networks you own or are explicitly authorized to access. It does not need authentication bypasses, firmware exploits, or public Internet exposure to deliver its intended functionality.

- Treat saved RTSP addresses and camera configuration files as sensitive, especially if an endpoint includes credentials.
- Do not assume camera passwords are encrypted at rest unless the particular build has verified secure-secret storage.
- RTSP connections may be unencrypted; use trusted networks and camera-side access controls.
- Keep private camera footage, credentials, device addresses, and unredacted logs out of public bug reports.
- Connecting to a camera does not imply consent to record or redistribute its footage.

## Roadmap

Development priorities include reliable multi-camera streaming, clearer per-camera diagnostics, camera-specific capability testing, secure credential persistence, local snapshots, and eventually recording/playback with predictable storage controls. These are **directions, not release commitments**.

## License

[<img src="https://www.gnu.org/graphics/gplv3-127x51.png" alt="GPLv3" >](http://www.gnu.org/licenses/gpl-3.0.html)

Vermouth is licensed under the **GNU General Public License v3.0**. See the [LICENSE](https://github.com/LucasErrNotFound/Vermouth/blob/main/LICENSE)
file for details.

### What GPLv3 lets you do

- **Run** the software for any purpose
- **Study** the source code and **modify** it to suit your needs
- **Share** copies, and distribute your modified versions

### What GPLv3 requires when you distribute

- Provide the **complete corresponding source code**
- License derivative works under **GPLv3** so the same freedoms are preserved
- Include a **copy of the GPL license** with every distribution
- **State any changes** you made, with a date, in the modified files

This is a plain-language summary, not legal advice. The full license text in the
[LICENSE](https://github.com/LucasErrNotFound/Vermouth/blob/main/LICENSE) file is what governs.

For more information, visit [gnu.org/licenses/gpl-3.0.html](https://www.gnu.org/licenses/gpl-3.0.html).
