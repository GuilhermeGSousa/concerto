# Wonderland UI Showcase

Run the standalone showcase without a project or imported content:

```sh
cargo run -p ui-showcase
```

The showcase is the visual reference for the Looking Glass palette,
typography, spacing, nested flex layout, controls, Unicode input, clipping and
virtual lists. Its 10,000-item sample creates eight reusable row entities.

Manual checks:

- Resize the window from very narrow to wide; neither panel should collapse
  below its stated minimum and the diagnostics must update.
- Check the layout at 100%, 150%, and 200% desktop scale. Logical dimensions
  should change independently from the physical framebuffer dimensions.
- Confirm every swatch remains legible, the Unicode sample renders, and the
  centered row keeps even gaps and cross-axis alignment.
- Confirm the diagnostics strip stays pinned below the flexible body.
- Tab and Shift-Tab through controls. Verify focus, typing, selection,
  Backspace/Delete, Home/End, and Ctrl/Cmd+A/C/X/V.
- Drag the slider out of its bounds and release; capture must keep the drag
  active and must not emit a click after crossing the drag threshold.
- Scroll the virtual list and confirm the visible range changes while its UI
  entity count remains constant.
