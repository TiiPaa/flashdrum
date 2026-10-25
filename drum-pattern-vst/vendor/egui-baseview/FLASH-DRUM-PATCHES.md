# Flash Drum adapter patch

Upstream: BillyDM/egui-baseview, revision
`ec70c3fe6b2f070dcacbc22924431edbe24bd1c0` (same revision previously used).
Source and MIT license are kept with this copy.

[243]: forward baseview's existing file-drag events to egui and return
`AcceptDrop(Copy)` for an accepted destination. No second `RegisterDragDrop`
registration is needed: baseview already owns and cleans up the native target.
Drop state belongs to each editor, including the exact drop position; it is
discarded when the window closes. Windows coordinates in this pinned baseview
revision need conversion from screen to client coordinates before egui scaling.

[243] lane-flash fix (`window.rs`): after a `do_repaint_now` render, keep the
`repaint_delay` egui just requested instead of clearing `repaint_after`.
Upstream discarded it, so a time-based effect whose last rendered frame still
showed a sliver of the effect (the [238] lane-name flash) never got the frame
that turns it fully off — the lit pixels stayed on screen until an unrelated
repaint.

Frame scheduling (`window.rs`, `repaint_schedule`): egui runs at every
baseview tick but a frame is drawn only when egui asks for it now or when a
repaint it asked for falls due. Two holes left the lane-name flash lit at
random: an undrawn frame REPLACED the pending deadline with its own request,
even a later one; and a drawn frame that asked for "now" (egui subtracts the
predicted frame time, so an effect ending within a frame asks for zero)
scheduled nothing after itself. The earliest deadline is now kept, and a
"now" also schedules the next tick. Covered by
`the_lane_flash_is_always_drawn_off` (fails on the previous logic).

[273] rejected-hover reporting (`file_drop.rs`): a drag the editor refuses
(target disabled, outside the rectangle, wrong file type) reports its hover
position through `rejected_hover_position()` until it leaves or is accepted.
The grid uses it to say WHY next to the bare OS ⊘ cursor (full grid, open
modal). Covered by `rejected_drag_reports_its_position_until_it_leaves`.

[270] hover reporting (`file_drop.rs`): `hover_position()` gives the position
of ANY drag over the editor, accepted or not, until it leaves or drops. The
grid uses it to accept a drop only where it would do something (a full grid
takes a file only on a One-Shot lane) and to warn before a replacement.
Covered by `any_drag_reports_its_hover_position_until_it_leaves_or_drops`.

[282] zoom support (`window.rs`): upstream ignored `Context::set_zoom_factor`
(screen size, pointer and wheel positions in native points, renderer at the
native scale only). The screen rectangle, pointer position and pixel wheel
deltas are now divided by the zoom factor, and the renderer uses
`full_output.pixels_per_point` (native scale x zoom). Flash Drum's Zoom
setting resizes the window and sets the zoom factor together.
