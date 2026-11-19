//! [334] CROWN — synthesised 80s snare. 29 dedicated controls plus the shared
//! Frequency / Attack / Decay / Volume; `special` index 4 stays reserved.
use super::*;

const CROWN_STD: &[StandardParamDef] = &[
    s(StandardField::Freq, "Tone", ParamFamily::Osc, 70.0, 260.0, false, Some(" Hz")),
    s(StandardField::Attack, "Attack", ParamFamily::Env, 0.0001, 0.005, false, Some(" s")),
    s(StandardField::Decay, "Tone Tail", ParamFamily::Env, 0.015, 0.3, false, Some(" s")),
    s(StandardField::Volume, "Volume", ParamFamily::Output, 0.0, 2.0, false, None),
];

const fn crown(
    name: &'static str,
    label: &'static str,
    default: f32,
    min: f32,
    max: f32,
    special_index: usize,
    family: ParamFamily,
    unit: Option<&'static str>,
) -> SpecialParamDef {
    SpecialParamDef {
        name,
        label,
        default,
        min,
        max,
        special_index,
        family,
        continuous: true,
        unit,
        curve: 1.0,
        options: None,
        display: SpecialValueDisplay::Number,
    }
}

pub(super) const CROWN: InstrumentDef = InstrumentDef {
    index: 40,
    name: "Crown",
    label: "CR",
    full_name: "CROWN",
    midi_note: 70,
    algo_count: 1,
    standard_params: CROWN_STD,
    special_params: &[
        crown("crown_sweep", "Sweep", 6.32, 0.0, 14.0, 0, ParamFamily::Osc, Some(" st")),
        crown("crown_sweep_time", "Sweep Time", 33.05, 3.0, 120.0, 1, ParamFamily::Osc, Some(" ms")),
        crown("crown_drift", "Drift", 0.0, -6.0, 2.0, 2, ParamFamily::Pitch, Some(" st")),
        crown("crown_tone_shape", "Tone Shape", 1.2, 0.6, 2.5, 3, ParamFamily::Osc, None),
        crown("crown_partial_ratio", "Partial Ratio", 1.63, 1.3, 3.2, 5, ParamFamily::Osc, Some(" x")),
        crown("crown_partial", "Partial", 0.5, 0.0, 0.8, 6, ParamFamily::Osc, None),
        crown("crown_drive", "Drive", 0.1, 0.0, 1.0, 7, ParamFamily::Saturation, None),
        crown("crown_tone", "Tone Level", 0.32, 0.05, 2.0, 8, ParamFamily::Osc, None),
        crown("crown_noise", "Noise Level", 0.4, 0.05, 4.0, 9, ParamFamily::Osc, None),
        crown("crown_noise_highpass", "Noise HP", 330.0, 300.0, 5000.0, 10, ParamFamily::Filter, Some(" Hz")),
        crown("crown_noise_lowpass", "Noise LP", 12000.0, 3000.0, 16000.0, 11, ParamFamily::Filter, Some(" Hz")),
        crown("crown_noise_peak", "Noise Peak", 6300.0, 1500.0, 9000.0, 12, ParamFamily::Filter, Some(" Hz")),
        crown("crown_noise_peak_gain", "Peak Gain", 10.0, -6.0, 15.0, 13, ParamFamily::Filter, Some(" dB")),
        crown("crown_noise_attack", "Noise Attack", 1.9, 0.1, 40.0, 14, ParamFamily::Env, Some(" ms")),
        crown("crown_noise_decay", "Noise Decay", 300.0, 15.0, 400.0, 15, ParamFamily::Env, Some(" ms")),
        crown("crown_noise_shape", "Noise Shape", 1.2, 0.6, 3.0, 16, ParamFamily::Env, None),
        crown("crown_noise_mod", "Noise Mod", 0.2, 0.0, 0.9, 17, ParamFamily::Modulation, None),
        crown("crown_tail", "Tail", 0.3, 0.0, 0.5, 18, ParamFamily::Env, None),
        crown("crown_tail_decay", "Tail Decay", 165.0, 50.0, 600.0, 19, ParamFamily::Env, Some(" ms")),
        crown("crown_phase", "Start Phase", 0.08, 0.0, 1.0, 20, ParamFamily::Osc, Some(" tr")),
        crown("crown_snap", "Snap", 0.99, -3.0, 3.0, 21, ParamFamily::Osc, None),
        crown("crown_snap_width", "Snap Width", 0.16, 0.15, 3.0, 22, ParamFamily::Osc, Some(" ms")),
        crown("crown_click", "Click", 2.7, 0.0, 3.0, 23, ParamFamily::Osc, None),
        crown("crown_click_freq", "Click Freq", 1000.0, 800.0, 6000.0, 24, ParamFamily::Osc, Some(" Hz")),
        crown("crown_click_decay", "Click Decay", 1.8, 0.5, 30.0, 25, ParamFamily::Osc, Some(" ms")),
        crown("crown_delay", "Body Delay", 2.39, 0.0, 40.0, 26, ParamFamily::Osc, Some(" ms")),
        crown("crown_gate", "Gate", 1000.0, 40.0, 1000.0, 27, ParamFamily::Env, Some(" ms")),
        crown("crown_gate_release", "Gate Release", 40.0, 0.5, 60.0, 28, ParamFamily::Env, Some(" ms")),
        crown("crown_density", "Density", 0.0, 0.0, 1.0, 29, ParamFamily::Saturation, None),
    ],
    sound_settings_default: [
        136.95, // f0, Hz
        0.15,   // t_dec, seconds (150 ms in the source)
        1.0,    // Volume
        12000.0,
        0.0003, // t_atk, seconds (0.3 ms in the source)
        0.0,
        0.0,
        0.0,
        0.0,
        0.0,
        0.05,
        0.0,
        0.0,
    ],
    filter_type_label: "",
    freq_display_ratio: 1.0,
    is_sampler: false,
    freq_as_notes: true,
    analog_fixed: true,
};
