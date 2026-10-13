# Release notes

Downloads: [releases page](https://github.com/TiiPaa/flashdrum/releases).

## 0.9.5 — 2026-10-13 (beta)

- **Easier to read**: brighter secondary text in every skin and all text one size larger.
- **Zoom**: Settings › **Zoom** enlarges the whole window to 125 % or 150 % (remembered between sessions).
- **Help mode**: the **Help** key in the header turns on explanations for every control. Hover over a control to read what it does.
- **Lane Editor redesign**: every section of the Sound tab is its own module with a clear title strip, values sit in aligned readouts, **Volume** stays in place while the sections below scroll, and the Sound / Track tabs are proper folder tabs.
- **Kick-like voices**: the Hz / Note choice has its own **Mode** row above **Frequency**; the Oscillator section no longer runs wider than the others.
- **Step / Start / End** only appears on a fused cell, where it applies.
- **Ctrl+click** (Cmd+click on Mac) a step to edit its sound in the Lane Editor without turning it on or off and without opening its menu.
- **Step menu**: it no longer closes the moment it opens when the window is zoomed.
- **Linked lanes**: a linked lane shows the p-locks of the lane it follows; fused cells are shared along the chain while each lane keeps its own morph.
- **Generator**: **Density** now has a clear effect in all four types, from sparse to busy, and still varies at its minimum; options that do not apply to the chosen type are greyed; style menus fit their longest name.
- **New instances** start on the **Tommy01** pattern. Saved projects open exactly as before.

## 0.9.4 — 2026-10-11 (beta)

- **Diagnostic log for the slowdown reported in Bitwig** (the DAW getting slower and slower after about 15 minutes). Off by default: nothing changes unless you turn it on. To help us find the cause:
  1. Close your DAW.
  2. Windows search → **Edit environment variables for your account** → **New…** → name `FLASH_DRUM_PERF_LOG`, value `1` → OK.
  3. Start the DAW, open Flash Drum's window and work as usual until the slowdown appears (20 minutes or more).
  4. Send us the file `flash-drum-perf.log` from your temp folder (type `%TEMP%` in the Explorer address bar), through [Issues](https://github.com/TiiPaa/flashdrum/issues).
  5. Delete the variable afterwards.

  Every 30 seconds while the window is open, the log records the host's memory use, Windows handle counts and CPU time. It contains no audio, no project data and no personal information.

## 0.9.3 — 2026-10-02 (beta)

- **Included presets**: 4 instrument presets and 18 patterns are now bundled with the plugin. They were missing from the 0.9.2 download.
- On the first editor open, these presets are added to `Documents/Flash Drum/presets/` and appear in the preset browser's **User** section. They can be renamed, edited or deleted like any other user preset.
- Existing files with the same name are preserved. Presets deleted after this initial setup are not restored on subsequent editor opens.

## 0.9.2 — 2026-10-01 (beta)

- **Grid header**: the column headings sit over their columns again (they were shifted one column to the left).
- **Kick-like voices**: switching the Frequency row between Hz and Notes no longer shifts the rows below it.
- **Installer**: the uninstaller now lives in `C:\Program Files\Flash Drum`, outside the plugin folder; the GPL is shown for information (nothing to accept); the third-party notice is plain text; the full license texts of every third-party component ship with the plugin (`THIRD-PARTY-LICENSES.txt`).
- Internal clean-up of the audio engine and the Sound panel, with no change to the sound.

## 0.9.1 — 2026-10-01

- **Grid**: two arrows ‹ › above the lane names shift the whole grid by one step — steps, fused cells and parameter locks move together and wrap inside the pattern length. When a fused cell sits on the edge, a warning asks before breaking it.
- **Randomize Lane**: the default density is now 80 %.
- **Settings › MIDI**: the Auto-assign caption sits beside its button, like the Macros row.
- **Snare606 and 808 Kick**: they now declare the single algorithm they actually have (no audible change).
- Internal clean-up: instrument traits declared in one place, dead code removed, the voice engine driven by a single list.
