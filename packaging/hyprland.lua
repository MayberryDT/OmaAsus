-- OmaAsus bindings for Hyprland ≥ 0.56 (Omarchy-style Lua config).
-- Append to ~/.config/hypr/bindings.lua
o.bind("SUPER + F12", "OmaAsus overlay", hl.dsp.exec({ cmd = "omaasus toggle" }))
o.bind("SUPER + SHIFT + F12", "OmaAsus window", hl.dsp.exec({ cmd = "omaasus window" }))
o.bind("SUPER + CTRL + G", "OmaAsus: Gaming profile", hl.dsp.exec({ cmd = "omaasus profile gaming" }))
o.bind("SUPER + CTRL + B", "OmaAsus: Balanced profile", hl.dsp.exec({ cmd = "omaasus profile balanced" }))
-- Autostart the overlay daemon (or enable the user unit: systemctl --user enable --now omaasus)
-- o.autostart("omaasus --overlay")
-- Keep OmaAsus fully opaque (Omarchy applies 0.985/0.96 opacity to every window by default)
o.window({ title = "^OmaAsus$" }, { opacity = "1 1" })
-- The overlay panel ("omaasus-overlay" layer) animates itself; Omarchy's default
-- layersIn/layersOut fade layers on top of that. To keep only the panel's own
-- motion, disable the compositor animation for it:
-- hl.layer_rule({ match = { namespace = "omaasus-overlay" }, no_anim = true, animation = "none" })
