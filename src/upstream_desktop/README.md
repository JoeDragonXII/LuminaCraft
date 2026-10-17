# Desktop host provenance

These Rust modules were copied from story told LightCraft's `apps/lightcraft/src` at commit b3d392b4994ee99dc389890f006600769d0c4ff5. LuminaCraft uses the upstream MIT license; see ../../upstream/lightcraft/LICENSE-MIT and NOTICE. The fixed upstream copy remains in ../../vendor/lightcraft.

Owned changes: entry point and window branding, isolated preferences/library, Chinese system font, default grid, no demo seed, no-overwrite export writer, and the companion project workflow button. The upstream child-process test's module-qualified test name is adapted to its new location. Platform services, render fallback, preferences, dialogs and the control server otherwise remain the full upstream host.
