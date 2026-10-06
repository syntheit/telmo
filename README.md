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
