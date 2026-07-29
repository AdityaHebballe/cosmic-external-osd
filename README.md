# COSMIC External OSD

A small, D-Bus-activated on-screen-display service for local COSMIC extensions.

It runs outside COSMIC Panel so it connects directly to the desktop Wayland display. This is required because panel applets receive a private nested Wayland socket and cannot reliably create desktop-wide layer surfaces.

The helper is shared by the external-monitor-brightness applet and `cosmic-audio-switch`. It is idle between requests and has no polling loop.

## Install

```bash
just build-release
sudo just install
```

The service starts automatically on its first D-Bus request. No logout is needed.

## D-Bus API

Service name: `io.github.cosmic_utils.ExternalMonitorOsd`  
Object path: `/io/github/cosmic_utils/ExternalMonitorOsd`

Brightness, from 0.0 to 1.0:

```bash
busctl --user call io.github.cosmic_utils.ExternalMonitorOsd \
  /io/github/cosmic_utils/ExternalMonitorOsd \
  io.github.cosmic_utils.ExternalMonitorOsd ShowBrightness d 0.5
```

Audio output name and icon:

```bash
busctl --user call io.github.cosmic_utils.ExternalMonitorOsd \
  /io/github/cosmic_utils/ExternalMonitorOsd \
  io.github.cosmic_utils.ExternalMonitorOsd ShowAudio ss Speakers audio-speakers-symbolic
```
