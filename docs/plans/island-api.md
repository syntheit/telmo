# Island API and arbitration (design draft, for discussion; nothing implemented)

Builds on `docs/plans/island.md`. Mockups: `docs/mockups/island-*.txt`.
Current code: `RebuildIsland.swift` (window, geometry: pill = notch height, wings 78 pt, bars 2.4x12 pt, pitch 5),
`RebuildStatus.swift` (`IslandContent`, `IslandTracker`), `IPC.swift` (one line, max 1024 bytes, `ok`/`error ...` reply),
`ClockWatcher.swift` (rings from `clock.json`, no pill support yet).

## 1. JSON API

CLI: `telmo island post '<json>'` | `post -` (stdin) | `clear <id>` | `list` | `next` / `prev` (page cycling) | `open [id]`.
`post` is an upsert by `id`: re-posting updates in place (a timer or progress ticks this way, no animation restart if unchanged).

```
{ "id": "rebuild",                 // required, [a-z0-9._-]{1,64}; owner namespace by convention (spotify.now, tg.anna)
  "kind": "live" | "event" | "page",
  "title": "Building",             // <=60 chars; popup/list/accessibility label, not always drawn
  "text":  "Anna: lunch?",         // <=80 chars; right-wing text for events/pages (clipped, 1 line)
  "icon":  "sf:snowflake" | "/abs/path.png" | "app:com.spotify.client" | "nix",  // left wing; "nix" = built-in snowflake
  "accent": "#5b86d6",             // icon/bars tint; default neutral grey 9aa0b8; green reserved for done (see 3)
  "progress": 0.6 | "indeterminate",  // omit = none
  "bars": "bars" | "level" | "none",  // right wing glyph: 10 progress bars (default if progress), 5-bar audio level, nothing
  "ttl": 4,                        // seconds; events default 4, live/page default none
  "endsAt": 1760100000,            // optional unix s; island itself computes progress/drain for timers (no per-second posts)
  "priority": 0,                   // -10..10 tie-break inside a kind only; kind order is fixed
  "done": {"text": "Tea is done", "ttl": 6, "ok": true},   // optional: replaces this item with a finished event on clear
  "action": {"popup": "clock"} | {"command": ["telmo","popup","net"]} | {"open": "https://..."}  // click / `telmo island open`
}
```
Rules: unknown keys are ignored (forward compatible); wrong types or a bad `id` make the whole post fail with `error <reason>`.
`expiry`: an item is removed at `ttl`, at `endsAt` (plus `done`), or by `clear`. A live item whose poster vanished needs `ttl` as a
dead-man switch (rebuild posts `ttl: 15`, refreshed each tick; or the host keeps reading `rebuild.json` as today and synthesizes the item).
`action.command` runs argv-only (no shell), looked up with `ModuleLookup.findExecutable`; icons by path must be readable files <=256 KB.

Examples
```
rebuild  {"id":"rebuild","kind":"live","icon":"nix","accent":"#5b86d6","progress":0.6,"ttl":15,"action":{"popup":"system"}}
timer    {"id":"clock.tea","kind":"live","title":"Tea","icon":"sf:timer","accent":"#7ebae4","endsAt":1760100300,
          "done":{"text":"Tea is done","ttl":6},"action":{"popup":"clock"}}
spotify  {"id":"spotify.now","kind":"page","title":"Song - Artist","icon":"/tmp/telmo/cover.png","bars":"level","action":{"popup":"music"}}
weather  {"id":"weather","kind":"page","title":"Berlin","icon":"sf:cloud.sun","text":"14 C","action":{"popup":"weather"}}
telegram {"id":"tg.anna","kind":"event","icon":"sf:paperplane.fill","text":"Anna: lunch?","ttl":4,"action":{"open":"tg://"}}
```

Transport: same socket, same line protocol: `island post <json>` as one line, reply `ok` or `error <reason>`. Needed changes: the 1024-byte
line cap is too small for covers/text, so raise it to 16 KB for `island` lines only and keep large images as file paths, never inline.
Handled off the main thread for parsing, applied on main. Who may post: the socket is `0600` in a `0700` directory, so only the local user
(and root); that is the whole trust model, same as the passwords that already travel there. Rate limit 20 posts/s per connection; max 32 live items.
Remote posters (raven) reach it through `ssh`/tailscale running `telmo island post` locally, never by exposing the socket.

## 2. Arbitration

Order is fixed: live > event > selected page > nothing. Within a kind: higher `priority`, then most recent post.
- Live: only the top live item shows; others wait (rebuild beats timer). A finished live item hands over to its `done` event.
- Event: shows for `ttl`, then the island falls back to the next layer, no queue pile-up: a burst keeps only the newest 3, shown in order,
  each 4 s; an event never replaces a live item (it is parked and shown when the live item ends, if still fresh, <60 s).
- Pages: the set of posted pages is the ring. `next`/`prev` rotate it; the selection is remembered per session. Selecting "nothing" is part of the ring
  (slot 0 = empty). A page that is cleared drops out of the ring. Default after boot: empty (slot 0).
- Nothing posted or selected: no window at all (as the pill is today), notch untouched. System popup open: island yields, as today.
- Cycling keys: Carbon `RegisterEventHotKey` (Hotkeys.swift) needs a non-empty modifier set plus an ANSI letter/digit in its `keyCodes` table, so
  it cannot bind Caps Lock, fn, or arrow keys as written. Options: (a) extend the table with arrow codes and bind `ctrl+opt+cmd+left/right`
  (no permission, but a clumsy chord); (b) Karabiner (already on swift) maps `fn+left/right` or Caps+arrows to `telmo island prev/next`
  via `shell_command` (recommended: no new permission, the key choice stays outside Telmo); (c) a `CGEventTap` inside Telmo.app (needs Input Monitoring TCC;
  heavier). Proposal: (b) now, (a) as built-in fallback. On Hyprland: `bind = SUPER, left/right, exec, telmo island prev/next`.

## 3. Rendering rules (from the rebuild pill)

- Window: borderless, `.statusBar` level, all Spaces, click-only (`canBecomeKey` false), height exactly the notch height, so nothing hangs below it.
  Pill width = notch + 2 x 78 pt wings; wings grow up to 120 pt only for text. Black fill, bottom corners rounded 14 pt, top flush.
  Nothing is drawn in the notch (the camera strip is physically black, so the wings read as one shape). No notch (external display): the 170 x 34 capsule, halves as wings.
- Left wing = identity: one icon (20 pt), tinted by `accent`, or a cover thumbnail (rounded 4 pt). Right wing = state: bars glyph, or one short text line (11 pt, white 85%), or nothing.
  Never both text and bars. No numbers or percentages in the island; counts and times belong in the popup.
- Colour: accent per source; `#9ece6a` green means done only, `#f7768e` failed, nothing else uses them.
- Motion: appear = grow out of the notch (0.35 s ease-out), retract = shrink back; content change cross-fades 0.25 s; animated glyphs honor Reduce Motion (static at 0.4 to 0.5 opacity), as `IslandView` does.
- Click or hotkey: runs `action` (popup minimizes into the notch on close, as the System popup does). Hover does not expand: the island stays a glance;
  all depth lives in popups. Open for later: a 2 s hover tooltip with the title.
- Refactor sketch: `IslandView` takes an `IslandItem` (icon, accent, right = bars|level|text|none) instead of `IslandContent`; the rebuild tracker becomes
  the first producer of items, so rebuild keeps today's exact look.

## 4. Linux (mantle, Hyprland)

A top-center layer-shell surface (`layer = overlay`, anchor top, exclusive zone 0, no keyboard focus), same geometry minus the notch: capsule 170 x 34, 
wing halves, bottom corners rounded, black. Options:
- Tiny Rust GTK4 app + gtk4-layer-shell (`telmo-island`, in the workspace): recommended. Same language as the CLI, no scripting layer, one binary the CLI talks to over
  `$XDG_RUNTIME_DIR/telmo-island.sock` with the same JSON lines; it also owns arbitration (a shared Rust crate `island-core` could be used by both ports later).
  Hyprland layer animations may need disabling for the surface (known gtk-layer-shell issue).
- Quickshell (Qt/QML, auto-reload, good Hyprland integration, has a notifd-style notification backend): most flexible UI, but adds a QML runtime and a second place with logic.
- eww (GTK3, Yuck + scripts): works, but leans on scripts and GTK3 is not GPU-accelerated; poor fit for animation.
`telmo island ...` on Linux just writes to that socket; JSON, ids, arbitration and rules are identical, so a poster does not know the platform.
Notifications: the Linux app claims `org.freedesktop.Notifications` on the session bus (zbus), and converts each `Notify` call into an `event` item
(`app_name` to icon, `summary` + body to text, `expire_timeout` to ttl, actions to `ActionInvoked`). It must honor `CloseNotification`, replaces_id (upsert by id `dbus.<n>`),
and urgency (critical = no auto-hide until clicked). Replacing mako/dunst means disabling theirs in the mantle config. Precedent: notifd, a Rust daemon with a UI-over-socket split.

## 5. Sources (wired later, each a small producer posting JSON)

- Spotify, macOS: `MediaRemote` (private, now entitlement-gated on recent macOS; spike) or Spotify's AppleScript (`tell application "Spotify" to name of current track`,
  artwork URL via `artwork url`), polled 1/s only while it runs, or the `com.spotify.client.PlaybackStateChanged` distributed notification (cheap, event-driven; recommended). Playlists: Spotify Web API (OAuth PKCE).
- Spotify/any player, Linux: MPRIS over D-Bus (`org.mpris.MediaPlayer2.*`, `PropertiesChanged` on `Metadata`/`PlaybackStatus`; `mpris:artUrl` for covers). `playerctl -F` is the shell-level equivalent.
- Weather: Open-Meteo, no key: `https://api.open-meteo.com/v1/forecast?latitude=..&longitude=..&current=temperature_2m,weather_code`; refresh every 15 min; weather_code maps to an SF Symbol.
- Telegram and the rest: a notification relay on raven (receives Telegram/bot/webhook events, keeps one persistent connection per device, e.g. over tailscale) pushing `{"kind":"event",...}` to a small
  `telmo island listen` client on each device, which just calls `post`. Sketch only; needs a decision on transport (SSE vs websocket) and dedupe per device.
- macOS notifications: Notification Center DB read needs Full Disk Access and is private format: spike before committing. Timers: `ClockWatcher` posts `endsAt` items when a timer starts (the file watch already exists).

## 6. Open questions

1. Hotkeys: is Karabiner (`fn`/Caps + arrows calling `telmo island next/prev`) acceptable, or must it be built into Telmo.app?
2. Should timers and rebuild share the one live slot, or may two live items split the wings (rebuild left bars, timer right)?
3. Is an event allowed to interrupt a live rebuild (e.g. a critical alarm), or never?
4. Notifications on swift: commit to the Full Disk Access spike, or keep Apple's banners and only mirror Telegram via raven?
5. Linux implementation: Rust GTK4 app (proposed) or Quickshell, given mantle may want a bar later?
6. Cover art and text in wings: OK to break "no text" for events (one clipped line), or icon-only plus the popup?

Sources: gtk4-layer-shell https://github.com/wmww/gtk4-layer-shell , Hyprland layer-animation discussion https://github.com/hyprwm/Hyprland/discussions/7355 ,
notifd https://github.com/gofroshka/notifd , Arch wiki Desktop notifications https://wiki.archlinux.org/title/Desktop_notifications , Hyprland status bars https://wiki.hypr.land/0.51.0/Useful-Utilities/Status-Bars/ .
The Spotify, MPRIS, Open-Meteo and MediaRemote details are from memory and were not re-verified online; check before building.
