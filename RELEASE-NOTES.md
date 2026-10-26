# Release notes

Downloads: [releases page](https://github.com/TiiPaa/flashdrum/releases).

## 0.9.6 — 2026-10-26 (beta)

**Effect lanes**

- The last two lanes are now effect buses, **Fx1** and **Fx2**. Each one hosts an effect: **Reverb**, **Delay** or **Rumble**, and is named after it. A project that already has an instrument on one of these lanes keeps it; that bus is unavailable until the lane is deleted.
- **Sends**: every instrument lane feeds Fx1 and Fx2 from its new **Effects** tab, after the lane's volume. Each send has a mute (**M**) and an **Edit** button that opens the effect lane. Sends can be locked per step.
- **Reverb**: Hall, Room and Plate types; Size, Decay, Damping, Pre-delay, Low Cut, High Cut, Width; a **Gate** (Gate Time, Gate Release) for gated sounds; **Warp** links Decay to the size of the room, so moving Decay bends the pitch of the tail.
- **Delay**: Time in ms or synced to the host tempo (1/32 to 1 bar), Feedback, Damping, Low Cut, High Cut, Ping-pong, Width.
- **Rumble**: the techno kick rumble. A huge, dark reverb that drops out on each kick and swells back between them, with Echo, Drive, Width, High Pass and its own reverb Size, Motion and Blur.
- **P-locks on an effect lane** move the effect along the pattern: each lock holds until the next one, or, with **Glide** on, the value glides from lock to lock. **Loop Restart** (or the arrow on the lane's first cell) chooses whether the effect goes back to its sliders when the pattern loops. **Playing now** shows the values the effect is using.

**Lane inserts** (Effects tab, on every instrument lane)

- Six effects in series, one of each, each with its own On switch: **Compressor** (with Makeup, Mix and a gain-reduction meter), **Transient** (reshapes the hits and keeps the lane's level), **Filter** (low-pass, high-pass, band-pass, notch), **EQ** (Low Shelf, Mid Peak, High Shelf, each with its own frequency), **Drive** (Soft, Hard, Tube, with anti-aliasing) and **Bitcrush**.
- Drag the handle before an insert's name to change the order of the chain; fold an insert with its triangle.

**Lanes and grid**

- A level meter at the end of every lane.
- **Own Swing** (Track tab): a lane can play its own swing amount and groove instead of the header's.
- The pulses of a fused cell now follow the swing, in playback and in MIDI export.
- **MIDI learn** for a lane's note: Track tab → **Learn**, then play a note on your controller.
- **Humanize** moves to the Track tab (Sequencing); the lane names get more room in the grid.
- Dragging a WAV from the Explorer onto a **One-Shot** lane replaces its file (the lane warns you before you drop). When a drop cannot be accepted, a message says why.
- **Randomize Lane**: each lane has its own Density, with a default per instrument type in **Settings**.
- The step menu opens under its cell (above it near the bottom of the window).
- **Ext MIDI**: Mute and Solo now work when the lanes are played from incoming MIDI notes. A muted lane neither plays nor forwards the note.
- **MIDI export**: a linked lane exports the steps of the lane it follows.
- Volume is shown from 0 to 1.
- Fixed: a lane name sometimes stayed lit after its flash.

**Instruments**

- **Decay Curve** now goes from −2 to +2 for much sharper or much more held decays (−1 to +1 sounds as before), on every voice except the AC606 models. **Attack Curve** added to Buzz and the 606 samplers.
- LFO rates can follow the host tempo (**Sync** menu under each rate): Rift's pitch and filter LFOs, SDrex modulation, Buzz gate and the Toms.
- **Toms**: a pitch LFO (Rate, Depth, Shape, Fade-in).
- Filter **Resonance** on SDrex, the Toms and Buzz.
- **SDrex**: a **Noise <-> Synth** balance replaces the Noise level (older lanes sound the same); modulation Depth works in both directions.
- **Rift**: new **Pad** texture; the **Noise** texture is regenerated with an even level and less low end; the filter LFO reaches 5 octaves and keeps its full swing at the ends of the range.
- **TM6(AC)**: Model is a menu (Auto / Low tom / High tom), Frequency is in Hz, and Overtones, Tail Noise and Decay have a clearer effect.
- **606 samplers**: with Analog Mode on, the Sample row reads "Random each hit".
- Fixed: in Studio One, the Algorithm menu of some lanes (for example Buzz's Razor) snapped back during playback.

**Interface**

- Settings that depend on the switch above them are indented under it.
- Changing the **Zoom** closes Settings.

**Presets**

- Starter presets refreshed: 4 new patterns, **Apollo**, **Battlestar**, **LastWinterV2** and **Wire** (LastWinterV2 replaces LastWinter). They are added on a new installation; an installation that already received the starter presets keeps its own.

**Compatibility**

- Projects saved with 0.9.5 open as before. A project that uses effect lanes cannot be opened correctly by 0.9.5 or earlier.

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
