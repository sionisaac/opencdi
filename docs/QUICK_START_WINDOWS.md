# OpenCDi for Windows: Quick Start

## 1. Download and launch

1. Download `OpenCDi-Windows-PreviewUI.zip` from the fork's GitHub Releases page.
2. Extract the archive to a folder you can write to.
3. Place your CD-i BIOS files as described below.
4. Run `opencdi.exe`. With no command-line arguments, it opens the library.

The build is for 64-bit Windows. BIOS files and game images are not included.

## 2. Add BIOS files

OpenCDi requires a Philips CD-i system BIOS, which must be supplied by the user. Use a legally obtained compatible ROM such as `cdi220b.rom` (512 KB). FMV and Video CD titles also require the DVC ROM `vmpega.rom` (256 KB).

By default, place the files here:

```text
%APPDATA%\OpenCDI\bios\cdi220b.rom
%APPDATA%\OpenCDI\bios\vmpega.rom
```

Create a `Portable` folder beside `opencdi.exe` before first launch to store settings, BIOS, artwork, save states, NVRAM, and per-disc configurations alongside the app. In portable mode, place firmware in `Portable\bios`.

## 3. Add discs and artwork

Use **Open Disc** to select a CHD, ISO, or CUE image. In **Paths**, add one or more disc folders; the library scans their subfolders. Set separate folders for cover, CD, and bezel artwork if desired. Missing art can be looked up from LaunchBox's Philips CD-i database when an internet connection is available.

The library uses a compact grid. Change cover size with the slider; the grid reflows to fit. Move through the library with the keyboard, mouse wheel, gamepad D-pad, or left analog stick. Press Enter or gamepad A to launch the selected disc.

## 4. Controller settings

Open **Controller** from the library toolbar. Choose Gamepad or Mouse input, then select a mapping field to bind a keyboard key or gamepad button. CD-i gamepad buttons are labeled **Button I**, **Button II**, and **Button I/II**. The I/II control presses I and II at the same time and can also have its own binding.

The pause-menu binding is in the button mapping list and defaults to **F1** for the keyboard and **North** for a gamepad. The exit shortcut defaults to **Start+Select**. Each disc can keep its own settings under `%APPDATA%\OpenCDI\Configs` or `Portable\Configs`; otherwise the global settings are used.

## 5. Display and pause menu

The default display mode is borderless fullscreen. Use **Display** in the library toolbar to switch between borderless fullscreen, decorated fullscreen, and windowed mode.

During a game, press the configured pause binding to pause. The pause screen offers:

- Resume
- Save State
- Load State
- Controller Settings
- Stop and Return to Library

Press the pause binding again to resume. Save and Load use the current game's state slot.

## 6. File locations

Without a `Portable` folder, app data is stored in `%APPDATA%\OpenCDI`, including artwork, BIOS, save states, NVRAM, and configs. With a `Portable` folder beside the executable, those files are stored in that folder.

Useful folders:

```text
OpenCDI\Artwork\Covers
OpenCDI\Artwork\CD
OpenCDI\Artwork\Bezels
OpenCDI\Configs
OpenCDI\bios
```

## 7. Build from source

Install Rust and Cargo, then run from the project root:

```powershell
cargo build --release --bin opencdi
```

The Windows executable is `target\release\opencdi.exe`. No BIOS ROM is required to compile the project.

