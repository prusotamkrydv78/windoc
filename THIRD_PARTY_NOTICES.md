# Third-party notices

This product is distributed under the GNU General Public License v3.0 (see `LICENSE`). It includes or links the third-party components below. Licence identifiers are given as currently understood. Entries marked **verify** must be confirmed against the generated report before release.

## Inherited source

| Component | Licence | Notes |
|---|---|---|
| WinIsland (`WinIslandProject/WinIsland`), via the BuddyDesk `winisland/` snapshot | GPL-3.0 | The base of this repository. See `UPSTREAM_NOTICE.md`. |
| Rounded-superellipse geometry from Flutter | BSD-3-Clause | Adapted in `crates/winisland-render/src/path.rs`. Licence text: `resources/licenses/flutter.txt`. |

## Rust dependencies (direct)

Transitive dependencies are not listed here. Generate the complete report as described at the end of this file.

| Crate | Used by | Licence |
|---|---|---|
| skia-safe (Skia) | render | MIT (bindings); Skia itself is BSD-3-Clause |
| windows, windows-core, windows-registry, windows-numerics | app, platform-windows, render | MIT OR Apache-2.0 |
| winit | platform-windows | Apache-2.0 |
| tray-icon | app, platform-windows | MIT OR Apache-2.0 |
| wasapi | app, platform-windows | MIT — verify |
| cpal | app | Apache-2.0 |
| realfft | app | MIT — verify |
| tokio, tokio-util | app, platform-windows, plugin-package | MIT |
| reqwest | app, core, plugin-package | MIT OR Apache-2.0 |
| serde, serde_json | app, core, plugin-package | MIT OR Apache-2.0 |
| serde_yaml | app, plugin-package | MIT OR Apache-2.0 (unmaintained upstream — verify replacement) |
| toml | app, core, plugin-api | MIT OR Apache-2.0 |
| quick-xml | app, core | MIT |
| image | app, platform-windows, render, plugin-package | MIT OR Apache-2.0 |
| zip | app, plugin-package | MIT — verify |
| ed25519-dalek | app, plugin-api, plugin-package | BSD-3-Clause |
| sha2 | app, plugin-host, plugin-package | MIT OR Apache-2.0 |
| base64 | app, core, plugin-package | MIT OR Apache-2.0 |
| hex | plugin-package | MIT OR Apache-2.0 |
| libloading | app, platform-windows, plugin-api, plugin-host | ISC |
| dirs | app, platform-windows, plugin-package | MIT OR Apache-2.0 |
| encoding_rs | app, core | (Apache-2.0 OR MIT) AND BSD-3-Clause |
| lrc | app, core | verify |
| fuzzengine | app, core | verify |
| rfd | app | MIT |
| pulldown-cmark | app | MIT |
| log | most crates | MIT OR Apache-2.0 |
| parking_lot | app, core, pollkit, platform-windows | MIT OR Apache-2.0 |
| thiserror | app, render, plugin-package | MIT OR Apache-2.0 |
| educe | core | MIT |
| strum | core | MIT |
| tempfile | plugin-api | MIT OR Apache-2.0 |
| heck, proc-macro2, quote, syn | settings-schema-derive (build time) | MIT OR Apache-2.0 |
| winres | app (build time) | MIT |

## Bundled resources

| Path | Origin | Status |
|---|---|---|
| `resources/icon.png`, `icon-dark.png`, `icon-dark.ico` | Inherited WinIsland application icon | Replace at rebrand; used by the tray, window, About page and installer. |
| `resources/in_app/settings/*.png`, `resources/in_app/icons/sun.*.png` | Inherited from WinIsland | Origin to be confirmed; replace with original icons in the design-system work. |
| `resources/info-en.png`, `resources/info-zh.png` | Inherited | **Not referenced anywhere in the code or packaging.** Recommend removal. |
| `resources/in_app/lang/*.lang` | Inherited and project-authored | Project text; same licence as the code. |
| `resources/licenses/flutter.txt` | Flutter licence text | **Required**: covers the geometry in `crates/winisland-render/src/path.rs`. Keep it. |

## Fonts

No fonts are bundled. Text uses Windows system fonts (for example Segoe UI Variable), which are not redistributed.

## Generating the full report

Run these on the build machine and commit the output alongside a release:

```powershell
cargo install cargo-about cargo-deny
cargo about init
cargo about generate about.hbs > THIRD_PARTY_LICENSES.html
cargo deny init
cargo deny check licenses
```

`cargo deny check licenses` fails when a dependency uses a licence not allowed in `deny.toml`. Allow only licences compatible with GPL-3.0.
