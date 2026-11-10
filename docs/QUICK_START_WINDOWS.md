# OpenCDi for Windows: Quick Start

## 1. Download and launch

1. Open `OpenCDi-Windows-x64.zip`.
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

Use **Open Disc** to select a CHD, ISO, or CUE image. When Windows detects a readable CD-i disc, the toolbar also offers **Play Disc** with the drive letter. Both open the disc details screen. Choose **Play** with the mouse, press Enter, or press the configured gamepad Button I to start emulation. Use **Back to Library**, Escape, or Button II to leave the preview.

Use **Settings → Add Disc Folder** to scan a folder and its subfolders. Artwork uses the default app-data folders. **Settings → Switch BIOS** lets you choose another system ROM, which OpenCDi remembers for later launches.

The smallest artwork setting shows 10 columns and 4 rows. Disc names appear below the artwork. Covers appear as they download. The library displays the Cover Missing image when no cover is available, and the startup screen uses a generic CD-i disc when it has no disc art. Use the artwork slider to choose a larger size. Move through the library with the keyboard, mouse wheel, gamepad D-pad, or left analog stick. Press Enter or gamepad A, or double-click a cover, to view the selected disc.

### Synopses and ScreenScraper

The app saves a UTF-8 text file for each available synopsis. For example,
`National Parks Tour, A.chd` uses `Synopses/National Parks Tour, A.txt`.
This folder is inside `Portable` when portable mode is active, or inside
`%APPDATA%\OpenCDi` otherwise. You can add or edit these files yourself. The
loading screen displays the synopsis to the right of the spinning disc.

When configured, ScreenScraper is tried first for flat covers, disc images, back covers, and Wheel logos. LaunchBox fills missing artwork and
remains the first source for synopses, which are saved before artwork downloads. The public ScreenScraper gallery works without credentials for known CD-i titles. The new image folders under Artwork are
Boxart, Back and Logo. Boxart is generated locally from the front cover. The library uses logos
below covers where available. The loading screen and pause Info show the disc,
3D case, back cover, logo or title, and synopsis where available. To enable it, set the following
Windows environment variables for your account and restart OpenCDi.

- `OPENCDI_SCREENSCRAPER_DEV_ID`
- `OPENCDI_SCREENSCRAPER_DEV_PASSWORD`
- `OPENCDI_SCREENSCRAPER_USER` and `OPENCDI_SCREENSCRAPER_PASSWORD` (optional account credentials)

Use developer credentials issued by ScreenScraper for this app. See the
[ScreenScraper API documentation](https://www.screenscraper.fr/webapi2.php).

### Search the library

Click **Search library** or press **Ctrl+F**. Type part of a disc title or filename
to filter the grid as you type. Search ignores case and punctuation. Press Enter,
Tab, or Down to return focus to the results. Click the × or press Escape to clear
the search. The usual keyboard, controller, and mouse navigation works on the
filtered results.

### Missing media report

OpenCDi writes `Missing Media.txt` beside `opencdi.exe` when it scans the
library. It lists each incomplete disc and its missing front cover, disc
image, 3D box art, back cover, clear logo, or synopsis. The report refreshes
automatically as downloads arrive. The app needs write access to this folder.

### App updates

At startup, OpenCDi checks the project's GitHub releases for a newer stable
Windows build. Choose Yes to download, verify, and restart, or No to keep
using the current version. The updater replaces only `opencdi.exe` and leaves
a copy of the old executable in a sibling `.opencdi-update-*` folder.

For release maintainers, publish a stable release with a numeric version tag
such as `v0.2.1` matching the version built into the app. Attach
`OpenCDi-Windows-x64.zip` with `OpenCDi/opencdi.exe` or `opencdi.exe` inside.
The updater requires the asset's GitHub SHA-256 digest.

## 4. Controller settings

Open **Controller** from the library toolbar. Choose Gamepad or Mouse input, then select a mapping field to bind a keyboard key or gamepad button. CD-i gamepad buttons are labeled **Button I**, **Button II**, and **Button I/II**. The I/II control presses I and II at the same time and can also have its own binding.

The pause-menu binding is in the button mapping list and defaults to **F1** for the keyboard and **North** for a gamepad. The exit shortcut defaults to **Start+Select**. Each disc can keep its own settings under `%APPDATA%\OpenCDI\Configs` or `Portable\Configs`; otherwise the global settings are used.

## 5. Display and pause menu

The default display mode is borderless fullscreen. Use **Settings** in the library toolbar to switch between borderless fullscreen, decorated fullscreen, and windowed mode.

During a game, press the configured pause binding to pause. The pause screen offers:

- Info
- Save State
- Load State
- Controller Settings
- Stop and Return to Library

Press the pause binding again to resume. Info opens the loading-screen artwork and synopsis while the game stays paused. Press the configured CD-i Button II to go back from Info, Controller Settings or About. Save and Load use the current game's state slot.

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


## Artwork progress

The bottom of the library shows the current download phase, discs checked,
images saved, a progress bar, and the title being checked. Covers are checked
first. The current library page is prioritised when a lookup starts.
Each disc tries ScreenScraper before LaunchBox for missing artwork.
The progress bar runs separately for covers and other media. A completion
summary stays visible for 15 seconds.

## Controller settings and NVRAM

The gamepad button used to open Controller Settings must be released before
a new press can start assigning a binding. This applies from the library
and the pause menu. Use the configured CD-i Button II to go back.

Per-disc NVRAM is now the default for new settings. An explicit NVRAM mode
saved in an existing configuration continues to apply.

## Generated 3D box art and square covers

OpenCDi generates 3D case artwork from each available front cover using the
approved black CD-i jewel-case template. The case has a transparent background
and no sideways title text. Generated images are saved in Artwork/Boxart with
a --generated.png filename and are refreshed if the cover changes.

3D box-art downloading is disabled for ScreenScraper and LaunchBox, including
supplemental artwork downloads. The loading and Info screens use the locally
generated cases.

New cover downloads from both sources are saved as 600 x 600 PNGs. Existing
downloaded covers in the managed Covers folder are normalised during the
background artwork pass. Rectangular covers are resized to a square.
