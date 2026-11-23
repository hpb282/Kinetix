# Rà soát bảo mật & tối ưu cho laptop

Bản này dựa trên Lumen v0.1.13 (MIT, © 2026 Nhat Ng — https://github.com/nhatnguyends/Lumen),
được rà soát và chỉnh sửa để chạy nhẹ hơn trên laptop.

## Lỗ hổng / rủi ro đã sửa

| # | Mức | Vấn đề | Cách sửa |
|---|-----|--------|----------|
| 1 | Trung bình | Widget Music chạy Chrome với `--do-not-de-elevate`. Nếu Kinetix bị chạy bằng quyền Administrator (hạ quyền thất bại hoặc dùng `--no-de-elevate`), trình duyệt sẽ tải nội dung web với token admin. | `ensure_not_elevated()` trong `widgets/music/native/backend.rs`: từ chối mở trình duyệt khi Kinetix đang có quyền admin. |
| 2 | Thấp | Tiến trình cập nhật `--apply-update` chép `Kinetix.exe` đè lên đường dẫn lấy từ tham số dòng lệnh; khi không mở được PID cha thì bước kiểm tra bị bỏ qua → có thể ghi đè file bất kỳ mà người dùng có quyền ghi. | `native-dock/src/update.rs`: chỉ chép đè khi đích là file `kinetix.exe` đã tồn tại. |

## Điểm đã kiểm tra và đánh giá là an toàn

- Bộ cập nhật: chỉ HTTPS, danh sách host GitHub cố định, chặn tự chuyển hướng, kiểm SHA-256 cho gói zip và từng file, chặn path traversal (`..`), symlink, có journal để rollback.
- Điều khiển Chrome qua `--remote-debugging-pipe` (không mở cổng debug TCP); máy chủ bootstrap chỉ nghe `127.0.0.1`, đường dẫn ngẫu nhiên, sống rất ngắn.
- Khởi động nhanh dùng Task Scheduler với `/RL LIMITED` (không chạy quyền cao); khóa Run trong registry có đặt dấu ngoặc kép cho đường dẫn.
- Lưu ý còn lại (không sửa được bằng mã): file exe không ký số; mã băm cập nhật lấy từ cùng tài khoản GitHub phát hành (nếu tài khoản bị chiếm thì cập nhật giả vẫn qua được).

## Tối ưu cho laptop (ít RAM, ít CPU, đỡ tốn pin)

1. **Tự bật Low-power khi dùng pin** — mục mới *General Settings › Auto low-power on battery* (mặc định bật).
   Khi rút sạc hoặc bật Battery Saver, Kinetix tự chuyển sang chế độ tiết kiệm (giảm tốc độ khung hình animation,
   giảm blur, giới hạn ảnh nền, xóa cache render và trả bộ nhớ thừa cho Windows). Cắm sạc lại thì tự về bình thường.
   Kiểm tra trạng thái nguồn bằng `GetSystemPowerStatus` trong timer 1 giây sẵn có, không thêm luồng/timer mới.
2. **Chrome ẩn của widget Music nhẹ hơn**
   - Luôn tắt các dịch vụ nền không dùng tới: `--disable-default-apps`,
     `--disable-component-extensions-with-background-pages`, `--disable-features=Translate,OptimizationHints,MediaRouter`.
   - Ở chế độ low-power, không còn ép Chrome “luôn thức” (bỏ 3 cờ chống throttling), để Chrome tự giảm tải
     khi tạm dừng nhạc. Khi đang phát nhạc Chrome vốn không throttle trang có âm thanh nên việc phát không bị ảnh hưởng.
     Thay đổi này áp dụng từ lần mở trình phát nhạc tiếp theo.

## Gợi ý dùng thêm để máy nhẹ nhất

- Chỉ bật những widget thực sự dùng (Music tốn RAM nhất vì chạy kèm một Chrome ẩn).
- Muốn luôn tiết kiệm, bật *Low-power mode*; tắt *Liquid glass*/*Frosted glass* nếu máy yếu.
- *Start with Windows* đang bật mặc định — tắt đi nếu muốn máy khởi động nhanh hơn.

## Build

```
cargo build --release --locked --manifest-path native-dock/Cargo.toml
```

## Bản 0.2.1: sửa giật/lag (đo trên máy thật)

Đo với Clock (afterglow), Music, Calendar, Control Center và App Dock cùng bật,
màn hình 2880×1800, scale 175%:

| Kịch bản | CPU khi rảnh (% một nhân) |
|---|---|
| 0.2.0, cài đặt như người dùng | 30,2% |
| 0.2.0, tắt hiệu ứng động của Clock | 3,6% |
| 0.2.1, cài đặt như người dùng | ~7% |

Nguyên nhân và cách sửa:

1. **Clock "afterglow" vẽ lại toàn bộ widget kính ở tần số màn hình (tới 120 fps)**
   và giữ bộ hẹn giờ 1 ms của Windows, trong khi hình mặt trời/mặt trăng chỉ dịch
   ~2 px mỗi 9 giây. Giờ chạy 20 fps bằng timer thường (`widgets/clock/native/afterglow.rs`).
2. **Music vẽ lại 4 lần/giây khi đang phát nhạc** dù chỉ thanh tiến trình nhích ~1 px/giây.
   Giờ vẽ ngay khi đổi bài/ảnh/trạng thái, còn tiến trình tối đa 1 lần/giây.
3. **Widget biến mất khi bấm/chuột phải ra desktop**: vị trí "neo" trên/dưới bị tính theo
   cả các cửa sổ ẩn (cloaked) và có thể rơi xuống dưới desktop; widget chỉ được ghim lại
   theo timer (Calendar tới 60 giây). Giờ neo tính ngay trên desktop, bỏ qua cửa sổ ẩn,
   và ghim lại tức thì khi cửa sổ đang chọn thay đổi (`native-dock/src/desktop_layer.rs`).
4. **Kéo widget bị giật**: liquid glass tốn 13–33 ms/khung khi hình dạng thay đổi
   (ngân sách 8,3 ms ở 120 Hz). Khi kéo, kính dùng kiểu frosted nhẹ; thả ra thì về đầy đủ.
5. **RAM tăng dần theo thời gian sử dụng** (~370 MB): khi bạn dùng app khác, Kinetix trả bộ nhớ
   không dùng cho Windows tối đa mỗi 90 giây → RAM thường trú còn ~18 MB.
