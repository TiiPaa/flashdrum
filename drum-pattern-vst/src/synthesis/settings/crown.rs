//! [334] CROWN typed controls. Persistence stays the shared `special[32]`
//! layout: index 4 is reserved by the p-lock fields.
use crate::synthesis::VoiceSettings;

macro_rules! crown_settings {
    ($($field:ident: $slot:literal),* $(,)?) => {
        #[derive(Clone, Copy, Debug)]
        pub struct CrownSettings {
            pub common: VoiceSettings,
            $(pub $field: f64,)*
        }
        impl From<VoiceSettings> for CrownSettings {
            fn from(v: VoiceSettings) -> Self {
                Self { common: v, $($field: v.special[$slot] as f64,)* }
            }
        }
        impl From<CrownSettings> for VoiceSettings {
            fn from(s: CrownSettings) -> Self {
                let mut v = s.common;
                $(v.special[$slot] = s.$field as f32;)*
                v
            }
        }
    };
}

crown_settings!(
    sweep: 0,
    sweep_t: 1,
    drift: 2,
    t_shape: 3,
    t2r: 5,
    t2: 6,
    drive: 7,
    tone: 8,
    noise: 9,
    n_hp: 10,
    n_lp: 11,
    n_pk_f: 12,
    n_pk_g: 13,
    n_atk: 14,
    n_dec: 15,
    n_shape: 16,
    n_mod: 17,
    tail: 18,
    tail_dec: 19,
    ph0: 20,
    snap: 21,
    snap_w: 22,
    click: 23,
    c_f: 24,
    c_dec: 25,
    dly: 26,
    gate: 27,
    g_rel: 28,
    sat: 29,
);

#[cfg(test)]
mod tests {
    use super::*;
    crate::settings_roundtrip_test!(crown_settings_roundtrip, crown, CrownSettings);
}
