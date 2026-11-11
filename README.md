# GP Guard

**Version 0.1.1** · Windows 10/11 x64 · [⬇ Download gp-guard.exe](https://github.com/alexindacomp/GlobalProtect-Client-guard/releases/latest/download/gp-guard.exe) · [Release notes](https://github.com/alexindacomp/GlobalProtect-Client-guard/releases/tag/v0.1.1)

A small tray app that keeps the **GlobalProtect** VPN client connected. It starts `PanGPA.exe`, stores your profile (the password goes to Windows Credential Manager) and reconnects automatically when the tunnel drops. It uses the installed GlobalProtect client as is and does not replace it.

- Launches with Windows (a Task Scheduler sign-in task plus a startup entry) and runs in the tray. The icon colour shows the VPN status.
- Reconnects after a drop and restarts PanGPA if it hangs. It stops retrying if the password is rejected, so your account doesn't get locked.
- Dashboard with status, a 60‑minute connection history and a log.
- Interface in English, Українська and Русский.

## Usage

1. Download `gp-guard.exe` and put it somewhere permanent, e.g. `%LOCALAPPDATA%\Programs\GpGuard\`.
2. Run it, enter your portal, username and password in **Profile**, and click **Save**.

Settings and the log are kept in `%APPDATA%\GpGuard\`. Run `gp-guard.exe --diag` to write diagnostics about the GlobalProtect window to the log.

## Build

```bash
cargo build --release
```

Rust 1.82+ is required. The output is `target\release\gp-guard.exe`.

## License

MIT
