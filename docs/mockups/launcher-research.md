**Build the first version around app launch and inline math.** Use an app only default list, `nucleo-matcher` for fuzzy search, learned ranking, and `fend-core` for calculations. Put other capabilities behind short prefixes. This is a recommendation from the sources below, not a measured comparison of the proposed implementation.

## What to borrow

“Bloat pressure” records a reported complaint where linked; otherwise it names a **design risk**, since I found no clear recurring bloat complaint for that launcher.

| Launcher | Distinctive idea worth borrowing | Bloat pressure |
|---|---|---|
| [Raycast](https://manual.raycast.com/quickstart) | One search field for apps, commands and calculations; searchable actions and user shortcuts. | **Reported:** AI and broader platform features draw feature creep complaints; Raycast also reports a higher v2 memory baseline. [User discussion](https://www.reddit.com/r/raycastapp/comments/1wc07tu/raycast_in_decline/), [Raycast’s figures](https://www.raycast.com/blog/a-technical-deep-dive-into-the-new-raycast). |
| [Alfred](https://www.alfredapp.com/help/overview/) | Keep default results focused; learn which result a user chooses; use keywords for deeper workflows. | Design risk: workflow configuration becoming the main task. Its [default-result controls](https://www.alfredapp.com/help/features/default-results/) are the useful lesson. |
| [Spotlight 26/27](https://support.apple.com/en-tm/guide/mac-help/mchl4953dfeb/mac) | Searchable actions with short *quick keys* such as `ft`, followed by an argument. | Design risk: hundreds of actions crowding app results. Actions, quick keys and clipboard browsing arrived in [macOS 26](https://www.apple.com/nz/os/pdf/All_New_Features_macOS_Tahoe_Sept_2025_NZ_Final.pdf); the current [macOS 27 guide](https://support.apple.com/en-tm/guide/mac-help/mchl4953dfeb/mac) adds Siri requests in Spotlight. |
| [rofi](https://github.com/davatorium/rofi) | `drun`, window, SSH and script modes; composable `combi` search; configurable matching. | Design risk: its many modes and theme options becoming necessary to configure. |
| [fuzzel](https://manpages.debian.org/unstable/fuzzel/fuzzel.1.en.html) | A small Wayland app launcher with a useful dmenu interface. | No clear recurring bloat complaint found; preserve its narrow default scope. |
| [Walker](https://walkerlauncher.com/docs/getting-started) | Discoverable one-character provider prefixes, including `=` for math and `:` for clipboard. | Design risk: the growing provider set spilling into the default list. |
| [anyrun](https://github.com/anyrun-org/anyrun) | Async providers and a small plugin contract; its Rink provider shows unit conversion in a launcher. | Design risk: loading every plugin for every query. |
| [Vicinae](https://docs.vicinae.com/) | Raycast-style actions on Linux, with dmenu and script commands as escape hatches. | Design risk: a full extension platform and rich views for simple launches. |
| [Sherlock](https://github.com/Skxxtz/sherlock) | App aliases, exclusions and category aliases directly address bad search results. | Design risk: widgets and elaborate themes competing with the result list. |
| [Ulauncher](https://ulauncher.io/) | Straightforward fuzzy app search plus keyword shortcuts and extensions. | No clear recurring bloat complaint found; keep extensions out of the core path. |
| [Kunkun](https://github.com/kunkunsh/kunkun) | Cross-platform extension ideas; a permission inspector is useful if extensions ever arrive. | Design risk: an extension store and broad utility catalogue; the project calls itself early stage. |
| [Flow Launcher](https://www.flowlauncher.com/) | Fast app and file lookup backed by the host index, plus web keywords. | Design risk: plugin discovery and configuration overtaking search. |
| [PowerToys Run](https://learn.microsoft.com/en-us/windows/powertoys/run) | Per-provider activation commands and the ability to disable providers in global results. | **Reported:** input delay has an [open issue](https://github.com/microsoft/PowerToys/issues/10429). Its settings explicitly trade waiting for slower plugins against a stable top result. |

Raycast’s core built-ins are a useful **capability inventory**, not a proposed default screen: app search; [calculator](https://manual.raycast.com/calculator) with units, currency, dates and time zones; clipboard history; [window management](https://manual.raycast.com/window-management); file search; quicklinks; [snippets](https://manual.raycast.com/snippets); emoji; system commands; [script commands](https://manual.raycast.com/script-commands); and [AI](https://manual.raycast.com/ai/ai-extensions). Alfred’s built-in calculator covers quick and advanced math, while currency symbols in pasted numbers are ignored rather than converted. [Alfred calculator docs](https://www.alfredapp.com/help/features/calculator/)

## Ranked feature backlog

Costs are **rough engineer-days for a working cross-platform feature**, assuming an existing ratatui input loop; OS integration and polish can extend them. Value reflects the stated app-launch-and-math use case. Ranking is my design judgment.

| Rank | Candidate | Value | Cost |
|---:|---|---|---:|
| 1 | Correct macOS `.app` and Linux desktop-entry discovery and launch | High | 3–6 |
| 2 | Reliable summon, focus, typing and Escape behavior | High | 4–10 |
| 3 | Inline arithmetic with copy-result action | High | 1–2 |
| 4 | Fuzzy app search with clear match highlighting | High | 1–2 |
| 5 | Learned per-query ranking and recency | High | 2–4 |
| 6 | App aliases and initials: `vsc` → Visual Studio Code | High | 1–2 |
| 7 | Unit conversion | High | 1–2 |
| 8 | Predictable app-first ordering; no late result jumps | High | 1–3 |
| 9 | Case and accent tolerant matching | High | 1–2 |
| 10 | User-defined quick keys for apps and commands | High | 2–4 |
| 11 | Tiny command/prefix registry, including `=` | Medium | 2–4 |
| 12 | Explicit typo fallback after exact and fuzzy matches fail | Medium | 2–4 |
| 13 | Basic percent, base and scientific math syntax | Medium | 1–3 |
| 14 | Currency conversion with rate timestamp and cache | Medium | 3–6 |
| 15 | Open URL and named web-search shortcuts | Medium | 1–3 |
| 16 | Opt-in shell/script commands | Medium | 2–5 |
| 17 | Recent apps on an empty query | Medium | 1–2 |
| 18 | Searchable open windows | Medium | 4–8 |
| 19 | Date arithmetic and time-zone conversion | Medium | 3–6 |
| 20 | Narrow file search via OS index or selected roots | Medium | 4–10 |
| 21 | Clipboard history | Low | 5–10 |
| 22 | Emoji and symbol picker | Low | 2–4 |
| 23 | Window placement and system commands | Low | 4–8 |
| 24 | Snippet expansion across apps | Low | 6–12 |
| 25 | AI chat or a general extension store | Low | 15–40+ |

For ranking, score **exact alias and exact name first**, then prefix, word-initial abbreviation, and [`nucleo-matcher`](https://github.com/helix-editor/nucleo) fuzzy score; add a bounded frequency/recency boost and a small penalty for long names. `nucleo` uses fzf-style scoring and handles Unicode; [fzf’s algorithm](https://github.com/junegunn/fzf/blob/master/src/algo/algo.go) rewards word boundaries and camel-case starts. Subsequence matching does **not** fix misspellings, so try edit-distance tolerance only when strong matches are absent. Keep a query-to-selected-app history: selecting Code for `vsc` should reliably teach that exact query. Alfred describes this learned prediction as part of its search behavior. [Alfred](https://www.alfredapp.com/help/features/default-results/fallback-searches/)

## Rust calculator choice

“Speed” is an **integration expectation**, not a benchmark: I found no comparable same-hardware latency results for all five engines. Keep each engine initialized between queries and benchmark the actual launcher path.

| Engine | Units and currency | Precision, integration and license | Fit |
|---|---|---|---|
| [fend / `fend-core`](https://docs.rs/fend-core/latest/fend_core/) | Rich units; currency through a supplied exchange-rate handler. The CLI can fetch and cache rates. [Manual](https://printfn.github.io/fend/documentation/) | Arbitrary-precision rational math; embeddable Rust API; **MIT**. Likely fast enough for a warm inline preview; measure it. | **Best first choice.** Dates are supported; add time zones separately. |
| [Numbat](https://github.com/sharkdp/numbat) | Strong dimension checking and units; ECB exchange-rate support. [Configuration](https://numbat.dev/docs/cli/customization/) | Uses [`f64`](https://docs.rs/numbat/latest/src/numbat/number.rs.html) for numbers; embeddable interpreter; **MIT/Apache-2.0**. More language setup than quick math needs. | Choose for scientific, typed calculations. |
| [`kalk` crate / Kalker](https://github.com/PaddiM8/kalker) | Scientific math; the linked Rust project does not document built-in unit or currency conversion. | Variables, complex numbers, matrices and approximate calculus; **MIT**. More parser capability than this MVP needs. | Good math-focused alternative. *This is distinct from the unrelated kalk.dev calculator.* |
| [`evalexpr`](https://docs.rs/evalexpr/latest/evalexpr/) | Neither units nor exchange rates built in. | Default `i64`/`f64`, customizable numeric types; compact expression evaluator; current docs say **AGPL-3.0**, with other licensing by arrangement. | Only if simple arithmetic is enough and the license fits. |
| [Rink / `rink-core`](https://github.com/tiffany352/rink-rs) | Excellent units and dimensional analysis; currency needs rate data. | Arbitrary precision; source **MPL-2.0**, bundled `definitions.units` **GPL-3.0**. Check data licensing before distribution. | Strong unit engine with a packaging decision. |

## First-keystroke latency

Neither vendor publishes a complete hotkey-to-first-character timing breakdown, so an internal mechanism claim would be speculation. The evidence is narrower: Alfred maintains an [app cache](https://www.alfredapp.com/help/troubleshooting/indexing/) and says missing initial characters commonly indicate a [hotkey conflict](https://www.alfredapp.com/help/troubleshooting/missing-typed-characters/). Raycast describes a resident native shell, lazily loaded components and background file indexing; it also documents work to prevent an empty frame appearing before its UI is drawn. [Raycast technical account](https://www.raycast.com/blog/a-technical-deep-dive-into-the-new-raycast)

For this **terminal UI**, test latency from hotkey through terminal focus, first character echo, and first result separately. Keep the launcher process and app index warm, render the input immediately, and run math and other providers after that render. A ratatui app cannot appear globally without a terminal window, so the terminal’s summon and focus behavior is part of the product’s latency budget.

**Next action (under two minutes):** record `app → result`, `vsc → Visual Studio Code`, and `2 ft to cm → 60.96 cm` as the first three acceptance cases. **Research complete: step 3 of 3.****Recommendation:** build the launcher index in the background and paint each popup from a cached snapshot. Keep app icons out of the scrolling list initially; use a glyph or colored initial, with a real icon for the selected item.

### 1. macOS discovery

Scan `/Applications`, `/System/Applications`, `/System/Applications/Utilities`, `~/Applications`, `/Applications/Nix Apps`, and `~/Applications/Home Manager Apps`. Treat `.app` as a bundle boundary, read `Contents/Info.plist`, and prefer localized `CFBundleDisplayName`, then `CFBundleName`, then the filename. Resolve symlinks into `/nix/store`; resolve Finder aliases separately with Foundation’s [alias URL API](https://developer.apple.com/documentation/foundation/url/init%28resolvingaliasfileat%3Aoptions%3A%29). Deduplicate by resolved path, then handle duplicate bundle IDs according to search path priority. Apple documents the [name keys and `InfoPlist.strings`](https://developer.apple.com/library/archive/documentation/General/Reference/InfoPlistKeyReference/Articles/CoreFoundationKeys.html).

On this Mac, a bounded scan found **118 bundles** and read their plists in **75.5 ms** on the first measured pass, then **26.8–26.9 ms**. That first pass was *not* a verified cold disk scan. Cache names, bundle IDs, paths, and source mtimes under `~/.cache/telmo`; show the cache first and refresh on a background task. Launch Services’ `LSCopyApplicationURLsForURL` is deprecated and returns apps that can open a **particular URL**, not a complete app inventory. `NSWorkspace.urlsForApplications(toOpen:)` has the same selection purpose; use `NSWorkspace` for launch and icon services instead. [Apple Launch Services](https://developer.apple.com/documentation/coreservices/1445148-lscopyapplicationurlsforurl), [NSWorkspace](https://developer.apple.com/documentation/appkit/nsworkspace)

Make System Settings panes separate launch items. This machine has versioned pane IDs in `/System/Applications/System Settings.app/Contents/Resources/Sidebar124.plist`; open validated IDs as `x-apple.systempreferences:<pane-id>`. Keep a small tested list because IDs and routes change across macOS releases. [Apple DTS documents the URL form and a changed macOS 13 route](https://developer.apple.com/forums/thread/791996).

### 2. Icons and terminal rendering

The cheapest macOS integration is `NSWorkspace.shared.icon(forFile:)` in Telmo.app: resize once and cache a small PNG per app. Pure Rust can read an app’s declared `.icns` with [`icns`](https://docs.rs/icns/latest/icns/), but the crate notes unsupported newer entry variants, and it misses icons supplied outside a simple `.icns` path. Cache by resolved bundle path plus bundle/icon mtime under `~/.cache/telmo/icons`.

A **two-cell-high** image consumes two rows per result: roughly ten results fit in Telmo’s 22-row normal popup before other UI. Re-encoding and sending several images on every filter redraw is likely to cause visible latency or stale image placements; that is a design risk to test, not a measured failure. Use glyphs or colored initials for rows and load one selected-item icon asynchronously. Telmo’s [image picker](/Users/daniel/Projects/telmo/crates/kit/src/images.rs) already forces iTerm2 for SwiftTerm via the [host](/Users/daniel/Projects/telmo/host/macos/PopupPanel.swift:96). Ghostty documents **kitty graphics** support; use that on Linux rather than assuming sixel support. [Ghostty features](https://ghostty.org/docs/features)

### 3. Launching and focusing

On macOS, send the selected bundle URL to Telmo.app and call `NSWorkspace.openApplication(at:configuration:completionHandler:)`; its default configuration reuses a running instance. For an already running app, `NSRunningApplication.activate()` can bring it forward. Selecting a **particular window** needs Accessibility APIs, such as its AX window and focused attributes, and may fail for apps that do not expose them. [Open configuration](https://developer.apple.com/documentation/appkit/nsworkspace/openconfiguration/createsnewapplicationinstance), [activation](https://developer.apple.com/documentation/appkit/nsrunningapplication/activate%28options%3A%29), [AX attributes](https://developer.apple.com/documentation/applicationservices/carbon_accessibility/attributes)

On Linux, search `$XDG_DATA_HOME/applications` and every `$XDG_DATA_DIRS/applications`, explicitly including `/run/current-system/sw/share`, `~/.nix-profile/share`, and `/etc/profiles/per-user/$USER/share` when the session omits them. Apply desktop entry precedence and `Hidden`, `NoDisplay`, `TryExec`, and desktop visibility rules. **Do not pass `Exec` through a shell:** its quoting and `%f/%F/%u/%U/%i/%c/%k/%%` expansions have specific rules; `DBusActivatable=true` also changes launch behavior. Prefer GIO’s `GDesktopAppInfo` launcher or `uwsm app -- <desktop-id>` in a UWSM session. For a manually launched command, use a detached process or `systemd-run --user --scope`. To reuse a window, inspect `hyprctl -j clients`, match a desktop entry’s `StartupWMClass` against the client class where available, then `hyprctl dispatch focuswindow address:0x…`. [Desktop Entry Specification](https://specifications.freedesktop.org/desktop-entry/latest-single/), [XDG directories](https://specifications.freedesktop.org/basedir/0.8/), [GIO](https://docs.gtk.org/gio-unix/class.DesktopAppInfo.html), [UWSM](https://wiki.hypr.land/0.50.0/Useful-Utilities/Systemd-start/), [Hyprland example](https://wiki.hypr.land/0.52.0/Configuring/Uncommon-tips--tricks/)

### 4. Matching and frecency

| Choice | Fit for this launcher | License |
|---|---|---|
| [`nucleo-matcher`](https://docs.rs/nucleo-matcher/latest/nucleo_matcher/struct.Matcher.html) | **Pick this:** reusable matcher, score all names, compute highlight indices only for visible results. The full `nucleo` worker is available if the index grows. | [MPL-2.0](https://github.com/helix-editor/nucleo/blob/master/matcher/Cargo.toml) |
| [`fuzzy-matcher`](https://docs.rs/crate/fuzzy-matcher/latest) | Small API with Skim V2 scoring; older crate and [archived repository](https://github.com/skim-rs/fuzzy-matcher/blob/master/Cargo.toml). | MIT |
| [`skim`](https://docs.rs/skim/latest/skim/) | Full finder and UI machinery; useful for its own interface, heavier than needed inside ratatui. | [MIT](https://github.com/skim-rs/skim/blob/master/Cargo.toml) |

There is no defensible cross-crate speed or quality ranking here without benchmarking Telmo’s app names and queries. Persist `{item_id, count, last_opened}` with the existing [locked state writer](/Users/daniel/Projects/telmo/crates/kit/src/state.rs). A simple boost can follow [zoxide’s frecency buckets](https://github.com/ajeetdsouza/zoxide/wiki/Algorithm): count ×4 within an hour, ×2 within a day, ÷2 within a week, ÷4 thereafter. Apply a bounded boost after fuzzy scoring so repeated use does not bury a clearly better text match.

### 5. First frame

The existing [runtime](/Users/daniel/Projects/telmo/crates/kit/src/runtime.rs) draws immediately, and [cache support](/Users/daniel/Projects/telmo/crates/kit/src/cache.rs) is designed for a populated first frame. A PTY probe of the existing `telmo-net --mock` binary saw its **first output bytes in 4–20 ms**, but those bytes were terminal setup queries; the probe did **not** measure a painted frame or the SwiftTerm/Hyprland window appearing. Make the cache the first-frame path, then scan and refresh off the UI thread. Telmo.app can precompute the macOS index while running; a separate daemon is unnecessary until measured popup latency shows the cache approach is insufficient.

**Next action:** prototype the cached, text-only launcher list and record time from popup request to completed first frame.