//! A per-editor bridge from baseview's native drop target to the UI.
//! The UI publishes an accepting rectangle each frame. The OS callback only
//! queues paths and their exact position; file decoding happens in a later frame.

use std::path::PathBuf;

use baseview::{DropData, DropEffect, EventStatus, MouseEvent};
use egui::{Context, Id, Pos2, RawInput, Rect};

#[derive(Clone)]
struct Target {
    rect: Rect,
    extensions: Vec<String>,
}

/// One accepted OS drop. Position is in this editor's egui coordinate system.
#[derive(Clone, Debug)]
pub struct FileDrop {
    pub position: Pos2,
    pub paths: Vec<PathBuf>,
}

fn target_id() -> Id {
    Id::new("egui_baseview_file_drop_target")
}

fn pending_id() -> Id {
    Id::new("egui_baseview_pending_file_drops")
}

fn rejected_id() -> Id {
    Id::new("egui_baseview_file_drop_rejected_hover")
}

fn hover_id() -> Id {
    Id::new("egui_baseview_file_drop_hover")
}

/// `None` disables drops (for example when the grid is full or a modal is open).
pub fn set_target(ctx: &Context, rect: Option<Rect>, extensions: &[&str]) {
    ctx.data_mut(|data| match rect {
        Some(rect) => data.insert_temp(
            target_id(),
            Target {
                rect,
                extensions: extensions.iter().map(|ext| ext.to_string()).collect(),
            },
        ),
        None => data.remove::<Target>(target_id()),
    });
}

/// Take each accepted drop once, even if egui runs more than one layout pass.
pub fn take_dropped(ctx: &Context) -> Vec<FileDrop> {
    ctx.data_mut(|data| {
        std::mem::take(data.get_temp_mut_or_default::<Vec<FileDrop>>(pending_id()))
    })
}

/// [273] The position of an OS drag this editor is currently REJECTING
/// (target disabled, outside the rectangle, or wrong file type) — so the UI
/// can say WHY next to the OS "forbidden" cursor. `None` while no rejected
/// drag is hovering (or after the drag leaves).
pub fn rejected_hover_position(ctx: &Context) -> Option<Pos2> {
    ctx.data_mut(|data| data.get_temp::<Pos2>(rejected_id()))
}

/// [270] The position of ANY OS drag hovering this editor, accepted or not, so the
/// UI can narrow its target to where a drop would do something. `None` once the
/// drag leaves or drops.
pub fn hover_position(ctx: &Context) -> Option<Pos2> {
    ctx.data_mut(|data| data.get_temp::<Pos2>(hover_id()))
}

fn accepted_paths(ctx: &Context, position: Pos2, files: &DropData) -> Vec<PathBuf> {
    let Some(target) = ctx.data_mut(|data| data.get_temp::<Target>(target_id())) else {
        return Vec::new();
    };
    if !target.rect.contains(position) {
        return Vec::new();
    }
    let DropData::Files(paths) = files else {
        return Vec::new();
    };
    paths.iter().filter(|path| {
        path.extension().and_then(|ext| ext.to_str()).is_some_and(|ext| {
            target.extensions.iter().any(|wanted| ext.eq_ignore_ascii_case(wanted))
        })
    }).cloned().collect()
}

pub(crate) fn handle_event(
    ctx: &Context,
    input: &mut RawInput,
    event: &MouseEvent,
    position: Option<Pos2>,
) -> Option<EventStatus> {
    let (files, dropped) = match event {
        MouseEvent::DragEntered { data, .. } | MouseEvent::DragMoved { data, .. } => (data, false),
        MouseEvent::DragDropped { data, .. } => (data, true),
        MouseEvent::DragLeft => {
            input.hovered_files.clear();
            ctx.data_mut(|data| {
                data.remove::<Pos2>(rejected_id());
                data.remove::<Pos2>(hover_id());
            });
            ctx.request_repaint();
            return Some(EventStatus::Ignored);
        }
        _ => return None,
    };
    let Some(position) = position else {
        input.hovered_files.clear();
        ctx.data_mut(|data| {
            data.remove::<Pos2>(rejected_id());
            data.remove::<Pos2>(hover_id());
        });
        return Some(EventStatus::Ignored);
    };
    ctx.data_mut(|data| {
        if dropped {
            data.remove::<Pos2>(hover_id());
        } else {
            data.insert_temp(hover_id(), position);
        }
    });
    let paths = accepted_paths(ctx, position, files);
    input.hovered_files.clear();
    if paths.is_empty() {
        // [273] Remember where the rejected drag hovers: the grid shows the
        // reason (full, modal open) instead of the bare ⊘ cursor.
        ctx.data_mut(|data| {
            data.insert_temp(rejected_id(), position);
        });
        ctx.request_repaint();
        return Some(EventStatus::Ignored);
    }
    ctx.data_mut(|data| {
        data.remove::<Pos2>(rejected_id());
    });
    if dropped {
        input.dropped_files.extend(paths.iter().map(|path| egui::DroppedFile {
            path: Some(path.clone()),
            ..Default::default()
        }));
        ctx.data_mut(|data| {
            data.get_temp_mut_or_default::<Vec<FileDrop>>(pending_id())
                .push(FileDrop { position, paths });
        });
    } else {
        input.hovered_files.extend(paths.into_iter().map(|path| egui::HoveredFile {
            path: Some(path),
            ..Default::default()
        }));
    }
    ctx.request_repaint();
    Some(EventStatus::AcceptDrop(DropEffect::Copy))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(files: &[&str], dropped: bool) -> MouseEvent {
        let position = baseview::Point::new(50.0, 30.0);
        let data = DropData::Files(files.iter().map(PathBuf::from).collect());
        if dropped {
            MouseEvent::DragDropped { position, data, modifiers: Default::default() }
        } else {
            MouseEvent::DragEntered { position, data, modifiers: Default::default() }
        }
    }

    fn enable(ctx: &Context) {
        set_target(
            ctx,
            Some(Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(100.0, 100.0))),
            &["wav"],
        );
    }

    /// [273] A rejected drag reports its position until it leaves or is
    /// accepted, so the UI can explain the refusal.
    #[test]
    fn rejected_drag_reports_its_position_until_it_leaves() {
        let ctx = Context::default();
        let mut input = RawInput::default();
        let position = egui::pos2(50.0, 30.0);
        assert_eq!(rejected_hover_position(&ctx), None);

        // No target published: rejection, and the position is reported.
        let status = handle_event(&ctx, &mut input, &event(&["kick.wav"], false), Some(position));
        assert_eq!(status, Some(EventStatus::Ignored));
        assert_eq!(rejected_hover_position(&ctx), Some(position));

        // Accepted this time: the marker is cleared.
        enable(&ctx);
        let status = handle_event(&ctx, &mut input, &event(&["kick.wav"], false), Some(position));
        assert_eq!(status, Some(EventStatus::AcceptDrop(DropEffect::Copy)));
        assert_eq!(rejected_hover_position(&ctx), None);

        // Rejected again (wrong type), then DragLeft clears the marker.
        let status = handle_event(&ctx, &mut input, &event(&["clip.mp3"], false), Some(position));
        assert_eq!(status, Some(EventStatus::Ignored));
        assert_eq!(rejected_hover_position(&ctx), Some(position));
        handle_event(&ctx, &mut input, &MouseEvent::DragLeft, None);
        assert_eq!(rejected_hover_position(&ctx), None);
    }

    /// [270] Any hovering drag reports its position, accepted or not, until it
    /// leaves or drops.
    #[test]
    fn any_drag_reports_its_hover_position_until_it_leaves_or_drops() {
        let ctx = Context::default();
        let mut input = RawInput::default();
        let position = egui::pos2(50.0, 30.0);
        assert_eq!(hover_position(&ctx), None);
        handle_event(&ctx, &mut input, &event(&["kick.wav"], false), Some(position));
        assert_eq!(hover_position(&ctx), Some(position), "rejected drag");
        enable(&ctx);
        handle_event(&ctx, &mut input, &event(&["kick.wav"], false), Some(position));
        assert_eq!(hover_position(&ctx), Some(position), "accepted drag");
        handle_event(&ctx, &mut input, &MouseEvent::DragLeft, None);
        assert_eq!(hover_position(&ctx), None);
        handle_event(&ctx, &mut input, &event(&["kick.wav"], false), Some(position));
        handle_event(&ctx, &mut input, &event(&["kick.wav"], true), Some(position));
        assert_eq!(hover_position(&ctx), None, "a drop ends the drag");
    }

    #[test]
    fn native_drag_is_accepted_and_delivered_once_at_its_drop_position() {
        let ctx = Context::default();
        enable(&ctx);
        let mut input = RawInput::default();
        let position = egui::pos2(50.0, 30.0);
        assert_eq!(handle_event(&ctx, &mut input, &event(&["kick.WAV", "notes.txt"], false), Some(position)),
            Some(EventStatus::AcceptDrop(DropEffect::Copy)));
        assert_eq!(input.hovered_files.len(), 1);
        assert!(take_dropped(&ctx).is_empty());
        assert_eq!(handle_event(&ctx, &mut input, &event(&["kick.WAV"], true), Some(position)),
            Some(EventStatus::AcceptDrop(DropEffect::Copy)));
        assert!(input.hovered_files.is_empty());
        assert_eq!(input.dropped_files.len(), 1);
        let drops = take_dropped(&ctx);
        assert_eq!(drops.len(), 1);
        assert_eq!(drops[0].position, position);
        assert_eq!(drops[0].paths, [PathBuf::from("kick.WAV")]);
        assert!(take_dropped(&ctx).is_empty());
    }

    #[test]
    fn rejects_other_formats_outside_grid_and_disabled_target() {
        let ctx = Context::default();
        enable(&ctx);
        let mut input = RawInput::default();
        let position = Some(egui::pos2(50.0, 30.0));
        assert_eq!(handle_event(&ctx, &mut input, &event(&["clip.mp3"], true), position), Some(EventStatus::Ignored));
        assert_eq!(handle_event(&ctx, &mut input, &event(&["kick.wav"], true), Some(egui::pos2(500.0, 300.0))), Some(EventStatus::Ignored));
        set_target(&ctx, None, &["wav"]);
        assert_eq!(handle_event(&ctx, &mut input, &event(&["kick.wav"], true), position), Some(EventStatus::Ignored));
        assert!(take_dropped(&ctx).is_empty());
    }

    #[test]
    fn leave_clears_hover_and_editors_never_share_drops() {
        let first = Context::default();
        let second = Context::default();
        enable(&first);
        enable(&second);
        let mut input = RawInput::default();
        let position = Some(egui::pos2(50.0, 30.0));
        handle_event(&first, &mut input, &event(&["kick.wav"], false), position);
        handle_event(&first, &mut input, &MouseEvent::DragLeft, None);
        assert!(input.hovered_files.is_empty());
        handle_event(&first, &mut input, &event(&["kick.wav"], true), position);
        assert!(take_dropped(&second).is_empty());
        assert!(take_dropped(&Context::default()).is_empty());
        assert_eq!(take_dropped(&first).len(), 1);
    }
}
