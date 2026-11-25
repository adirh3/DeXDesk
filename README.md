# DeXDesk

**Wireless Samsung DeX on your Windows laptop, with the laptop's mouse and keyboard.**

DeXDesk is a small, portable Rust application: open one EXE, connect your Galaxy,
and use DeX inside a clean window. One settings icon keeps everything else out
of the way.

[Download DeXDesk for Windows x64](https://github.com/adirh3/DeXDesk/releases/latest/download/DeXDesk.exe)
&middot; [All releases](https://github.com/adirh3/DeXDesk/releases)
&middot; [Report an issue](https://github.com/adirh3/DeXDesk/issues)

## What it does

- Receives wireless DeX directly over Wi-Fi Direct / Miracast.
- Routes input only while DeXDesk is active and the pointer is over the picture.
- Supports clicking, typing, long-press right-click, and continuous wheel scrolling.
- Decodes H.264 with Windows Media Foundation and presents it with Direct3D 11.
- Offers quality targets, scroll speed, always-on-top, actual FPS, and diagnostics.
- Runs without installation, administrator access, phone apps, USB debugging,
  .NET, a browser engine, or a bundled FFmpeg process.

**Early release:** verified with a Galaxy Fold7 running Android 17 on one Windows
PC, and with a Galaxy S22 Ultra on a Snapdragon X Elite PC (Qualcomm FastConnect
7800 Wi-Fi). Other phones, adapters, drivers, and Windows versions may behave
differently.

## Connect in three steps

1. Close Windows Wireless Display or any other Miracast receiver, then open
   `DeXDesk.exe`.
2. Keep the laptop's Wi-Fi enabled, even if its internet connection uses Ethernet.
3. On the phone, open **Quick settings > DeX > TV or monitor**, then select the
   computer name shown on the welcome screen.

Click the picture to use DeX. Input stays scoped to the picture; moving the
pointer away stops forwarding input without changing which window is active.
Windows manages activation normally, so hovering over DeXDesk won't switch
you away from another app. Windows-key shortcuts and Alt+Tab remain local;
**Ctrl+Alt+Esc** immediately releases input.

Use the settings icon for quality, scroll speed, always-on-top, disconnect, and
diagnostics. Quality changes apply on the next connection.

The welcome screen shows whether the receiver is starting, waiting for your
Galaxy, setting up a cast, or waiting for video. Use **Cancel connection** during
setup or **Retry receiver** after a receiver startup error. If the cast times
out, end it on the Galaxy and select the PC again.

## Requirements and limits

| Requirement | Details |
|---|---|
| Laptop | Windows 10/11 x64, or Windows 11 ARM64 (build from source), Wi-Fi Direct-capable adapter and driver |
| Video | Windows Media Foundation H.264 decoder and Direct3D 11 |
| Phone | A Galaxy device supporting **wireless DeX** |
| Windows N | The Windows Media Feature Pack may be required |

### Quality and frame rate

The verified live stream is **1920 x 1080 at approximately 30 decoded FPS**.
The tested phone selects 30 FPS even when 1080p60 or 720p60 is offered. A
**target** is not a guaranteed or forced frame rate; DeXDesk reports the actual
selection and decoding rate. It does not manufacture duplicate frames.

### Input is intentionally scoped

Input is released when the pointer leaves the picture, the app loses activation,
settings open, the phone disconnects, or the app closes. There is no global
keyboard or mouse capture.

The tested phone does not accept a native HID mouse. DeXDesk uses a negotiated
GENERIC pointer and native HID keyboard instead. Right-click is an approximately
720 ms long-press, and wheel input becomes a continuous touch-scroll gesture.
These translations can behave differently from a physical mouse in some apps.

### Not implemented

Audio, clipboard synchronization, USB transport, and file transfer are not
implemented in this release.

The EXE is unsigned. Windows may display an unknown-publisher or reputation
warning. Do not disable antivirus, firewall, or other Windows security controls.

The Windows Miracast advertisement entry point used by this project is
**undocumented**. It is resolved at runtime, errors are shown, and a Windows
update may change its availability. This is not a guarantee of compatibility
with every Miracast source.

## Troubleshooting

| Symptom | What to check |
|---|---|
| PC is not listed | Enable Wi-Fi, close other receivers, and check Wi-Fi Direct driver support. Use **Retry receiver** if DeXDesk reports a startup error. |
| Another receiver is using the PC | Close Windows Wireless Display and any previous DeXDesk instance |
| Cast connects without a picture | End the cast on the Galaxy and try again. Open **Settings → Diagnostics** for the connection stage and any decoder errors. |
| Picture works but input does not | Click the actual picture, not its black borders, and keep the pointer over it |
| Picture is not sharp | Select the 1080p target and reconnect; the phone still controls the negotiated mode |
| Frame rate remains 30 FPS | Check the actual selection in settings; choosing a 60 FPS target cannot force the sender |
| Decoder is unavailable | Check that the Windows Media Feature Pack is installed on Windows N |

Local settings and bounded diagnostic logs are stored in
`%LOCALAPPDATA%\DeXDesk`. Diagnostics can contain connection addresses and device
names. **Redact logs, screenshots, and captures before sharing them publicly.**
Never include phone content, credentials, serial numbers, or personal paths in
an issue.

## Build from source

Install [Rust through rustup](https://www.rust-lang.org/tools/install) and run
these commands from the repository root in PowerShell:

```powershell
rustup toolchain install 1.99.1-x86_64-pc-windows-gnu --profile minimal
.\build.ps1 -Test
```

The resulting portable executable is **`dist\DeXDesk.exe`**. Use
`-OutputDirectory .\dist\preview` to build alongside a running copy. The build uses a
locked dependency graph and the GNU Rust toolchain, so a system-wide Visual
Studio installation is not required.

The packaging step embeds the Windows manifest, icon, version, and license
notices through native Windows APIs. Source paths are remapped for application
code and dependencies so release binaries do not disclose the builder's local
checkout path. Packaging fails if checkout, home, or Cargo-home paths remain in
UTF-8 or UTF-16 binary strings. Environment changes are process-local and
restored when it finishes.

### Windows on ARM (ARM64)

On an ARM64 PC, build a native ARM64 EXE with the `aarch64-pc-windows-gnullvm`
toolchain. It links with clang/lld from
[llvm-mingw](https://github.com/mstorsjo/llvm-mingw/releases): extract the
`ucrt-aarch64` release into `.tools\` (or put its `bin` folder on `PATH`).

```powershell
rustup toolchain install 1.99.1-aarch64-pc-windows-gnullvm --profile minimal
.\build.ps1 -Architecture arm64 -Test
```

libunwind is linked statically, so the EXE needs no runtime DLLs beyond those
supplied by Windows.

### Developer checks

```powershell
cargo +1.99.1-x86_64-pc-windows-gnu test --locked
cargo +1.99.1-x86_64-pc-windows-gnu fmt --all -- --check
cargo +1.99.1-x86_64-pc-windows-gnu clippy --locked --all-targets -- -D warnings
```

Install the `rustfmt` and `clippy` rustup components if needed. The
`media_check` developer binary accepts a **generated** H.264 MPEG-TS clip to
exercise the actual Windows decoder and presenter:

```powershell
cargo +1.99.1-x86_64-pc-windows-gnu run --locked --bin media_check -- .\test-clip.ts
```

`DeXDesk.exe --preview` opens the UI without advertising a receiver;
`--smoke-ui` opens that preview and closes it automatically. Neither option is
needed for normal use.

## Architecture

| Module | Responsibility |
|---|---|
| `ui` | Win32 welcome screen, settings, pointer-scoped input, safe shutdown |
| `advertise` | Wi-Fi Direct discovery and Windows Miracast provider lifecycle |
| `receiver` | Connection ownership, RTSP negotiation, RTP and UIBC workers |
| `protocol` | Bounded framing, negotiated modes, packet encoding and reordering |
| `transport_stream` | MPEG-TS / H.264 PES reassembly and loss boundaries |
| `media` | Thread-owned Media Foundation decoder and Direct3D presentation |
| `input` | Cancellable long-press and continuous-scroll gestures |
| `settings` / `runtime` | Validated preferences, shared state and bounded local logs |

One application owns the complete session and one UIBC connection. Video uses
bounded packet reordering and rate-limited keyframe recovery. COM and graphics
objects remain on their owning thread. See [AGENTS.md](AGENTS.md) for contributor
and coding-agent guidance.

## License and attribution

DeXDesk is [MIT licensed](LICENSE). Third-party notices are included in
[THIRD-PARTY-NOTICES.txt](THIRD-PARTY-NOTICES.txt) and embedded in the packaged
EXE. Windows components are supplied by Windows and are not redistributed here.

This is an independent project, not affiliated with or endorsed by Samsung or
Microsoft. Samsung, Galaxy, and DeX are trademarks of their respective owners.
