//! Typed source controls. Standard fields retain the common persistence schema;
//! named engine controls round-trip only their own specials (slot 4 is reserved).
use crate::synthesis::VoiceSettings;

macro_rules! forge_settings {
    ($name:ident { $($field:ident: $slot:literal),* $(,)? }) => {
        #[derive(Clone, Copy, Debug)]
        pub struct $name {
            pub common: VoiceSettings,
            $(pub $field: f64,)*
        }
        impl From<VoiceSettings> for $name {
            fn from(v: VoiceSettings) -> Self {
                Self { common: v, $($field: v.special[$slot] as f64,)* }
            }
        }
        impl From<$name> for VoiceSettings {
            fn from(s: $name) -> Self {
                let mut v = s.common;
                $(v.special[$slot] = s.$field as f32;)*
                v
            }
        }
    };
}

forge_settings!(IonBloomSettings {
    drop: 0,
    bloom: 1,
    impact: 2,
    pressure: 3
});
forge_settings!(FerroskinSettings {
    tension: 0,
    coupling: 1,
    impact: 2,
    noise: 3,
    wire: 5
});
forge_settings!(PrismHatSettings {
    spread: 0,
    density: 1,
    color: 2,
    bite: 3
});
forge_settings!(OrbitClaySettings {
    material: 0,
    bounce: 1,
    gravity: 2,
    drift: 3,
    mod_rate: 5,
    release: 6,
    drive: 7
});
forge_settings!(NeonTomSettings {
    drop: 0,
    shape: 1,
    snap: 2,
    drive: 3
});
forge_settings!(ArcZapSettings {
    amount: 0,
    speed: 1,
    shape: 2,
    fm: 3
});
forge_settings!(TidalAlloySettings {
    morph: 0,
    bell: 1,
    tone_drop: 2,
    drop_speed: 3,
    spread: 5,
    tide: 6,
    lfo_rate: 7,
    lfo_depth: 8
});
forge_settings!(MobiusSkinSettings {
    material: 0,
    twist: 1,
    rotation: 2,
    chaos: 3,
    strike: 5,
    width: 6
});
forge_settings!(AbyssFmSettings {
    drop: 0,
    length: 1,
    ratio: 2,
    index: 3,
    fm_tail: 5,
    fundamental: 6,
    fold: 7,
    drive: 8
});

#[cfg(test)]
mod tests {
    use super::*;
    crate::settings_roundtrip_test!(ion_bloom_settings_roundtrip, ion_bloom, IonBloomSettings);
    crate::settings_roundtrip_test!(ferroskin_settings_roundtrip, ferroskin, FerroskinSettings);
    crate::settings_roundtrip_test!(prism_hat_settings_roundtrip, prism_hat, PrismHatSettings);
    crate::settings_roundtrip_test!(orbit_clay_settings_roundtrip, orbit_clay, OrbitClaySettings);
    crate::settings_roundtrip_test!(neon_tom_settings_roundtrip, neon_tom, NeonTomSettings);
    crate::settings_roundtrip_test!(arc_zap_settings_roundtrip, arc_zap, ArcZapSettings);
    crate::settings_roundtrip_test!(
        tidal_alloy_settings_roundtrip,
        tidal_alloy,
        TidalAlloySettings
    );
    crate::settings_roundtrip_test!(
        mobius_skin_settings_roundtrip,
        mobius_skin,
        MobiusSkinSettings
    );
    crate::settings_roundtrip_test!(abyss_fm_settings_roundtrip, abyss_fm, AbyssFmSettings);
}
