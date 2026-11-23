**Kinetix 0.2.1**: smoothness and battery release.

Measured on a laptop running the Clock (afterglow), Music, Calendar, Control
Center and App Dock widgets together, at 2880×1800 and 175% scaling:

| | 0.2.0 | 0.2.1 |
| --- | --- | --- |
| Idle CPU (share of one core) | ~30% | ~7% |
| Resident memory after 90 s in the background | ~190 MB (grew to ~370 MB with use) | ~18 MB |

- **Clock**: the "afterglow" sun/moon animation repainted the whole glass widget
  at the display refresh rate (up to 120 fps) and kept Windows' 1 ms timer
  resolution active, even though its motion is only a few pixels per second.
  It now runs at 20 fps with the normal system timer.
- **Music**: while a track plays, the widget repainted four times a second.
  It now repaints immediately when the track, cover or playback state changes,
  and at most once a second for the progress bar.
- **Widgets no longer vanish when you click the desktop**: widgets now stay
  directly above the desktop when Windows reshuffles windows, and they re-pin
  as soon as focus changes. Previously some only fixed themselves on a timer,
  up to 60 s later for the Calendar.
- **Smoother dragging**: while you drag a widget, its glass uses the light
  frosted style, then full liquid glass returns when you drop it. Redrawing
  liquid glass took 13–33 ms per frame, over the 8 ms budget at 120 Hz.
- **Lower memory**: while you use other apps, Kinetix returns its unused
  memory to Windows, at most every 90 s.

Tiếng Việt: bản 0.2.1 giảm CPU khi rảnh từ ~30% xuống ~7% một nhân, RAM thường trú
còn ~18 MB, widget không còn biến mất khi bấm ra desktop, kéo widget mượt hơn.

**Install:** download `Kinetix-0.2.1-windows-x86_64.zip`, extract the whole ZIP
to a writable folder (not Program Files) and run `Kinetix.exe`. Requires
Windows 10/11 x64. The exe is not code-signed, so SmartScreen may warn about an
unknown publisher.

Based on [Lumen](https://github.com/nhatnguyends/Lumen) by Nhat Ng (MIT).
