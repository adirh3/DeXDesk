# Contributor and coding-agent guide

## Goal

Keep DeXDesk a small, portable native Windows receiver for wireless Samsung DeX:
open the EXE, follow the welcome screen, then use DeX with input scoped to the
picture. Preserve the clean window with one settings icon.

## Architecture and invariants

- Use Rust, Win32, Media Foundation and Direct3D 11. Do not introduce .NET,
  Electron, WebView, external media processes, phone-side apps or debug mode.
- `advertise.rs` owns the Wi-Fi Direct publisher and dynamically resolved
  Windows Miracast provider. Do not patch Windows, install drivers, or replace
  the provider with a success-shaped fallback.
- `receiver.rs` owns one phone session, one RTSP connection and one UIBC
  connection. No parallel-input takeover or unbounded media queues.
- Keep Media Foundation, COM and Direct3D objects on their owning thread.
  Pump window messages while workers shut down; do not destroy a video HWND
  before its presentation resources are released.
- Keep RTSP framing, RTP reordering, PES assembly, queues and logs bounded.
  Preserve packet-loss recovery, sequence wrap handling and cancellation.
- Parse the source's selected video bitmap. A requested 60 FPS target is not an
  achieved 60 FPS stream. Do not duplicate frames or claim unmeasured quality.
- Preserve the verified GENERIC pointer plus HID keyboard report format.
  Right-click is a cancellable long-press, and wheel input is one continuous
  contact per burst. Do not relabel these as native HID mouse events.
- Never capture input globally. Release all keys and contacts on picture leave,
  activation loss, settings, disconnect, stop and exit. Keep Windows shortcuts
  and the Ctrl+Alt+Esc release path usable.

## Working on the code

- Read the affected module and its callers before editing. Prefer a small
  direct fix over new frameworks, wrappers or configuration layers.
- Reuse existing helpers and match naming. Use proper Windows types and keep
  unsafe code at narrow FFI boundaries.
- Surface actionable errors in the UI or local diagnostics. Do not silently
  swallow failures or substitute defaults for invalid persisted preferences.
- Keep settings validated and atomically replaced. Preserve compatibility
  unless the user explicitly requests a change.
- Update README compatibility and limitations when observable behavior changes.
  Audio, clipboard, USB and file transfer must not be advertised as implemented.

## Build and verification

This is a Windows x64 project. Use the pinned GNU toolchain and committed
`Cargo.lock`; do not update dependencies incidentally.

```powershell
rustup toolchain install 1.99.1-x86_64-pc-windows-gnu --profile minimal
.\build.ps1 -Test
cargo +1.99.1-x86_64-pc-windows-gnu fmt --all -- --check
cargo +1.99.1-x86_64-pc-windows-gnu clippy --locked --all-targets -- -D warnings
```

Install `rustfmt` and `clippy` with rustup when those tools are needed. Add a few
focused tests for changed protocol, loss recovery or gesture behavior, preferably
in the existing test modules.

The packaged artifact is `dist\DeXDesk.exe`, not the unstamped Cargo output.
`build.ps1` remaps local source paths in dependencies as well as application code,
then embeds native resources and license notices. Its final binary privacy gate
must pass; do not bypass it or publish a stale EXE after a failed build.

Use `--preview` or `--smoke-ui` for UI-only checks without competing with another
Miracast receiver. Test media with generated clips, not recordings of somebody's
phone. Live phone verification is manual and must be coordinated with its owner;
do not enable debugging, capture packets, alter security settings or disconnect
an active session without authorization.

## Privacy and publishing

- Never commit or upload `.tools`, `target`, `dist`, local settings, logs,
  packet captures, dumps, private screenshots, phone content or credentials.
- Do not put personal filesystem paths, machine names, phone identifiers,
  private IP addresses or private contact details into fixtures or documentation.
  Runtime environment lookups are fine; captured values are not.
- Use a public GitHub noreply identity for public commits. Keep internal session
  identifiers and private development metadata out of public history.
- Inspect the actual source tree, reachable publication history, source archive
  and EXE before a release. `.gitignore` and stripped symbols are not a privacy
  audit: source-location strings can remain in binaries.
- Keep release builds path-remapped. Scan ASCII and UTF-16 strings for builder
  paths and sensitive identifiers, inspect debug metadata, and check that imports
  need no extra GNU runtime DLLs.
- Update third-party notices when dependencies or toolchains change. The
  maintainer notice generator reads public license texts from the locked Cargo
  dependency sources; it must not copy personal files or local paths.
- Do not publish, push backup/development branches, create releases, or upload
  assets unless explicitly requested. Push only the reviewed branch and tag,
  never `--mirror` or `--all`.
- Redact diagnostics before posting issues. If a sensitive value is discovered,
  stop publication and remove it from both artifacts and publishable history.
