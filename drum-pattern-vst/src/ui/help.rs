//! [283] Help mode: the header "Help" keycap turns on explanatory tooltips
//! over the features. The texts live here, in one place; a call site wraps a
//! block of widgets with [`zone`]. Off, a zone costs one flag read.

use nih_plug_egui::egui;

fn flag_id() -> egui::Id {
    egui::Id::new("flash_drum_help_mode")
}

/// Whether help mode is on for this editor (set once per frame from the
/// persisted `GlobalConfig::help_mode`, and by the header keycap).
pub fn is_on(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp(flag_id()).unwrap_or(false))
}

pub fn set(ctx: &egui::Context, on: bool) {
    ctx.data_mut(|d| d.insert_temp(flag_id(), on));
}

/// Lays out `add` as usual; in help mode, hovering the block shows `text`.
/// The hover zone is registered after the contents with a hover-only sense:
/// it sits on top, so egui marks it hovered, but it never takes a click or a
/// drag from the widgets underneath.
pub fn zone<R>(
    ui: &mut egui::Ui,
    id_salt: &str,
    text: &str,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let inner = ui.scope(add);
    if is_on(ui.ctx()) {
        ui.interact(
            inner.response.rect,
            ui.id().with(("help_zone", id_salt)),
            egui::Sense::hover(),
        )
        .on_hover_text(text);
    }
    inner.inner
}

/// Tooltip of the header keycap itself, shown whether help mode is on or off.
pub const HELP_KEYCAP: &str =
    "Help mode: hover a control to see what it does. Click again to turn it off.";

// --- Generator (bottom panel) -------------------------------------------

pub const GEN_TYPE: &str = "Generator type: how the pattern is built.\n\
- Probabilistic: each step is drawn from the style's odds. The only type that blends style A with style B.\n\
- Markov: each step depends on the one before, for flowing grooves.\n\
- Euclidean: the hits are spread as evenly as possible over the bar.\n\
- Classic: starts from an iconic pattern of the style, then varies it.";

pub const GEN_STYLE_A: &str =
    "Style A: the main style (Rock, Funk, Techno...). Every generator type follows it.";

pub const GEN_MIX: &str = "Mix A/B: blends style A with style B (Probabilistic type only).\n\
Far left: style A only. Far right: style B only. In between: the odds of both styles are mixed.";

pub const GEN_STYLE_B: &str =
    "Style B: the second style, blended in with the Mix A/B slider. Only the Probabilistic type uses it.";

pub const GEN_DENSITY: &str = "Density: how many hits. Left = sparse, right = busy.";

pub const GEN_VARIATION: &str = "Variation:\n\
- Classic type: how far the pattern moves away from the original.\n\
- Every type: how differently several lanes of the same instrument (two kicks, two snares...) play.";

pub const GEN_GENERATE: &str = "Writes a new pattern over the whole grid with the settings above. Each click gives a different result.\n\
Replaces the current steps, fused cells and parameter locks. The pattern bank is not touched.";

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs one frame with a button wrapped in a help zone, laid out in a row
    /// like the generator's controls. Returns (button clicked, tooltip shown).
    fn frame(ctx: &egui::Context, events: Vec<egui::Event>, help_on: bool) -> (bool, bool) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::Vec2::new(800.0, 600.0),
            )),
            events,
            ..Default::default()
        };
        let mut clicked = false;
        let _ = ctx.run(input, |ctx| {
            set(ctx, help_on);
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.horizontal(|ui| {
                    clicked = zone(ui, "test", "help text", |ui| {
                        ui.add_sized(egui::Vec2::new(100.0, 30.0), egui::Button::new("B"))
                    })
                    .clicked();
                });
            });
        });
        let tooltip = ctx.memory(|m| {
            m.areas()
                .visible_layer_ids()
                .iter()
                .any(|l| l.order == egui::Order::Tooltip)
        });
        (clicked, tooltip)
    }

    /// In help mode the explanation shows on hover, and the hover zone (on
    /// top of the widgets) never eats their clicks, help mode on or off.
    #[test]
    fn a_help_zone_explains_on_hover_and_never_takes_the_click() {
        for help_on in [false, true] {
            let ctx = egui::Context::default();
            let pos = egui::pos2(58.0, 23.0); // inside the button (panel margin 8)
            frame(&ctx, vec![], help_on);
            let (_, tooltip) = frame(&ctx, vec![egui::Event::PointerMoved(pos)], help_on);
            assert_eq!(tooltip, help_on, "tooltip with help mode {help_on}");
            let press = |pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            frame(&ctx, vec![press(true)], help_on);
            let (clicked, _) = frame(&ctx, vec![press(false)], help_on);
            assert!(clicked, "click lost with help mode {help_on}");
        }
    }

    #[test]
    fn help_mode_flag_follows_set() {
        let ctx = egui::Context::default();
        assert!(!is_on(&ctx));
        set(&ctx, true);
        assert!(is_on(&ctx));
        set(&ctx, false);
        assert!(!is_on(&ctx));
    }
}
