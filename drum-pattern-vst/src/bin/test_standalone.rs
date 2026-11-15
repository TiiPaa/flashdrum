#![allow(dead_code)]

// [254] Same allocation watchdog as lib.rs: this binary recompiles the same
// modules through `#[path]`, and their `assert_no_alloc` tests need the
// disabler registered in THIS crate's test build too.
#[cfg(test)]
#[global_allocator]
static ALLOC_DISABLER: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

#[path = "../groove.rs"]
mod groove;
#[path = "../instrument_registry.rs"]
mod instrument_registry;
#[path = "../param_id.rs"]
mod param_id;
#[path = "../sequencer/mod.rs"]
mod sequencer;
#[path = "../synthesis/mod.rs"]
mod synthesis;
#[path = "../track.rs"]
mod track;

use groove::GrooveType;
use sequencer::Sequencer;
use sequencer::{Pattern, SharedPattern};
use synthesis::DrumSynthesizer;

fn main() {
    if std::env::args().any(|arg| arg == "--forge-benchmark") {
        forge_benchmark();
        return;
    }
    println!("Flash Drum - Test Standalone");
    println!("==========================================");
    println!();

    let shared_pattern = SharedPattern::new(&Pattern::rock_pattern());
    let mut sequencer = Sequencer::new(shared_pattern);
    let mut synthesizer = DrumSynthesizer::new();
    let sample_rate = 44100.0;

    synthesizer.initialize(sample_rate);
    sequencer.play();

    println!("Kick drum should be playing...");
    println!("Press Ctrl+C to stop");
    println!();

    // Simulate 5 seconds of playback
    let bpm = 120.0;
    let samples_per_second = sample_rate as usize;
    let total_samples = samples_per_second * 5;

    let mut output_buffer = vec![0.0f32; 512];

    for i in 0..(total_samples / 512) {
        for sample in output_buffer.iter_mut() {
            let triggers = sequencer.process_sample(bpm, sample_rate, 0.0, GrooveType::Swing16);

            for (voice_idx, trigger) in triggers.iter().enumerate() {
                if trigger.should_trigger {
                    synthesizer.trigger(voice_idx, trigger.velocity);
                }
            }

            synthesizer.process_sample(sample);
        }

        // Display activity every second
        if i % (samples_per_second / 512) == 0 {
            let second = i / (samples_per_second / 512);
            let max_sample = output_buffer
                .iter()
                .map(|s| s.abs())
                .fold(0.0f32, |a, b| a.max(b));
            println!("Second {}: max amplitude = {:.4}", second, max_sample);
        }
    }

    println!();
    println!("Test complete!");
}

/// Real production code, without the debug-only allocation watchdog or the
/// full unit-test executable. Run with cargo run --release --bin test_standalone
/// -- --forge-benchmark. The timed loop does not allocate or print.
fn forge_benchmark() {
    use synthesis::{DrumVoice, VoiceSettings};
    use track::{TrackInstrumentKind as K, TrackLayoutState, TrackSlot};
    let kinds = [
        K::IonBloom,
        K::Ferroskin,
        K::PrismHat,
        K::OrbitClay,
        K::NeonTom,
        K::ArcZap,
        K::TidalAlloy,
        K::MobiusSkin,
        K::AbyssFm,
    ];
    for (label, sr, stereo, extreme) in [
        ("mono48", 48000.0, false, false),
        ("stereo48", 48000.0, true, false),
        ("extreme96", 96000.0, true, true),
    ] {
        let mut total = 0.0;
        for kind in kinds {
            let mut layout = TrackLayoutState::empty_layout();
            layout.slots[0] = TrackSlot::active_with_kind(kind);
            let mut synth = DrumSynthesizer::new();
            synth.initialize_with_layout(sr, &layout);
            let voice = DrumVoice::from_index(kind.drum_voice_index()).unwrap();
            let mut settings = VoiceSettings::forge(voice);
            settings.stereo = if stereo { 1.0 } else { 0.0 };
            if extreme {
                for p in instrument_registry::INSTRUMENTS[voice as usize].special_params {
                    settings.special[p.special_index] = p.max;
                }
            }
            synth.set_voice_settings(0, settings);
            synth.trigger(0, 1.0);
            let mut outputs = [[0.0; 2]; track::MAX_TRACKS];
            let seconds = 0.25;
            let start = std::time::Instant::now();
            for _ in 0..(sr * seconds) as usize {
                synth.process_voice_samples_stereo(&mut outputs);
                std::hint::black_box(&outputs);
            }
            let percent = start.elapsed().as_secs_f64() / seconds as f64 * 100.0;
            total += percent;
            println!(
                "{label} {kind:?}: {percent:.2}% of one core; Forge pool {} KiB",
                synth.forge_buffer_bytes() / 1024
            );
        }
        println!("{label} sum of nine separate measurements: {total:.2}% of one core");
    }
}
