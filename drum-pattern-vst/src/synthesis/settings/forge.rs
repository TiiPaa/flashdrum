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
/// Orbit keeps derived production controls out of persistence. The source
/// conversion below always describes the original engine; `forge.rs` may then
/// adapt those derived values without touching `core.rs`' source equations.
#[derive(Clone, Copy, Debug)]
pub struct OrbitClaySettings {
    pub common: VoiceSettings,
    pub material: f64,
    pub bounce: f64,
    pub gravity: f64,
    pub drift: f64,
    pub mod_rate: f64,
    pub release: f64,
    pub drive: f64,
    pub bounce_interval: f64,
    pub bounce_growth: f64,
    pub bounce_smoothing: f64,
    pub mode_deformation: f64,
    pub soft_drive: f64,
}
impl From<VoiceSettings> for OrbitClaySettings {
    fn from(v: VoiceSettings) -> Self {
        let gravity = v.special[2] as f64;
        let drive = v.special[7] as f64;
        Self {
            common: v,
            material: v.special[0] as f64,
            bounce: v.special[1] as f64,
            gravity,
            drift: v.special[3] as f64,
            mod_rate: v.special[5] as f64,
            release: v.special[6] as f64,
            drive,
            bounce_interval: 0.045 + (1.0 - gravity / 100.0) * 0.085,
            bounce_growth: 1.18 + gravity / 100.0 * 0.42,
            bounce_smoothing: 0.0,
            mode_deformation: 0.0,
            soft_drive: (drive / 100.0).powf(1.5) * 0.3,
        }
    }
}
impl From<OrbitClaySettings> for VoiceSettings {
    fn from(s: OrbitClaySettings) -> Self {
        let mut v = s.common;
        for (index, value) in [
            (0, s.material),
            (1, s.bounce),
            (2, s.gravity),
            (3, s.drift),
            (5, s.mod_rate),
            (6, s.release),
            (7, s.drive),
        ] {
            v.special[index] = value as f32;
        }
        v
    }
}
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
