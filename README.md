# OpenCDi

OpenCDi is an open source Philips CD-i emulator written in Rust. This fork adds a Windows library and settings interface with configurable CD-i controls, per-disc profiles, artwork paths, and a pause menu.

## Windows download

Download **OpenCDi-Windows-PreviewUI.zip** from the [Releases](../../releases) page and extract it. Run `opencdi.exe`. The archive includes a quick-start guide. Windows x64 is required.

OpenCDi does not include Philips firmware. Place a legally obtained system BIOS named `cdi220b.rom` in `%APPDATA%\OpenCDI\bios`. For FMV titles, place `vmpega.rom` in the same folder. Create a `Portable` folder beside `opencdi.exe` before first launch to keep settings, firmware, artwork, saves, and profiles beside the app instead.

## Interface

With no command-line arguments, OpenCDi opens the disc library. Use the toolbar to open a disc, configure controls, set paths, refresh the library, change display mode, or exit. The toolbar appears in the library. The library grid responds to the artwork-size slider, mouse wheel, keyboard, D-pad, and analog stick.

Press the configured pause binding during emulation to open the pause screen. Choose Resume, Save State, Load State, Controller Settings, or Stop and Return to Library. The default pause binding is F1 or the gamepad North button. CD-i buttons are named **I**, **II**, and **I/II**; I/II sends both buttons together.

See [Quick Start for Windows](docs/QUICK_START_WINDOWS.md) for setup, controls, paths, and per-disc configuration details.

The [screenshots folder](screenshots/README.md) contains the user-provided UI reference images used to guide this interface update. They are design references, not captures from this release build.

## Build from source

Install Rust and Cargo, then run:

```powershell
cargo build --release --bin opencdi
```

The executable is written to `target/release/opencdi.exe` on Windows. The project also builds on Linux; platform development libraries may be required by the window and audio backends.

## License

The project is distributed under the MIT License. See [LICENSE](LICENSE).
