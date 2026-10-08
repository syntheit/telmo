# telmo

Keyboard-driven popups for the things you change all day: **network**,
**bluetooth** and **sound**. Small terminal UIs that open centered over a
dimmed screen, do one job, and close with `esc`. Same look and keys on macOS
and Linux.

| Module | What it does |
|---|---|
| `telmo-net` | Wi-Fi (scan, join, forget, power), Ethernet IPv4/DNS, VPNs (Tailscale, system/NetworkManager), speedtest (`s`) |
| `telmo-bt` | Paired devices, connect/disconnect, scan and pair (with codes), battery |
| `telmo-sound` | Output and input devices, volume, mute, per-app volume and routing (Linux), headset mode |
| `telmo-system` | Apple-menu style popup: an animated logo, lock, sleep, restart, shut down, log out |

Each module is a standalone binary that works in any terminal. `telmo <module>`
runs one; `telmo popup <module>` opens it as a popup.

## The popup

- **macOS:** `Telmo.app` is a tiny resident host: a rounded glass panel with an
  embedded terminal, a dim layer behind it, and global hotkeys
  (`⌃⌥⌘N`, `⌃⌥⌘B`, `⌃⌥⌘M` by default; override in `~/.config/telmo/hotkeys`,
  e.g. `net = ctrl+opt+cmd+n` or `none`). No yabai, skhd or Accessibility
  permission needed. It also holds the Location (Wi-Fi names) and Bluetooth
  permissions for the modules it starts.
- **Linux (Hyprland):** `telmo popup net` toggles a terminal window with class
  `telmo.net`; the home-manager module adds the window rules and binds.

## Custom popups

Any TUI can open as a popup. Describe it in `~/.config/telmo/popups.json`
(hand-editable; re-read on every open) or let home-manager write it:

```nix
programs.telmo = {
  popups.perf = { command = [ "btop" ]; size = "large"; escape = "close"; };
  hyprland.binds.perf = "SUPER, P";
};
```

Then `telmo popup perf` toggles it, `telmo list` shows it, and on macOS
`perf = ctrl+opt+cmd+p` in `~/.config/telmo/hotkeys` binds it.

- `command`: program and arguments, found like the built-in modules (PATH).
- `size`: `normal` (90×22 cells, default) or `large` (80% of the screen's
  visible width and height).
- `escape`: `pass` (default, the TUI handles Esc) or `close` (macOS: the host
  closes the popup on Esc; for TUIs like btop whose own Esc opens a menu).

## Install (Nix)

```nix
# flake inputs
telmo.url = "github:syntheit/telmo";

# home-manager
imports = [ inputs.telmo.homeManagerModules.default ];
programs.telmo = {
  enable = true;
  # macOS: sign a copy of Telmo.app so permissions survive rebuilds
  signingIdentity = "Developer ID Application: …";
  # Hyprland
  hyprland.binds = { net = "SUPER, N"; bt = "SUPER, B"; sound = "SUPER, M"; };
};
```

Building `Telmo.app` uses the Xcode Command Line Tools' `swiftc` (macOS 26 SDK).

## Privileged helper (optional)

macOS keeps per-network Wi-Fi auto-join in a root-only file. `telmo-helper` is a
small root launchd daemon that lets the net popup show and toggle it without a
prompt. It listens on `/var/run/telmo-helper.sock` (mode 0600, owned by you),
checks the connecting process's user and its Developer ID code signature (your
team ID), and answers exactly two requests: list auto-join per saved network,
and set it for a network that's already saved. Without it the popup opens
Wi-Fi settings instead.

```nix
# nix-darwin
imports = [ inputs.telmo.darwinModules.default ];
services.telmo-helper = { enable = true; user = "you"; teamId = "ABCDE12345"; };
```

`user` needs an explicit `users.users.<name>.uid`. Telmo.app must be signed with
that team's Developer ID (see `signingIdentity` above).

## Keys

`j`/`k` move · `↵` primary action · `tab` next pane · `i` details · `?` all keys ·
`esc` close dialog, then quit.

## Develop

```sh
nix develop -c cargo run -p telmo-net -- --mock   # fake data
nix develop -c cargo run -p telmo-net -- status   # one real snapshot as JSON
nix develop -c cargo insta test                   # snapshot tests
```

See `AGENTS.md` for how the code is organized and `docs/mockups/` for every
screen.
