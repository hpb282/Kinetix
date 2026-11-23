**Kinetix 0.2.0**: first release under the Kinetix name.

- **New name and icon**: the app, window titles, tray menu, notifications,
  executable (`Kinetix.exe`) and package (`Kinetix-0.2.0-windows-x86_64.zip`)
  are now Kinetix, with a new icon.
- **Auto low-power on battery** (General Settings, on by default): while you're
  unplugged or Battery Saver is on, animations run at a lower frame rate, blur and
  caches shrink, and freed memory goes back to Windows. Normal mode returns
  automatically when you plug in.
- **Lighter Music browser**: the hidden Chrome/Edge player turns off background
  features it doesn't use, and in low-power mode it may be throttled while paused.
- **Security**: the Music widget no longer opens a browser while Kinetix is running
  as administrator, and the update helper only overwrites an existing `kinetix.exe`.

Tiếng Việt: Kinetix 0.2.0 có tên và icon mới, tự bật tiết kiệm pin khi rút sạc,
widget Music nhẹ hơn, và vá 2 lỗi bảo mật. Chi tiết: `docs/laptop-optimization-vi.md`.

**Install:** download `Kinetix-0.2.0-windows-x86_64.zip`, extract the whole ZIP
to a writable folder (not Program Files) and run `Kinetix.exe`. You don't need to
install Python, Node or Rust. Music playback needs Chrome or Edge and an internet
connection. Kinetix stores its settings in its own folder, separately from Lumen.

Requires Windows 10/11 x64. The exe is not code-signed, so SmartScreen may warn
about an unknown publisher.

Based on [Lumen](https://github.com/nhatnguyends/Lumen) by Nhat Ng (MIT).
