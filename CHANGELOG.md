# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
While pre-1.0, the minor version is bumped for breaking changes.

<!-- next-header -->

## [Unreleased] - ReleaseDate

### Fixed

- `include_svg!` now frames a baked image by the artwork's own bounding box and
  maps every contour through its `abs_transform` (the full ancestor chain,
  viewBox→viewport scale included). This fixes SVGs that previously baked
  mis-scaled or off-frame: a viewBox with a non-zero origin, an `<svg>` whose
  width/height differ from the viewBox, or a drawing tucked into one corner of a
  large canvas.

### Added

- `include_svg!` now honors `clip-path`: clip regions are intersected with the
  drawn geometry at build time (via `i_overlay`), so clipped artwork bakes
  correctly instead of being dropped. `i_overlay` is a compile-time-only
  dependency of `saudade-macros` and never reaches a shipped binary.
- `include_svg!` now approximates linear and radial gradient paint instead of
  dropping it: each gradient bakes into a stack of flat-color bands (strips for
  linear, nested disks for radial) clipped to the painted shape. Gradient fills
  and strokes are no longer reported as unsupported.

## [0.2.0] - 2026-06-06

### Added

- Compile-time SVG support: the `include_svg!` macro bakes an SVG into
  flattened polygons at build time and expands to a `const SvgImage`, so no SVG
  parser is linked into the binary. The parsing weight (usvg + kurbo) is
  confined to the new `saudade-macros` proc-macro crate and runs only at compile
  time. (#22)
- `Modal::on_cancel()` to run a handler when a modal is dismissed; `App` now
  also invokes `on_cancel()` for top-level windows. (#18, #19)
- `Container::layout()` for custom child layout. (#17)
- Nested popups — a popup opened from within another popup. (#11)
- Painting at physical size, with a dedicated code path for crisper rendering at
  1.25× scale. (#13)
- The system scale factor is now readable on Wayland. (#16)

### Changed

- Reworked the dialog architecture. (#21)
- Refreshed checkbox and dropdown styling. (#24, #25)

### Fixed

- Buttons now activate on key release rather than on key press. (#14)
- Corrected button autorepeat behavior. (#20)
- FreeBSD: load fonts from `/usr/local/share/fonts`. (#12)

## [0.1.0] - 2026-06-01

Initial release.

<!-- next-url -->
[Unreleased]: https://github.com/roblillack/saudade/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/roblillack/saudade/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/roblillack/saudade/releases/tag/v0.1.0
