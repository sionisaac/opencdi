# OpenCDi v0.2.4

## Interface and input

- Floating docks, outline icons and matching category tabs, controller settings and pause controls. Controller device photographs remain available.
- Redesigned disc info: original transparent case artwork and back cover on the left, logo, synopsis and disc on the right, and a floating Play/Back dock.
- One left click on a library disc opens details. Play starts emulation separately. Press the mouse wheel to open pause, and right click or choose Resume to return to gameplay.
- Physical mouse, dedicated gamepad and Wiimote remain enabled together. F9 device switching is removed. Motion follows deliberate use of a device and buttons are combined without an idle device cancelling another device's press.
- Wiimote A/B assignments are unchanged. The Windows pointer is hidden during Wiimote gameplay and shown in app menus. Pausing releases physical mouse confinement.
- Start 220 and Start 490 boot the selected system without a disc. The configured pause key/button, Wiimote Home or mouse-wheel press returns directly to the library from a standalone BIOS. Escape and existing back controls remain available.
- Settings offers BIOS 220 and BIOS 490 choices with a tick beside the active system. About shows the package version and an automatic build identifier.

## Startup, media and saves

- Library artwork loads in the background. Accelerated BIOS startup runs in short batches between screen updates and returns to normal speed at disc-program handover, with a timeout if startup stalls.
- The disc is stationary before Play and uses wall-clock rotation during loading and paused disc info.
- Box-art and CD filenames use the original disc name. Legacy generated-art suffixes remain readable and conflicting files are preserved.
- The firmware directory is named BIOS. Supply your own system and DVC ROMs: cdi220b.rom with vmpega.rom, or cdi490a.rom with impega.rom.
- Per-disc NVRAM is stored in NVRAM/220 and NVRAM/490. Save states are stored in Savestates/220 and Savestates/490. These directories are under Portable when portable mode is active, otherwise under the normal OpenCDI app-data directory.
- Shared legacy NVRAM is copied only into the board matching its RAM size. Existing destination files take priority and source files are preserved. Legacy save states must pass their BIOS/disc/DVC identity and checksum checks before import.
- The desktop clock starts with PC-local date and time and advances at wall-clock speed. Restoring a state retains its emulated time. Headless validation keeps its deterministic clock.

## Emulation and diagnostics

- Mono-IV / 490 board, audio-buffer completion timing and pointer-report corrections, including the fixes tested with Dragon's Lair, Space Ace and Burn-Cycle.
- The 7th Guest video-start interrupt correction that allows progress beyond the first room.
- CD data/audio filtering, disc metadata handling and BIOS hardware-contract diagnostics. Firmware inspection tools are read-only; no replacement BIOS or Philips firmware is included.
- Preserve the established relative player-shell Play sequence while using absolute gameplay pointer reports where needed.

## Validation and limitations

194 automated tests pass: 144 library and 50 app tests. Five optional diagnostics are excluded from the normal run; the interface-render diagnostic was run separately and its new disc-info output inspected using original local artwork. Both BIOS startup checks passed and replayed video/audio matched with unchanged state resaves.

The user confirmed combined mouse/gamepad/Wiimote operation with both BIOSes and both system launch buttons. The newest BIOS-return shortcut, PC clock display, save-folder workflow and UI behavior still need broader live testing. The 490 board remains experimental; these checks do not establish compatibility with every disc or completion of every game.

Save-state format is now version 13. States from older formats are rejected rather than converted; retain the matching older build if needed. Per-disc NVRAM files are a separate format and are preserved during migration.

## Install

Close OpenCDi, extract OpenCDi-Windows-x64.zip and run opencdi.exe. For an existing installation, replace the executable while retaining the Portable folder. Keep a copy of the previous executable when comparing behavior. The package contains no BIOS ROMs, disc images, personal settings or saved games.
