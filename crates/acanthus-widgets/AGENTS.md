# acanthus-widgets

Reusable widgets that do not depend on the shell binary: `timing` (FAST, BASE, SLOW tokens), `button` (Feel, Press, icon button, focus and flash helpers), `toggle`, `switch`, `slider` (value maths and the `build` node, shared by OSD and system panel), `wheel` (`Wheel` accumulator, `wheel_units`), `marquee`, `scrollbar` (`scroll_handle` maths and the `scroll_bar` node).

Depends only on `acanthus-core`, `acanthus-anim` and `acanthus-scene`. More widgets (slider, text field, popup frame) move here as they are needed by a second module (`.agents/ENGINE.md` E5; engine layer, must end up depending on theme tokens only, E3.4).

- `search_field`: `search_field(scene, parent, keep, SearchField{..})` builds the themed text input plus clear button used by the dock picker, launcher and clipboard.
- `wallpaper_blur`: `backdrop(scene, parent, Spec{..})` adds a rounded clip holding the screen-aligned wallpaper snapshot in a 64 px padded blur layer (QML `WallpaperBlur`, non-liquid path); callers add it under the translucent base fill of a popup and gate it with `theme.glass_active`; the application supplies the watched snapshot URL.
