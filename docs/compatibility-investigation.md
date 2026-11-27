# Compatibility investigation — 17 November 2026

Input: the user's `CD-i BIOS compatibility.xlsx`, tested using
`opencdi-multidisc-test.exe`. The workbook is unchanged. Its Games sheet has
142 annotated/result rows for BIOS 220 and 141 for BIOS 490, including rows
whose Compatibility cell still says Untested but whose Notes record a failure.
The other category sheets and All sheet have not been populated with these tests.

## First confirmed defect: interrupted pointer reports on 490

IKAT port B replaced its unread reply on every new host pointer update. An
update arriving after the BIOS had consumed one to three bytes of a four-byte
report replaced the remaining bytes with a new packet. A quick press followed
by release before the BIOS read it could also erase the press.

Controlled tests reproduced this on the baseline. Pointer reports now use the
existing saved serial FIFO, preserving partial reports and button transitions.
Complete unread motion reports can be coalesced; relative coalescing preserves
distance. A bounded overflow policy retains the in-flight packet and latest
state. No saved-state fields or format versions changed. No ROM or NVRAM files
were modified by the checks.

Validation: 205 automated tests passed (five optional tests ignored). Real local
BIOS pointer-service contracts passed 32 checks for 220 and 32 for 490, each
covering PAL and NTSC. This establishes protocol correctness for those cases;
it does not establish that the 69 recorded 490 control failures are all fixed.

Test executable: `opencdi-490-controls-test.exe`, beside the user's other apps.
Next live checks: Tetris/Battleship directional behaviour on 490, then Burn-Cycle
menu selection. Check a known-working 220 title as a regression comparison.

## Remaining work

* Controls marked Issues: 69 BIOS-490 rows, 16 BIOS-220 rows. Compare live results
  before making further input routing changes; relative versus absolute handling
  remains a hypothesis, not a confirmed diagnosis for the remaining failures.
* Loading overlay: 12 rows (five on 220, seven on 490). Startup handover currently
  controls both the overlay and input gate. Reproduce affected titles before
  changing detection or assuming a true boot failure.
* Peacekeeper: six titles have calibration notes; several report delayed shots.
  The user identifies version 0.2.3 as the last working calibration baseline.
* Dirty-disc messages: five 490 titles. Inspect emulated read/timing failures;
  the on-screen message does not establish damaged disc images.
* Creature Shock: user reports a black screen from Disc 1 to Disc 2 and an
  apparently working reverse transition. Determine whether the title expects a
  save and fresh boot rather than hot swapping before changing the drive logic.
* Other audio, boot and freeze reports remain open. Do not mark workbook rows
  fixed without title-specific confirmation.

This test build has not been published. The earlier requested multi-disc preview
publication also has no verified completion in this work session.

## Live feedback and scoped motion profiles — 18 November

The user confirmed Burn-Cycle's menus and gameplay were usable in the first
packet-fix build. Tetris and Battleship remained excessively sensitive.

The subsequent motion build improved Tetris and Battleship with physical mouse
and gamepad, but regressed Burn-Cycle menu selection. Battleship's Wiimote
pointing remained too sensitive because that route still sent absolute reports.
That build is not a replacement baseline.

The revised build limits relative motion to resolved Tetris and Battleship title
identities for every input source, including Wiimote pointing. Other games,
including Burn-Cycle and the gun titles, use exactly the prior absolute delivery
path. Saved device choices and button mappings are not changed. A held direction
continues supplying small relative motion at a host coordinate edge and supplies
none when released. Regional-name and non-matching-title tests guard the scope.

The real 490 pointer-service contract now tests repeated PT_Pos recentring with
small relative moves, button changes, held-edge motion and stopping on release
in PAL and NTSC. Live Burn-Cycle and Battleship-Wiimote checks remain required.


## Automated investigation — 18 November, first full boot pass

Later live feedback supersedes the initial Burn-Cycle confirmation: its menu also
fails in the earlier controls test. Tetris and Battleship (including Wiimote
pointing) were confirmed improved by the user; Alice in Wonderland remains a
reported regression. Neither Burn-Cycle nor Alice should be marked fixed merely
because their application boots.

The new `compatibility_probe` and `tools/run_compatibility.py` provide isolated
runs from the extracted workbook. They read the selected BIOS, image and optional
NVRAM into disposable machine instances. They do not attach user save paths or
write the workbook, settings, original NVRAM or original savestates. Snapshots,
audio levels, program counters and drive positions are observations, not
playability verdicts. A fixed clock makes these runs reproducible.

The boot pass covered all 102 distinct problem/annotated titles in the Games
sheet, one available disc-image edition per title, on both BIOSes: 204 runs of
1,800 emulated fields each. There were no probe failures or CPU double-fault
halts. Six did not satisfy the old handover heuristic: Arcade Classics/220,
Escape from CyberCity/220, Golgo 13/220, Master Labyrinth/220, Flashback/490 and
Who Shot Johnny Rock/490. Screenshots show that several already display their
application. This is not evidence that these games are playable.

### Startup detection

The player status word can remain at its menu value after the disc application
starts. The new fallback requires actual user-mode execution inside a bounded,
header-validated OS-9 program whose name matches the disc label's initial
application. The application path is read from the Green Book III.2 label field
(bytes 575–702, one-based). DVC/player code and arbitrary RAM addresses are not
accepted. It does not infer readiness merely from elapsed time.

The frame-end fallback alone fixed two cases. A bounded, host-only observation
of early user-mode traps also recognises short bootloaders that run and exit
between frame samples. These observations do not alter guest state or timing,
are excluded from savestates, and are cleared on reset, disc load and state
restore. Directory lookup additionally resolves differing file/module names,
using the Green Book directory attribute bit rather than the ISO-9660 flag.
The bounded reader supports at most eight path components, 64 KB per directory
and a module name contained in the first Form-1 program sector; unsupported
layouts retain the label-name fallback.

All six reproduced misses now register handover: Arcade Classics/220 (534),
Escape from CyberCity/220 (537), Golgo 13/220 (604), Master Labyrinth/220 (648),
Flashback/490 (1226), and Who Shot Johnny Rock/490 (1231). This fixes the
reproduced loading/input gate, not every gameplay failure for these titles.
Mad Dog McCree still boots with both BIOSes in the targeted regression check.

### Burn-Cycle/490: audio restart, not another pointer profile

A repeatable fresh-boot script reaches the menu, moves to New Game and clicks.
The unchanged core leaves it stuck; the modified core highlights New Game,
starts a new disc read and proceeds through the opening movie to the in-game
terminal. Both 220 and 490 receive pointer button presses in BIOS PT_Coord;
changing pointer sensitivity or trying all vertical menu positions did not
resolve the 490 failure. The input routing is therefore left unchanged.

CIAP ignored a START (APCR 0x0140) while `audio_playing` still described an older
stream. A newly queued stream could inherit the old completion timer and strand
the application's audio/UI progress. START now re-establishes the initial
completion interval for the queued stream. A regression test demonstrably fails
with the old guard (13 periods retained instead of four), and passes after the
change, while ensuring the replacement does not complete early.

An already-stuck checkpoint did not recover just by changing the core: the new
START had already been lost. Validation and any user retest must use a fresh
launch, not a frozen old state. This does not certify full-game completion.

The separate scripted-input pass applies repeatable centre presses and a pointer
sweep only after startup handover. It preserves the current Tetris/Battleship
relative profile and keeps other titles on their previous pointer route. Intro
progress or image differences alone must never be promoted to a gameplay pass.


Final validation: 153 library tests and 58 app tests pass (five optional app
checks remain ignored), and the real BIOS pointer contracts pass all 32 checks
on 220 and 42 on 490. Eighteen focused scripted runs cover nine titles on both
BIOSes: Alice, Battleship, Burn-Cycle, Dragon's Lair, Dragon's Lair 2, Mad Dog
McCree, Space Ace, Tetris and The 7th Guest. No CPU halts were observed; screenshots
were inspected for the ReadySoft titles, Tetris/Battleship and Burn-Cycle. The
fresh combined build also passes the targeted Burn-Cycle New Game sequence.

The standalone test executable is named `opencdi-compatibility-test.exe`.
No stable executable, original saves, workbook or saved controller preferences
were replaced. The physical feel of Wiimote aiming, complete games, remaining
Alice reports, dirty-disc reports and multi-disc transitions remain unverified.
The additional Atlantis script reached its attract/demo gameplay on both BIOSes;
that does not establish a successful manual New Game or resolve its dirty-disc
report. No GitHub publication was performed for this investigation.


Both broad passes are complete: 204 boot-only runs and 204 scripted-input runs,
with zero probe errors or CPU halts. The longer scripted baseline pass (before
the audio-restart and full boot-evidence fixes) still missed two handovers. This
is a diagnostic baseline, not a claim that all 102 titles are now playable.
Reports and per-case configurations remain separate from the workbook.

### Issue-level follow-up: current limits and drive-position fix

The user's live results supersede any inference of a general Burn-Cycle fix
from the fresh scripted sequence above. New Game worked after the user removed
the old NVRAM. The original failure remains NVRAM-dependent and unresolved;
neither corruption nor a complete input fix has been established. Original saves
and the user's workbook are not modified by these probes.

Autoplay is explicitly deferred. The issue ledger retains 131 annotated problem
rows across 102 titles; broad no-halt runs do not close their recorded faults.

CIAP C4 streaming reads previously disabled Q-position reports. With that code,
a real 490 BIOS asynchronous seek finishes with ASY_Stat 0x8001 (error). Keeping
the Q locator active makes the same seek finish with 0x0001 (success), preserving
the sector payload. A synthetic Q/data regression fails before and passes after.
Real 490 PAL and NTSC checks pass three seeks, 42 file checks each, and linked
streaming-buffer checks. The test suite passes 154 library plus 58 app tests,
with five optional app tests ignored.

Cold targeted runs under 490 now reach Flashback's language menu and opening
movie, and Master Labyrinth's language menu. This resolves the reproduced early
black-screen barrier in those runs, not later gameplay. Longer L'Affaire Morlov
runs reach language selection and the title movie without establishing full
compatibility. Secret Mission still remains black with saved and clean NVRAM;
the equivalent 220 sequence reaches the bedroom.

Secret Mission's current diagnostic wait is for application event 1005, with
video signals continuing. Its completion request returns successfully, but the
requested buffer/event remains pending after the drive stops. This is an open
lead, not a reason to force a guest signal or declare a fix. Diagnostic tracing
and register observation are confined to the disposable probe. No new app
executable or GitHub release contains this latest change yet.

### IMPEG audio clock: Christmas Crisis/490

Tracing the playback-control block identifies a real buffer-full failure:
ciapdriv finds the next MPEG-audio PCL still full and records E$Read (0xF4).
The `/ma` firmware polls the audio clock low word before its high word. The
model refreshed the low word only on a high-word read, so an old value from
the previous stream/idle interval was returned first, followed by a lower
current value. This could make the firmware's observed clock run backwards.

The IMPEG low-word access now refreshes the low clock word independently.
The focused regression demonstrably returns zero instead of 0x5678 before
the correction and passes after it, including a subsequent advancing read.
VMPEG retains its previous high-word read behavior. Host-only optional audio
register/DMA tracing is excluded from savestates and is off by default.

Christmas Crisis/490 reaches its Start menu instead of the reproduced dirty-disc
screen, and selecting Start reaches opening gameplay. This is a verified early
failure correction, not a complete-game result. A fresh full menu-to-Race sequence
still fails in Video Speedway. Atlantis still displays its dirty-disc error;
Secret Mission still remains black. Burn-Cycle still reaches its menu and
Dragon's Lair still advances beyond Start in the limited regression scripts;
their previously reported later/NVRAM-dependent problems are not closed.

Two diagnostic hypotheses were rejected: clearing the audio-arm status after
decode did not resolve the three targeted failures, and early refill-ready
signalling did not resolve Atlantis or Video Speedway. Neither change was
applied to the production device model. No autoplay changes were made.


### Controller-isolation test build and shared input paths

User priority is now all recorded control faults, including The 7th Guest.
The original workbook remains unchanged; a shared change is not a title-level
pass until its recorded failing step is checked.

The controller screen adds independent Gamepad, Mouse and Peacekeeper gameplay
switches and a physical-mouse sensitivity slider from 25% to 150%. Preferences
and the preferred controller are saved in each disc's existing config. From the
library these controls edit defaults. Menu/pause navigation stays available
when gameplay methods are disabled. Pictures are retained. Keyboard G/M/P toggle
methods and minus/plus adjust sensitivity while this screen is open.

Ordinary gamepad movement uses the upstream relative path, preserving guest
PT_Pos recentering. Normal mouse aiming keeps its previous path, including the
established Tetris/Battleship motion profiles. Peacekeeper preferences restore
the earlier v0.2.3 reporting route; the Wiimote mapping/calibration source is
unchanged from that release. This is a candidate compatibility correction, not
proof of live gun calibration or all 93 annotated control/calibration rows.

The [upstream mouse report change](https://github.com/sigsantilio/opencdi/blob/main/src/slave.rs)
asserts CH0 input interrupts immediately. It is included with tests for a queued
button release and deferred command replies. The extra local IKAT safeguards for
partial reports/button edges are retained rather than replacing the input core.

Validation: 160 library and 58 app suite tests passed, then the additional
controller hit-target test passed. All eight input combinations persist and
filter disabled buttons correctly. Real BIOS pointer checks passed 32 cases
on 220 and 42 on 490 across PAL and NTSC. The native controller layout was
rendered and inspected. These are mechanical checks, not full gameplay verdicts.

Separate executable: `opencdi-controller-isolation-test.exe`, copied beside the
existing E-drive apps with a verified hash. The stable executable, original
NVRAM and workbook were not replaced. Atlantis, Video Speedway and Secret
Mission remain unresolved; autoplay remains deferred. No GitHub publication.


### Three-title follow-up: separate failure paths

 Isolated core runs; the running controller test app was not replaced. All three faults remain open.

| Title / BIOS | Reproduced failure | Evidence | Proposed correction to investigate |
|---|---|---|---|
| Atlantis / 490 | Dirty-disc screen | Firmware reports E$Read (F4) when the next Play Control List buffer is still marked full. Error-word write observed at firmware PC 4289EE; disc position ends at LBA 4814. | Trace buffer ownership from sector delivery through guest consumption and MPEG transfer. Correct the missing release or incorrect delivery pacing at its source; do not hide the overrun or discard sectors. |
| Video Speedway / 490 | Kart Race → one player → car colour → Race reaches dirty-disc screen | No F4 error-word write in the failing Race trace. Sectors 262971–262977 are delivered before the drive stops, including audio, data and video submodes. | Trace the game's actual error branch and play-control completion status around the stop. The cause is not yet established, so reusing Atlantis's presumed fix would be unsupported. |
| Secret Mission / 490 | Black screen | Same application request (SetStat 37) under both BIOSes. BIOS 220 delivers signal 1005 and proceeds; 490 returns from the request but the expected signal does not reach the waiter. Video signals still occur. The exact service meaning remains unconfirmed. | Identify the request's firmware dispatch, then implement its missing CIAP completion/state transition at the device level. Do not inject a game-specific signal or fabricate successful playback. |

## Rejected experiments

The IMPEG low-clock correction helps Christmas Crisis but does not resolve these three. Early refill-ready signalling did not resolve Atlantis or Video Speedway. Clearing the audio-arm status did not resolve the targeted failures.

A private diagnostic binary additionally suppressed the CIAP trigger/EOF marker IRQ bits. Atlantis and Video Speedway still displayed their dirty-disc screens; Secret Mission remained black. The production source was restored byte-for-byte after compiling the private binary. This hypothesis is rejected as a standalone correction.

Evidence: `outputs/issue-tests-disc-read/atlantis-error-origin`, `video-error-origin`, and the three `*-no-marker-irq` directories; paired Secret Mission traces are under `outputs/issue-tests-black-screens/secret-220-paired-service-trace` and `secret-completion-trace`.

## Acceptance requirements

A correction must get past the reproduced failing step, then pass the real-BIOS disc/pointer checks and cross-check Christmas Crisis, Burn-Cycle, Dragon's Lair and the other affected titles. Boot detection and lack of a CPU halt are not gameplay passes. Saved NVRAM and the original compatibility workbook remain untouched. Autoplay remains deferred.


## Follow-up experiments — 18 November

### Working comparisons now exercised

- Video Speedway / 220: the same Kart Race → one player → car colour → Race sequence reaches a rendered race at frame 10000. Evidence: `issue-tests-disc-read/video-220-paired-race`. The corresponding 490 sequence reaches the dirty-disc screen. Both versions request pause (33) and continue (37), but the 490 run returns to the error path shortly after continue. There are no OS-9 carry-error returns in the captured 490 sequence. Its PCB ends at status 0072 (video, real-time, Form 2, trigger), rather than an error-marked status. This rules out calling it the same proven F4 overrun as Atlantis.
- Atlantis / 220: a paired cold run reaches the main menu by frame 4200 (`atlantis-220-paired-intro`). Its trace advances to later play requests; the 490 run aborts during the earlier stream after the F4 buffer-full error. The next useful trace must identify the full PCL and its producer/consumer, before frame 3206, rather than suppressing that error.
- Secret Mission: firmware request dispatch identifies SetStat 37 as the continue/resume path. It restarts a paused stream using a seek/pre-roll sequence under 490. The expected application signal is absent after that sequence; this does not mean request 37 itself takes an asynchronous-status block. The A0 memory printed with the call is not sufficient to identify the service.

### Padding and pipeline experiments — not production fixes

1. Rejecting all sectors without video/audio/data type bits while selection is active moves Secret Mission past the first black wait to the Philips Media intro and produces audio (peak 0.842). A 9000-frame run then remains on that logo. It does not fix Atlantis or Video Speedway. Therefore this broad filter is not accepted: it can lose marker/completion events required later.
2. Filtering only while acquiring the first typed sector fails to boot correctly in a cold Secret Mission run. A real-time-only variant instead halts the CPU at PC zero. Both variants are rejected.
3. Suppressing the first data delivery after selection starts leaves Video Speedway's dirty-disc failure and disrupts cold boot in Secret Mission and Atlantis. This blanket one-sector delay is rejected.

These experiments localize the sensitivity to selection/restart and marker handling, but do not establish the correct hardware semantics. Empty-sector definitions were checked against the supplied specification, Chapter II.4.9.1; that definition alone does not justify dropping EOF/EOR notifications. The correction must preserve raw reads, selected data and valid completion markers.

All experiments used separate private probe binaries. The app source was restored byte-for-byte after each compilation and the running controller test executable was not replaced. The original workbook and NVRAM were only read. The release probe in Cargo's build directory may represent the last diagnostic compilation; rebuild from restored source before packaging anything.

No title is marked fixed by this follow-up. Autoplay was not changed.


# 490 disc restart test — 18 November 2026

Close the current app and open `opencdi-490-disc-resume-test.exe` beside your other E-drive test apps. Select BIOS 490 and launch each title fresh from the library; do not load an old frozen savestate.

## What to check

- Video Speedway: Kart Race → one player → car colour → Race. The isolated fresh run now reaches a rendered race with an advancing timer instead of the dirty-disc screen.
- Secret Mission: launch and allow the intro to run. The fresh run now reaches the first bedroom with sound instead of the black screen.
- Atlantis: remains unresolved and still produces the dirty-disc error. No claim of a correction for this title.

These are automated reproductions of the original failing steps, not full-game compatibility verdicts. Live controls and later gameplay still need checking. The build retains the current controller switches, mouse sensitivity, seven-title crosshair, Thunder/Chaos pointing-button swaps and NFL multipack rule. No new autoplay changes are included. Existing executables, NVRAM and compatibility workbook were not changed by this correction.

## Correction and evidence

The 490 pause handler compares the two CIAP sector headers to find where to resume. Previously the alternate header still described an earlier sector, causing repeated payloads after continue. The CIAP model now makes a validated next-sector header available while retaining the current payload, sector position and interrupt timing. It does not skip game sectors, inject signals or change the BIOS.

A new real-BIOS pause/continue test compares payloads directly against disc sectors. Before the correction, 490 repeats earlier payloads while 220 continues in order. Afterward, both pass. PAL/NTSC 490 seek, linked-buffer and resume tests pass, as do 220 PAL/NTSC resume checks (eight real-firmware runs). All 161 core tests and 63 app tests pass; five optional app checks remain ignored. The added unit test checks that header read-ahead neither overwrites the next payload early nor counts/delivers an extra sector or interrupt.

Limited regressions: Burn-Cycle reaches its menu with sound and Dragon's Lair advances into opening gameplay using the existing scripts. This does not close Burn-Cycle's historical NVRAM-dependent issue or other later gameplay faults.

SHA256: 998366A384771C10401727EE00EF998EE9AD7309B72765F021066D134BA2BC50

Atlantis follow-up: a watched video PCL is returned by fmvdrv at field 3083, filled again at field 3202, and the ring overruns at field 3206. Therefore this is not simply a buffer that was never released. The next trace targets video transfer/completion across that new record.

At field 3195, before the overrun, Atlantis has decoded/presented 292 frames; the video decoder is enabled and armed but playback is stopped. Both MPEG input FIFOs are empty, FIFO-ready status is asserted, and the displayed picture-queue register still says three. This does not support FIFO-full as the cause. The remaining lead is video end/re-arm/completion across the next record; no change to that logic has been made.

## Atlantis boundary-marker correction — 19 November 2026

The 490 failure is caused by channel filtering of a file boundary, not an MPEG decoder reset. The previously recorded video-PCL overrun is a downstream symptom.

The failing stream reaches an EOF/EOR data sector at LBA 4805 (file 1, channel 31, submode C9). CIAP rejected it because channel 31 was outside the selected channel mask. Streaming therefore continued into the following file. Atlantis then received video buffers while the video play was waiting for its corresponding audio play, and the circular video buffers overran. The working 220/CDIC path preserves the file-boundary sector.

The supplied May 1994 specification, Chapter VII.4, page VII-266 (PDF page 576), explicitly requires EOF and trigger interrupts after file selection only; EOR requires both file and channel selection. CIAP selection now preserves EOF/trigger sectors belonging to the selected file regardless of channel. Foreign-file markers remain rejected and EOR alone remains channel-filtered. No title-specific override, BIOS patch, sector skip, guest-memory modification or forced success signal is used.

A disposable candidate retaining all three marker types initially passed the former error; the accepted correction is narrower (mask 90, EOF/trigger only). The final cold run reaches the menu and animated gameplay demo. Scripted New Game reaches difficulty selection. The separate New Game-to-gameplay check is recorded in the test-build guide after completion.

Validation: 162 core tests and 63 app tests passed, five optional app tests ignored. A new synthetic-disc test exercises actual delivery/interrupts for foreign EOF, unselected-channel EOR, trigger, EOF and ordinary data. Eight real firmware contracts passed: 490 seek, buffer and resume in PAL/NTSC, and 220 resume in PAL/NTSC. Secret Mission reaches the bedroom; Video Speedway reaches a rendered race with advancing timer; Burn-Cycle and Dragon's Lair match their previous limited checkpoints. None of these observations certifies full-game completion.

Evidence: task workspace outputs/atlantis-boundary-validation. User NVRAM, saves and the original compatibility workbook were not modified. Autoplay remains deferred.

Final Atlantis follow-through: the cold run selected New Game and Easy, started the story with audio, then its disposable continuation reached the first playable level without the demo label. Evidence: atlantis-boundary-validation/atlantis-playing and atlantis-first-level. Live controller feel and full completion remain unverified. Separate executable: opencdi-atlantis-test.exe, SHA256 B5CDEB6DD157E439FF2DE4F9F92B03D2A53C33231A3C22DD73846C199444C1F7.

A subsequent scripted controller input changed the first-level view (atlantis-movement before/after screenshots). The completed test executable and guide were copied beside the existing apps on E:, with executable hashes verified identical. No running GUI was interrupted.

## Burn-Cycle reopened — user live test, 19 November 2026

The user reports intermittent main-menu input failure in the latest test build: New Game sometimes cannot be selected, returning to the library and relaunching may make it work, another relaunch may fail again, and some launches respond only after a delay. This title must not be classed as reliably working. The earlier successful fresh-NVRAM run is insufficient; deleting NVRAM is not an established remedy for this repeated-launch fault.

Acceptance must cover consecutive launches through the actual app's library-return path, preserving the same BIOS, settings and a disposable copy of the user's NVRAM across sessions. Include immediate and delayed menu presses, first click and repeated presses, and compare cold process startup with relaunch within one process. Do not overwrite or delete the user's original NVRAM. A single headless menu success does not close this issue.

Initial code review confirms PlaySelectedDisc performs a full bus/CPU reset and reactivates per-disc NVRAM after return to the library. The app also keeps host-controller state outside the emulation core. No causal diagnosis or new correction has yet been established. Workbook unchanged in this review.

### Burn-Cycle diagnostic follow-up (still unresolved)

Disposable tests are in the task workspace `outputs/burn-repeat-investigation`. Three cold/restarted sessions, settled-menu button tests and a fresh untouched boot were recorded. New Game has not been verified in these runs. The live isolated frontend also showed a mismatch between the host and green game cursors. Coordinate automation with captured mouse input is not sufficient evidence of a game fix.

The 490 firmware trace receives complete press/release reports and returns PT_Coord status 8001/8000 with the requested coordinates. The game's own callback queues and consumes the button messages. Its pointer handler limits motion to 24 pixels per report and can issue PT_Pos, so jumping directly to a menu coordinate is not a reliable click test. Gradual movement also failed to demonstrate New Game. Neither a packet-active-bit experiment nor forced repeated absolute reporting established a correction; the active-bit experiment was reverted.

The copied failing NVRAM contains `player_shell_settings.prf` and `csd` device configuration, not an identifiable Burn-Cycle save. Consequently, the user's deletion workaround must not be described as proof of a corrupt game save. A fresh-NVRAM scripted run reached a Restore dialog, which demonstrates a difference but is not a New Game pass. Original NVRAM files have not been deleted or overwritten.

Temporary instruction/stack instrumentation was removed after recording the traces. The probe retains optional restart replay and NVRAM export inside its diagnostic output directory. No new Burn-Cycle product correction or replacement executable is accepted. Further investigation needs to explain why the failing menu receives clicks without activating its menu action; do not close this on a BIOS input contract or screenshot of the player menu.

### Burn-Cycle / 490 first-door stall — user state, 19 November

The user supplied a saved state after shooting the initial enemy and passing the first door. A read-only copy is kept in the task workspace `work/burn-doorway-original.state`; the original slot is untouched. The saved picture matches the supplied corridor/HUD screenshot. A 1500-frame continuation leaves the image and disc position unchanged (LBA 10387), without a CPU halt or ongoing CIAP transfer/audio. This is not proof of a missing disc completion.

The VBL signal and game event loop continue. Gradual movement correctly highlights Map and TX. Primary/secondary clicks do not activate them. The real firmware reports the requested coordinates and pressed/released buttons. An instruction trace establishes that the game's interaction lock at A5-0xEF4 is set, and its own click callback consequently discards these hotspot actions. Mode A5-0x954 is 3. The game has explicit lock/unlock routines at 0x2786B2/0x2786C0; room/media startup locks it, while normal completion paths unlock it. The missing completion/transition which leaves this lock set remains to be identified. Do not patch the flag to zero as a product correction.

Diagnostic one-shot CIAP data/audio/trigger/EOF interrupt experiments did not recover the state and are rejected. No interrupt injection or temporary game-specific instruction tracing remains in the source. Evidence: `outputs/burn-doorway-investigation` and `work/burn-doorway-handler-trace.log`. The earlier intermittent main-menu fault is still open; a relationship to this later interaction lock is not established.

Further doorway investigation: the video PCLs have been consumed and the PCB has status 0162; the secondary `cdi_BurnCycle2` task has a pending 1420 signal. A timing experiment with memory waits disabled lets this task run (verified by its own real OS-9 calls), but does not recover the corridor or its locked actions. A cold no-waits/menu selection run likewise stalls in the media-stop transition (mode 5). Therefore removing memory waits is not an accepted correction. Artificially restarting CIAP delivery advanced disc sectors but did not advance the scene; that experiment is also rejected and removed from source. Do not infer a missing resume command merely from the stopped drive.

The BIOS event/process structures and game media-completion paths have been inspected locally. The frozen checkpoint is after the initiating fault, so a separate save immediately before the first-door transition has been requested to capture the actual commands, interrupts and callbacks leading into the stall. No Burn-Cycle fix or replacement build is claimed at this point. Diagnostic RAM exports remain local in the task output directory; no original saves are changed.

Upstream comparison on 19 November: the current public CIAP changes mainly concern CD-DA/TOC and retain older audio-start/header/marker handling. Its timer implementation matches ours apart from whitespace. No relevant upstream correction was established or merged. The accepted Atlantis, Secret Mission and Video Speedway changes remain intact.

### Wiimote five-minute idle test

Host transport now measures buttons/meaningful IR movement rather than continuous report traffic. At five minutes it disables IR using the existing cleanup sequence, requests noncontinuous button-only reporting, closes its handles, clears the input sample and latches discovery off. No remote connected also times out. Opening Controller Settings explicitly resumes discovery; otherwise reconnect attempts would undo the idle release. Small jitter/tracking dropouts do not continually reset the timer; held buttons/directions do.

The build guide distinguishes Windows HID-handle release from a verified Bluetooth disconnect/power-off. Actual remote sleep and wake behaviour require a physical hardware check. The separate `opencdi-wiimote-idle-test.exe` includes both library fixes but no claimed Burn-Cycle correction. Validation: 162 core and 67 app tests pass, with six optional checks ignored; controller-screen rendering was also inspected.

### Burn-Cycle 490 doorway correction — 19 November 2026

The user supplied a second save before the first door. An isolated replay moves to (384,250), presses CD-i button 2 at field 100 and releases at 120. Before correction this reproduces the corridor freeze: CIAP stops at next LBA 10387, interaction lock D02230 remains set, and subsequent actions are ignored. The original frozen save and the new pre-door save are preserved separately in the task workspace. Neither user save nor NVRAM was edited.

The video-queue stall is downstream of CPU-fed audio restarting with stale APTR. At the previous stream's completion APTR points to the other buffer. START correctly selects a queued buffer for its completion timer, but did not update APTR or audio_next to that selected buffer. The 490 driver then reads APTR and refills the opposite buffer. With stale APTR it toggles off the newly queued playing buffer instead. SM_Out completion stops, the game's audio-paced video consumer stops draining, its producer fills the queue and sets the record count to zero to pause the disc. This explains why forcing disc delivery or changing CPU waits did not restore interaction.

Correction: on APCR 0140, when a queued buffer exists, synchronize both audio_next and APTR with that buffer. No title detection, game-memory patches, synthetic completion, guest-code changes or save deletion is involved. Temporary instruction/stop tracing and the rejected lookahead experiment have been removed; the accepted header and EOF/trigger handling remain unchanged.

Verification: the same pre-door replay now continues through the transmission and into the next playable view, with sound and further disc reads. A follow-up scripted MAP click displays MAPPING ACTIVATED / POSITION LOCATED / POSITION LOCKED; interaction lock and movie mode are both zero. The new regression exercises consecutive audio streams from either buffer and the BIOS's opposite-APTR refill rule. It fails against the original behavior (APTR 0480 instead of 0000), and passes with the correction. All 163 core and 67 app tests pass, with six optional app checks ignored. Further checks continue the saved Atlantis first level, Secret Mission bedroom and Video Speedway race; Dragon's Lair is checked from cold launch. These are bounded regression checks, not full-game completion claims.

Evidence: task outputs/burn-transition-investigation (door-click, start-pointer, map-after-door-reports, regression-*); work/burn-regression-before.log records the failing regression against the old behavior. The separate test app is opencdi-burn-cycle-fix.exe. Load the pre-door save or start a fresh game; the already-frozen save has passed the lost audio event and is not claimed recoverable. Repeated frontend launches for the separately reported intermittent New Game menu issue still require a live check. Autoplay remains deferred.
