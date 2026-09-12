# OmaAsus identity

An original OA monogram: a split, chamfered O frames a central A. The enclosure suggests a controlled hardware system; the open seams keep the silhouette light and legible. The design uses straight edges and deliberate negative space to complement the Omarchy-inspired interface without reusing its logo.

## Assets

- `omaasus-mark.svg`: transparent, full-color vector master.
- `identity.svg` and `identity.png`: repository presentation artwork. The editable SVG uses Geist with a sans-serif fallback; the PNG preserves the rendered presentation.
- `../../crates/oma-gui/assets/icons/omaasus.svg`: application icon with a dark ground.
- `../../crates/oma-gui/assets/icons/omaasus-symbolic.svg`: transparent monochrome source, also used by the GUI with the active theme color.
- Adjacent PNG files: 16, 22, 24, 32, 48 and 64 px tray compatibility assets.

## Usage

Primary colors: jade `#76c6a4`, ivory `#f3edce`, dark green `#101b17`. Use the monochrome version on light backgrounds and wherever the desktop controls icon coloring. Keep clear space at least one enclosure stroke (96 units) outside the visible mark. Use the supplied icon canvas at small sizes; do not add shadows, outlines, gradients, or distort its proportions. Minimum intended icon canvas is 16 px; prefer 24 px or larger where space permits.

The GUI loads the symbolic SVG directly rather than maintaining separate canvas geometry. When editing the icon SVGs, regenerate all compatibility PNGs with `rsvg-convert -w SIZE -h SIZE -o OUTPUT INPUT`. Review at native sizes on dark and light backgrounds before shipping.

Artwork is provided under the repository MIT license. This is an independent project identity, not an ASUS or Omarchy endorsement.
