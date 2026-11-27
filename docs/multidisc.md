# Multi-disc titles and refreshed box art

The library shows one entry for each unambiguous set of numbered disc images,
with a disc-count badge. Regional, language, revision and directory differences
remain separate. Files with duplicate disc numbers are not automatically merged.
Numbered volumes and sequels are not inferred to be disc sets.

Open the entry to choose a disc in its info screen, then press Play. Mouse and
Wiimote pointing can select a disc directly. Keyboard Left/Right and gamepad or
Wiimote D-pad Left/Right change the selection; the existing confirmation control
starts Play. The last selected disc is remembered in `DiscSelections.json`.

The shared cover, logo and synopsis remain visible, while the CD image follows
the selected disc. Missing CD images use the neutral disc placeholder, never a
different disc number's artwork. Searching for a member disc also finds the set.
The collection count still counts physical disc images, rather than hiding them
from the count when the grid combines them into one entry.

During gameplay, open Pause and then Info to select another disc manually.
Selection there inserts that disc through the existing hot-swap path without
resetting CPU/RAM. Return to the game with the usual back/resume control.
Automatic disc-change detection and physical-drive enhancements are deferred.
Live in-game switching still needs testing against actual disc-change prompts.

## Shared NVRAM

Numbered discs use a readable title-level filename, for example:

```text
NVRAM/
  220/Example (Europe).bin
  490/Example (Europe).bin
```

On first use/library refresh, an existing Disc 1 NVRAM file seeds the shared file.
Its size must match the BIOS. Existing shared files are never replaced. Original
Disc 1 and Disc 2 files remain intact as backups; Disc 2's differing save is not
silently chosen. If Disc 1 or its save is unavailable, the shared save starts
normally without importing Disc 2's file. Global-NVRAM mode is unchanged.
Save-state filenames and their BIOS separation are unchanged by this update.

## Box art

The new case perspective has the left CD-i spine closer and the right edge
receding. Each title's original cover pixels are projected onto the new frame.
Original Covers images are never overwritten. Generated Boxart images have a
new version signature and regenerate during background library loading, including
titles whose online media lookup was already completed. Unchanged images are
then reused. Missing source covers cannot be regenerated until a cover is present.

The new frame in `assets/boxart_case_v2.png` was produced with the built-in image
generation tool from the previous case preview. The prompt requested an upright
closed CD-i jewel case with the left spine nearer, right edge receding, original
CD-i spine/plastic edges retained, and a transparent background. Its reference
front artwork is replaced by the renderer using each game's original cover;
generated front lettering is not used in the final per-title artwork.
