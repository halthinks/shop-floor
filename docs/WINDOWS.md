# Windows: build the shop binary and start shop ui

Shop ui is a local process. It is not a cloud-agent myth. CI already builds the Windows release artifact (`shop-windows` from `.github/workflows/ci.yml`). This page is how you build that same artifact on a Windows machine and start it.

Do not invent a module. Do not write `C:\TextPCB Platform`. Incomplete evidence is **WAIT**, never a fake PASS.

## What you get

```text
cargo build --release
```

writes:

```text
target\release\shop.exe
```

That is the runnable Windows binary. The GitHub Actions job **Windows release** uploads the same path as artifact `shop-windows`.

## Prerequisites

- Windows
- [Rust](https://rustup.rs/) 1.83 or newer (`Cargo.toml` `rust-version`)
- `cargo` on `PATH`

Check:

```powershell
rustc --version
cargo --version
```

If either command is missing, install rustup and open a new shell. Do not invent a binary.

## Build the release artifact

From the shop-floor repo root:

```powershell
cargo build --release
```

Confirm the file exists:

```powershell
Get-Item .\target\release\shop.exe
```

If `shop.exe` is missing, that is **WAIT**. Do not pretend the UI is running.

Optional: run tests first.

```powershell
cargo test
```

## Start shop ui

The command center binds `127.0.0.1` and prints the URL. Default port is **7745** (`src/ui.rs` `DEFAULT_PORT`). `shop ui`, `shop serve`, and `shop listen` all start that same local server.

```powershell
.\target\release\shop.exe ui
```

Expected line:

```text
shop ui http://127.0.0.1:7745/
```

Open that URL in a browser. Leave the process running. Ctrl+C stops it.

Port override:

```powershell
.\target\release\shop.exe ui --port 7745
```

If 7745 is taken, shop binds an ephemeral port and prints the real URL. Use the printed port.

### Helper script

`scripts\shop-ui.ps1` starts the local release binary. It does not invent a server and does not build a missing exe.

```powershell
.\scripts\shop-ui.ps1
.\scripts\shop-ui.ps1 -Port 7745
.\scripts\shop-ui.ps1 -Store .shop
```

If `target\release\shop.exe` is missing, the script exits **WAIT** and tells you to `cargo build --release`.

## Optional store and mailbox

Global flags go on the binary, not on a new module:

```powershell
.\target\release\shop.exe --store .shop ui
.\target\release\shop.exe --store .shop --mailbox $env:SHOP_MAILBOX ui
```

`--store` defaults to `.shop` in the current directory. `--mailbox` is optional; otherwise shop uses `SHOP_MAILBOX` or `.shop\mailbox`.

## Download the CI artifact instead of building

On a green `main` or PR run, Actions uploads `shop-windows` = `target/release/shop.exe`. Download that file, then:

```powershell
.\shop.exe ui
```

A missing artifact is **WAIT**. CI test-on-Linux is not a Windows binary.

## What this is not

- Not AASM and not T3 Code
- Not a Platform write
- Not a running worker just because a name is on the roster
- Not a fake PASS if the exe is missing or the process did not print a `shop ui http://127.0.0.1:…/` line
