# Capture: screenshot + screen recorder (research, not built)

## 1. Current state

**swift (macOS)** - `hosts/swift/skhdrc`, all via `screencapture`:
- Shift+Cmd+S area to clipboard; +D area to ~/Pictures/Screenshots; +A full screen to clipboard; +W window to clipboard.
- Shift+Cmd+X runs `~/.local/bin/screenshot-to-harbor` (defined in `home/darwin.nix`): interactive `screencapture -i` to /tmp, `ssh harbor mkdir -p ~/screenshots/<host>`, `scp` the PNG there, delete local copy, osascript notification. Purpose: let Claude Code on harbor read the shot. It is blocking, has no clipboard path/URL, and stores nothing locally.
- No annotation, no OCR, no recording, no scroll capture. Nothing is in the TCC list for capture except `overview` (kTCCServiceScreenCapture, `modules/darwin/tcc-grants.nix`); Telmo.app has no grant.
- Telmo.app already has `AudioTap.swift` (CoreAudio process taps, `CATapDescription`), but no ScreenCaptureKit code. Clipboard watcher (`ClipboardWatcher.swift`) and the clipboard crate already store/preview images.

**mantle (Hyprland)** - `home/modules/hyprland.nix`:
- Area selection freezes the screen with `hyprpicker -r -z`, then `slurp` + `grim`: Super+S to clipboard, Super+Shift+S to ~/Pictures/Screenshots, Super+A area into Satty (annotate).
- Super+Shift+A full output to clipboard; Super+W / +Shift+W active window (hyprctl geometry) to clipboard / file; Super+O / +Shift+O focused monitor; Super+P hyprpicker color.
- Recording: Super+Shift+R (slurp area) and Super+Alt+R (monitor) toggle `wf-recorder` to ~/Videos/*.mp4 with notify-send. No audio flag, software x264 by default (RTX 2070 SUPER, proprietary driver, so no VAAPI), no gif, no indicator while recording.
- Pain points: each bind is a long inline shell string (duplicated per variant); no history or thumbnail; no OCR; no upload to harbor on Linux; Satty output goes to /tmp and is not saved; window capture ignores overlap/popups; the two hosts have different binds (Cmd vs Super).

## 2. What a great tool does (feature checklist)
Sources: CleanShot X, Shottr, Flameshot, Kooha, Satty, swappy, grimblast, wl-screenrec, OBS.
- Modes: area, window, screen/monitor, previous-area repeat; freeze-frame selection with size readout and arrow-key nudge (Shottr, CleanShot).
- After-capture: floating thumbnail with copy/save/annotate/upload/dismiss (CleanShot); auto-copy and auto-save by default.
- Annotate: arrow, box, text, number badges, highlight, blur/pixelate, crop (Satty, Flameshot, Shottr).
- OCR to clipboard (macOS Vision `VNRecognizeTextRequest`; Linux tesseract), optionally QR.
- Scroll capture (CleanShot, Shottr): stitched; hard, v2+.
- Recording: mp4 and gif, mic and system-audio toggles, region/window/screen, menu-bar/tray indicator, stop hotkey, optional cursor/click highlight (Kooha, OBS-lite).
- Share: upload and copy path/URL. Here: the existing harbor scp flow, generalised (copy remote path to clipboard so it can be pasted into a Claude Code session).
- History: list of recent captures with preview, re-copy, re-annotate, delete. The telmo clipboard already does image preview; captures can simply land in it (or a sibling "Captures" list reading the capture directory).

## 3. Technical plan

**macOS**
- Still: `SCScreenshotManager.captureImage(contentFilter:configuration:)` (macOS 14+) for window/display/rect; for freeze-frame, capture the display first, show it in a borderless full-screen overlay window per screen, and crop from that image (no race with live content).
- Recording: `SCStream` + `SCRecordingOutput` (macOS 15+, writes mp4/mov directly), `capturesAudio` for system audio and `captureMicrophone` (macOS 15+) for mic; reuse the existing AudioTap only if per-app audio is wanted. GIF: encode via ffmpeg/ImageIO from mp4 after the fact.
- `SCContentSharingPicker` gives Apple's own picker for windows, but is mouse-driven and shows a system UI; use only as optional fallback. Own overlay is needed for the keyboard flow.
- OCR: Vision framework, in-process. Annotation: native AppKit/SwiftUI canvas window (not a TUI).
- TCC: Telmo.app needs kTCCServiceScreenCapture (+ kTCCServiceMicrophone for mic). Add rows to `modules/darwin/tcc-grants.nix` for Telmo (SIP is off, so it is declarative like `overview`). Note macOS 15+ shows a periodic "is still recording your screen" re-approval prompt; the TCC.db row may or may not suppress it (open item, test on swift).
- Home: capture engine, overlay and annotation canvas live in Telmo.app (Swift, `host/macos/Capture*.swift`); the telmo popup is the front door. Hotkeys register in the existing `Hotkeys.swift`, replacing skhd's screencapture binds.

**Linux/Hyprland**
- Stills: keep `grim` + `slurp` + `hyprpicker -r -z` freeze (works, wlr-screencopy); `grimblast` or a small telmo Rust wrapper replaces the inline strings. Window geometry from `hyprctl clients -j` (can pick any window under the cursor, not only active). Satty for annotation (already used); swap to `satty --early-exit --copy-command wl-copy --output-filename ...`.
- Recording: `wl-screenrec` is fast but needs VAAPI and is reported unreliable on NVIDIA; `wf-recorder` works but is CPU x264 unless pointed at a VAAPI device. For NVENC on the 2070 SUPER the standard answer is `gpu-screen-recorder` (NVENC, low overhead, has audio mixing via PipeWire, area/window/monitor, replay buffer; use via the portal or KMS). Recommend gpu-screen-recorder for v1 recording on mantle, with wf-recorder `-c libx264 -p preset=ultrafast` as fallback. Verify on mantle before committing.
- Audio: PipeWire monitor source for system audio, default source for mic; both are plain flags.
- OCR: `tesseract` on the grabbed PNG, result piped to wl-copy.
- Portals: grim/slurp need no portal; the xdg-desktop-portal-hyprland screencast portal is only needed for sandboxed apps or OBS, not for this tool.

**Popup vs native**
- telmo popup (TUI, `crates/capture`, same style as clipboard): mode menu (area/window/screen/record/OCR + toggles for mic/system audio/upload/annotate), recent captures list with image preview, actions (copy, open, annotate, upload to harbor, delete, copy harbor path). Shells out to the engine over the Telmo.app socket on macOS, or to grim/etc. on Linux.
- Must be native: selection overlay (macOS NSWindow overlay; Linux slurp is enough), annotation canvas (AppKit on mac; Satty on Linux), recording HUD/indicator, floating thumbnail.

## 4. Scope and effort
**v1 (about 3-4 days):**
- mantle: replace inline binds with one `capture` script/Rust bin (modes, freeze, clipboard/file/Satty, tesseract OCR bit, harbor upload, gpu-screen-recorder toggle with mic/system toggles). ~1 day.
- swift: Telmo.app capture engine on SCScreenshotManager with own freeze overlay, area/window/screen, auto-copy + save to dir, Vision OCR hotkey, harbor upload keeping current behavior (non-blocking, copies remote path), TCC row. ~2 days.
- telmo capture popup listing recent captures with preview (reuse clipboard image view code). ~1 day.

**v2 (about 1-2 weeks):**
- macOS SCStream/SCRecordingOutput recording with mic/system audio and a menu-bar indicator (~2 days); GIF export (~0.5 day).
- Native annotation canvas on swift: arrows, boxes, text, blur (~3-4 days; or pipe to Satty-like minimal tool).
- Floating thumbnail (~1 day), unified keybinds (~0.5 day), scroll capture (~3 days, optional/maybe never).

## 5. Open questions
1. Annotation on swift: build a native canvas, or is "crop + arrow + box + blur" in a small window enough?
2. Should harbor upload stay a separate hotkey, or become a toggle on every capture (always copy remote path)?
3. Is the Cmd/Super keybind mismatch fine, or should both hosts get identical chords (fn-based on swift like launcher/clock)?
4. Recording: do you need gif/audio often enough to make it v1 on swift, or is mantle-only recording OK first?
5. History: store captures under ~/Pictures/Screenshots and list them in a capture popup, or push them into the telmo clipboard store (retention 50 items/30 days would delete them)?
6. OK to depend on gpu-screen-recorder (NVENC, extra package) on mantle rather than staying with wf-recorder?

## Sources
- wl-screenrec: https://www.freshports.org/multimedia/wl-screenrec/
- wf-recorder: https://github.com/any1/wf-recorder
- Hyprland FAQ / screen recording tools: https://wiki.hypr.land/FAQ/
- Apple ScreenCaptureKit docs (SCScreenshotManager, SCRecordingOutput, SCContentSharingPicker): developer.apple.com/documentation/screencapturekit (from knowledge, not fetched)
- gpu-screen-recorder NVENC recommendation: from web search summary and prior knowledge, verify upstream.
