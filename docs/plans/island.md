# Island (planned, week of 2026-10-12)

The rebuild notch pill grows into a general status surface that anything can post to.
Plan it with mockups before building.

## Shape
- **API:** JSON in, like an API call, but no server: `telmo island post '{...}'` over
  Telmo.app's existing socket (and the same JSON from anywhere else later).
- **Pages that stay** (music, weather, ...): cycled with Caps Lock / fn + ←/→. Default is empty.
- **Short events that interrupt** (a finished build, a notification), then fall back.
- **Priority:** a live activity (rebuild) > an event > the chosen page > empty.
- **Click or hotkey expands** into a telmo popup; the island itself stays a glance.
- Style rules from the rebuild pill: nothing drawn in the notch (no pixels), nothing below
  the notch's height, Nix logo left / progress right, no numbers, green only when done.

## Page ideas
- **Music:** album cover left of the notch, an icon (animated or static) right.
- **Spotify playlists:** from the music page, search playlists and add/remove the current song
  (Spotify Web API sign-in).
- **Weather.**
- **Notifications:** replace macOS banners (type icon + preview, expands then hides).
  mantle: be the org.freedesktop.Notifications daemon. swift: read the Notification Center
  database (Full Disk Access) and turn Apple's banners off; spike first.

## Bigger picture
- A notification service on raven that pushes to every device (Telegram etc. even when the
  app isn't open), with the island as its frontend.
- mantle gets the same island as a top-center pill.
