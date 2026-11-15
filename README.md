# Flash Drum

**A free drum machine plugin (VST3, Windows).** Flash Drum started as a sidekick for a Roland TR-606 and grew into a full instrument: a 64-step sequencer with modular lanes for synthesized, sampled, modelled or hybrid drums, plus two send-effect buses.

**[Download the latest version](https://github.com/TiiPaa/flashdrum/releases)** · Beta — feedback and bug reports welcome in [Issues](https://github.com/TiiPaa/flashdrum/issues).

## Highlights

### Sound engines

- **12 instrument lanes plus two effect buses** — each instrument lane holds its own engine and sound settings.
- **Synthesized voices** — Kick, Snare, Hi-Hat, Tom, Clap, Ride, Cymbal, an 808-style kick and percussion.
- **TR-606 multisamples** — every hit picks a different layer, like the hardware.
- **Six modelled 606 voices** — ported from analogcode's open-source models.
- **SDrex** — a metallic snare with filter modulation, flanger feedback and dedicated modulation envelopes.
- **Buzz** — a tonal-percussion voice with adjustable noise, a fast gate and a machine-gun retrigger.
- **Rift** — slices a long embedded or custom texture with Offset, Wander, Advance, Grain, Loop and Reverse.
- **One-Shot** — drop your own WAV file on the grid and play it from any start point, forward or reversed.
- **Nine Percussion Forge engines** — ION BLOOM, FERROSKIN, PRISM HAT, ORBIT CLAY, NEON TOM, ARC ZAP, TIDAL ALLOY, MÖBIUS SKIN and ABYSS FM. Their 68 source controls are editable and available as per-step sound locks; seven engines offer optional stereo, with mono as the default.
- **Full sound editor** — oscillator/sample, amp envelope, pitch envelope, filter, modulation, distortion and output sections per instrument.
- **Analog drift and five distortion flavours** — SoftClip, Valve, Transistor, HardClip and Tape, with pre-/post-filter routing.

### Sequencer

- **64 steps on 4 pages**, with a different length per lane for polyrhythms.
- **Swing with four groove shapes** — Straight, Swing 16th, Shuffle and MPC Style.
- **Per-lane Push/Pull timing and Humanize velocity**.
- **Per-step sound locks** — lock one or several sound parameters on any cell, while untouched parameters keep following the lane.
- **Sequencer locks per step** — probability, stutter up to ×16, ±100 ms nudge, loop conditions with inversion and optional second condition, and per-cell solo.
- **Fused cells** — join adjacent steps into a longer note that can morph from one sound to another.
- **Page tools** — follow the playhead, copy/paste/clear pages, shift the whole grid, or loop one page.
- **Lane tools** — copy or paste a complete lane or only its grid, link a lane's steps to the one above, clear or randomize it.
- **Pattern generators** — Probabilistic, Markov, Euclidean and Classic algorithms, with 16 musical styles, style mixing, density and variation.
- **Quick presets** — Rock, Funk, Disco, House, Dub, Drum'n'Bass, Bossa, Afrobeat and Breakbeat kits and grooves.
- **16-pattern bank with copy/paste** and a **16-block song mode** with repeats.

### DAW integration

- **Main mix plus 14 stereo aux outputs** — route several lanes to the same aux, or keep one in both the main mix and an aux.
- **Four choke groups** — for hats and any combination of lanes.
- **External MIDI input** — choose the note of each lane, auto-assign notes, or switch patterns by MIDI.
- **MIDI output** — Flash Drum emits NoteOn/NoteOff on its configured channel.
- **MIDI export and drag** — export a MIDI file or drag the current pattern directly into your DAW, preserving fusions, stutters, swing and microtiming.
- **16 MIDI-learnable macros** — assign DAW controls or automation to any sound parameter on any lane.
- **Presets** — instruments, patterns, kits and songs; 4 instrument presets and 18 patterns are included.

## Install

Requirements: Windows 10 or 11 (64-bit) and a VST3 host. Tested in Studio One and REAPER.

1. Download `FlashDrum-Setup-<version>.exe` from the [releases page](https://github.com/TiiPaa/flashdrum/releases) and run it.
2. Windows may show *"Windows protected your PC"*: the installer is not code-signed yet. Click **More info**, then **Run anyway**.
3. Accept the administrator prompt: the plugin goes to `C:\Program Files\Common Files\VST3`, where every DAW looks for VST3 plugins.
4. Rescan your plugins in your DAW.

To uninstall: Windows **Settings → Apps → Installed apps → Flash Drum → Uninstall**.

What changed in each version: [RELEASE-NOTES.md](RELEASE-NOTES.md).

## Build from source

Flash Drum is written in Rust with [nih-plug](https://github.com/robbert-vdh/nih-plug) and [egui](https://github.com/emilk/egui). The toolchain is pinned by `drum-pattern-vst/rust-toolchain.toml`.

```powershell
cd drum-pattern-vst
cargo test                # unit and integration tests
.\build.ps1               # release build + VST3 bundle in build\drum-pattern-vst.vst3
.\build.ps1 -Install      # same, then installs into the system VST3 folder (close your DAW first)
```

More details in [drum-pattern-vst/README.md](drum-pattern-vst/README.md).

For the Percussion Forge engines, `cargo run --release --bin test_standalone -- --forge-benchmark`
measures the production DSP at 48 kHz mono/stereo and 96 kHz with extreme settings.
Deterministic source-reference fixtures are exercised by `cargo test forge`.
The browser app's whole-hit peak normalisation is replaced by calibrated fixed
gains and an over-range safety ceiling for real-time playback. Rapid retriggers
reuse the lane's voice rather than layering unlimited hits.

## License

Flash Drum is free software under the [GNU General Public License v3](LICENSE). It builds on third-party components listed in [THIRD-PARTY.txt](THIRD-PARTY.txt); their full license texts are in [THIRD-PARTY-LICENSES.txt](THIRD-PARTY-LICENSES.txt).
