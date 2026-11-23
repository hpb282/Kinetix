//! Small, unblended background tiles. No content, desktop capture or alpha fade.
use super::geometry::Pane;
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Material {
    pub width: usize,
    pub height: usize,
    pub is_delta: bool,
    pub is_overlay: bool,
    pub bgr: Vec<[u8; 3]>,
    pub dbgr: Vec<[i16; 3]>,
    pub overlay: Vec<[u8; 4]>,
    pub gain_q: u16,
}
impl Material {
    pub fn gain(&self) -> f32 {
        f32::from(self.gain_q) / 65535.0
    }
    pub fn is_opaque(&self) -> bool {
        !self.is_delta && !self.is_overlay
    }
}
pub struct Field {
    pub window: [i32; 4],
    pub width: usize,
    pub height: usize,
    pub is_delta: bool,
    pub bgr: Vec<[f32; 3]>,
}
impl Field {
    pub fn sample(&self, x: f32, y: f32) -> [f32; 3] {
        let u = ((x - self.window[0] as f32) / (self.window[2] - self.window[0]) as f32
            * self.width as f32
            - 0.5)
            .clamp(0., (self.width - 1) as f32);
        let v = ((y - self.window[1] as f32) / (self.window[3] - self.window[1]) as f32
            * self.height as f32
            - 0.5)
            .clamp(0., (self.height - 1) as f32);
        let (x0, y0) = (u.floor() as usize, v.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.width - 1), (y0 + 1).min(self.height - 1));
        let (fx, fy) = (u - x0 as f32, v - y0 as f32);
        std::array::from_fn(|c| {
            let a = self.bgr[y0 * self.width + x0][c] * (1. - fx)
                + self.bgr[y0 * self.width + x1][c] * fx;
            let b = self.bgr[y1 * self.width + x0][c] * (1. - fx)
                + self.bgr[y1 * self.width + x1][c] * fx;
            a * (1. - fy) + b * fy
        })
    }
}
pub struct OverlayField {
    pub window: [i32; 4],
    pub width: usize,
    pub height: usize,
    pub bgra: Vec<[f32; 4]>,
}
impl OverlayField {
    pub fn sample(&self, x: f32, y: f32) -> [f32; 4] {
        if self.width == 0 || self.height == 0 || self.bgra.len() < self.width * self.height {
            return [0.0; 4];
        }
        let u = ((x - self.window[0] as f32) / (self.window[2] - self.window[0]).max(1) as f32
            * self.width as f32
            - 0.5)
            .clamp(0., (self.width - 1) as f32);
        let v = ((y - self.window[1] as f32) / (self.window[3] - self.window[1]).max(1) as f32
            * self.height as f32
            - 0.5)
            .clamp(0., (self.height - 1) as f32);
        let (x0, y0) = (u.floor() as usize, v.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.width - 1), (y0 + 1).min(self.height - 1));
        let (fx, fy) = (u - x0 as f32, v - y0 as f32);
        std::array::from_fn(|c| {
            let a = self.bgra[y0 * self.width + x0][c] * (1. - fx)
                + self.bgra[y0 * self.width + x1][c] * fx;
            let b = self.bgra[y1 * self.width + x0][c] * (1. - fx)
                + self.bgra[y1 * self.width + x1][c] * fx;
            a * (1. - fy) + b * fy
        })
    }
}
pub struct FrostField {
    pub window: [i32; 4],
    pub width: usize,
    pub height: usize,
    pub values: Vec<f32>,
}
impl FrostField {
    pub fn sample(&self, x: f32, y: f32) -> f32 {
        if self.width == 0 || self.height == 0 || self.values.len() < self.width * self.height {
            return 1.0;
        }
        let u = ((x - self.window[0] as f32) / (self.window[2] - self.window[0]).max(1) as f32
            * self.width as f32
            - 0.5)
            .clamp(0., (self.width - 1) as f32);
        let v = ((y - self.window[1] as f32) / (self.window[3] - self.window[1]).max(1) as f32
            * self.height as f32
            - 0.5)
            .clamp(0., (self.height - 1) as f32);
        let (x0, y0) = (u.floor() as usize, v.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.width - 1), (y0 + 1).min(self.height - 1));
        let (fx, fy) = (u - x0 as f32, v - y0 as f32);
        let a =
            self.values[y0 * self.width + x0] * (1. - fx) + self.values[y0 * self.width + x1] * fx;
        let b =
            self.values[y1 * self.width + x0] * (1. - fx) + self.values[y1 * self.width + x1] * fx;
        a * (1. - fy) + b * fy
    }
}
fn area_filter_cell(
    width: usize,
    height: usize,
    pixels: &[u8],
    tw: usize,
    th: usize,
    x: usize,
    y: usize,
) -> [u8; 3] {
    let (l, r, t, b) = (
        x * width / tw,
        (x + 1) * width / tw,
        y * height / th,
        (y + 1) * height / th,
    );
    let mut sum = [0u64; 3];
    for sy in t..b {
        for sx in l..r {
            let i = (sy * width + sx) * 4;
            for c in 0..3 {
                sum[c] += pixels[i + c] as u64;
            }
        }
    }
    let n = ((r - l) * (b - t)).max(1) as u64;
    sum.map(|v| ((v + n / 2) / n) as u8)
}

fn soften_tile<T: Copy, const N: usize>(
    tile: &mut Vec<[T; N]>,
    tw: usize,
    th: usize,
    to_i32: fn(T) -> i32,
    from_i32: fn(i32) -> T,
) {
    // A small positive Gaussian kernel softens the reflection, without
    // ringing, alpha changes or overshoot into brighter/darker colors.
    let mut scratch = tile.clone();
    for axis in 0..2 {
        for y in 0..th {
            for x in 0..tw {
                let mut sum = [0i32; N];
                for (k, w) in [1i32, 2, 1].into_iter().enumerate() {
                    let sx = if axis == 0 {
                        (x as isize + k as isize - 1).clamp(0, tw as isize - 1) as usize
                    } else {
                        x
                    };
                    let sy = if axis == 1 {
                        (y as isize + k as isize - 1).clamp(0, th as isize - 1) as usize
                    } else {
                        y
                    };
                    for c in 0..N {
                        sum[c] += to_i32(tile[sy * tw + sx][c]) * w;
                    }
                }
                scratch[y * tw + x] = sum.map(|v| from_i32(v));
            }
        }
        std::mem::swap(tile, &mut scratch);
    }
}

fn sample_grid<T: Copy, const N: usize>(
    grid: &[[T; N]],
    width: usize,
    height: usize,
    window: [i32; 4],
    x: f32,
    y: f32,
    to_f32: fn(T) -> f32,
) -> [f32; N] {
    if width == 0 || height == 0 || grid.len() < width * height {
        return [0.0; N];
    }
    let u = ((x - window[0] as f32) / (window[2] - window[0]).max(1) as f32 * width as f32 - 0.5)
        .clamp(0., (width - 1) as f32);
    let v = ((y - window[1] as f32) / (window[3] - window[1]).max(1) as f32 * height as f32 - 0.5)
        .clamp(0., (height - 1) as f32);
    let (x0, y0) = (u.floor() as usize, v.floor() as usize);
    let (x1, y1) = ((x0 + 1).min(width - 1), (y0 + 1).min(height - 1));
    let (fx, fy) = (u - x0 as f32, v - y0 as f32);
    std::array::from_fn(|c| {
        let a =
            to_f32(grid[y0 * width + x0][c]) * (1. - fx) + to_f32(grid[y0 * width + x1][c]) * fx;
        let b =
            to_f32(grid[y1 * width + x0][c]) * (1. - fx) + to_f32(grid[y1 * width + x1][c]) * fx;
        a * (1. - fy) + b * fy
    })
}

impl Material {
    /// Area filtering prevents individual grain/caustic pixels from becoming
    /// large bright cells when the low-resolution fusion tile is enlarged.
    pub fn from_bgra(width: usize, height: usize, pixels: &[u8]) -> Self {
        let (tw, th) = (width.min(64), height.min(64));
        let mut bgr = Vec::with_capacity(tw * th);
        for y in 0..th {
            for x in 0..tw {
                bgr.push(area_filter_cell(width, height, pixels, tw, th, x, y));
            }
        }
        soften_tile(
            &mut bgr,
            tw,
            th,
            |v| v as i32,
            |sum| ((sum + 2) / 4) as u8,
        );
        Self {
            width: tw,
            height: th,
            is_delta: false,
            is_overlay: false,
            bgr,
            dbgr: Vec::new(),
            overlay: Vec::new(),
            gain_q: u16::MAX,
        }
    }
    pub fn from_overlay_bgra(
        width: usize,
        height: usize,
        pixels: &[u8],
        inset_px: usize,
    ) -> Self {
        if width == 0 || height == 0 || pixels.len() < width * height * 4 {
            return Self {
                width: 0,
                height: 0,
                is_delta: false,
                is_overlay: true,
                bgr: Vec::new(),
                dbgr: Vec::new(),
                overlay: Vec::new(),
                gain_q: u16::MAX,
            };
        }
        let (tw, th) = (width.min(64), height.min(64));
        let ix = inset_px.min(width.saturating_sub(2) / 2);
        let iy = inset_px.min(height.saturating_sub(2) / 2);
        let iw = (width - ix * 2).max(1);
        let ih = (height - iy * 2).max(1);
        let mut overlay = Vec::with_capacity(tw * th);
        for y in 0..th {
            let t = iy + y * ih / th;
            let b = (iy + (y + 1) * ih / th).max(t + 1).min(height);
            let t = t.min(b - 1);
            for x in 0..tw {
                let l = ix + x * iw / tw;
                let r = (ix + (x + 1) * iw / tw).max(l + 1).min(width);
                let l = l.min(r - 1);
                let mut sum = [0u64; 4];
                for sy in t..b {
                    for sx in l..r {
                        let i = (sy * width + sx) * 4;
                        for c in 0..4 {
                            sum[c] += pixels[i + c] as u64;
                        }
                    }
                }
                let n = ((r - l) * (b - t)).max(1) as u64;
                overlay.push(sum.map(|v| ((v + n / 2) / n) as u8));
            }
        }
        soften_tile(
            &mut overlay,
            tw,
            th,
            |v| v as i32,
            |sum| ((sum + 2) / 4) as u8,
        );
        Self {
            width: tw,
            height: th,
            is_delta: false,
            is_overlay: true,
            bgr: Vec::new(),
            dbgr: Vec::new(),
            overlay,
            gain_q: u16::MAX,
        }
    }
    /// Captures only the per-widget chrome delta (`chrome - wallpaper`) so
    /// seams blend tints/overlays without blurring the underlying wallpaper.
    pub fn from_bgra_delta(
        width: usize,
        height: usize,
        chrome_bgra: &[u8],
        wall_tile: &[[u8; 3]],
        tw: usize,
        th: usize,
        rim_mask: Option<&[bool]>,
    ) -> Self {
        if width == 0 || height == 0 || tw == 0 || th == 0 || wall_tile.len() < tw * th {
            return Self {
                width: tw,
                height: th,
                is_delta: true,
                is_overlay: false,
                bgr: Vec::new(),
                dbgr: vec![[0; 3]; tw * th],
                overlay: Vec::new(),
                gain_q: u16::MAX,
            };
        }
        let mut dbgr_unmasked_count = 0usize;
        let mut unmasked_cells = Vec::new();
        let mut gain_cells = Vec::new();
        let border = (tw.min(th) / 6).max(1);
        let mut cell_data = vec![([0u8; 3], [0u8; 3], false, false); tw * th];

        for y in 0..th {
            for x in 0..tw {
                let cell_idx = y * tw + x;
                let masked = rim_mask.is_some_and(|m| {
                    if m.len() != width * height {
                        false
                    } else {
                        let (l, r, t, b) = (
                            x * width / tw,
                            (x + 1) * width / tw,
                            y * height / th,
                            (y + 1) * height / th,
                        );
                        let mut any_masked = false;
                        for sy in t..b {
                            for sx in l..r {
                                if m[sy * width + sx] {
                                    any_masked = true;
                                    break;
                                }
                            }
                            if any_masked {
                                break;
                            }
                        }
                        any_masked
                    }
                });
                let gain_masked = masked
                    || (tw >= 12
                        && th >= 12
                        && (x < border || x + border >= tw || y < border || y + border >= th));

                let chrome = area_filter_cell(width, height, chrome_bgra, tw, th, x, y);
                let wall = wall_tile[cell_idx];
                cell_data[cell_idx] = (chrome, wall, masked, gain_masked);
                if !masked {
                    dbgr_unmasked_count += 1;
                    unmasked_cells.push((chrome, wall));
                }
                if !gain_masked {
                    gain_cells.push((chrome, wall));
                }
            }
        }

        let use_gain_mask = !gain_cells.is_empty();
        let active_cells = if use_gain_mask {
            &gain_cells
        } else {
            &unmasked_cells
        };
        let is_gain_masked = |c: &([u8; 3], [u8; 3], bool, bool)| {
            if use_gain_mask { c.3 } else { c.2 }
        };

        let mut gain = 1.0f32;
        if !active_cells.is_empty() {
            let mut lap_num = 0.0f32;
            let mut lap_den = 0.0f32;
            for y in 0..th {
                for x in 1..tw.saturating_sub(1) {
                    let c0 = cell_data[y * tw + x - 1];
                    let c1 = cell_data[y * tw + x];
                    let c2 = cell_data[y * tw + x + 1];
                    if !is_gain_masked(&c0) && !is_gain_masked(&c1) && !is_gain_masked(&c2) {
                        for ch in 0..3 {
                            let dp = 2.0 * c1.0[ch] as f32 - c0.0[ch] as f32 - c2.0[ch] as f32;
                            let dw = 2.0 * c1.1[ch] as f32 - c0.1[ch] as f32 - c2.1[ch] as f32;
                            lap_num += dp * dw;
                            lap_den += dw * dw;
                        }
                    }
                }
            }
            for y in 1..th.saturating_sub(1) {
                for x in 0..tw {
                    let c0 = cell_data[(y - 1) * tw + x];
                    let c1 = cell_data[y * tw + x];
                    let c2 = cell_data[(y + 1) * tw + x];
                    if !is_gain_masked(&c0) && !is_gain_masked(&c1) && !is_gain_masked(&c2) {
                        for ch in 0..3 {
                            let dp = 2.0 * c1.0[ch] as f32 - c0.0[ch] as f32 - c2.0[ch] as f32;
                            let dw = 2.0 * c1.1[ch] as f32 - c0.1[ch] as f32 - c2.1[ch] as f32;
                            lap_num += dp * dw;
                            lap_den += dw * dw;
                        }
                    }
                }
            }
            let (num, den) = if lap_den >= (active_cells.len() as f32) * 8.0 {
                (lap_num, lap_den)
            } else {
                let mut mean_p = [0.0f32; 3];
                let mut mean_w = [0.0f32; 3];
                for (p, w) in active_cells {
                    for c in 0..3 {
                        mean_p[c] += p[c] as f32;
                        mean_w[c] += w[c] as f32;
                    }
                }
                let n = active_cells.len() as f32;
                for c in 0..3 {
                    mean_p[c] /= n;
                    mean_w[c] /= n;
                }
                let mut g_num = 0.0f32;
                let mut g_den = 0.0f32;
                for (p, w) in active_cells {
                    let dp = [
                        p[0] as f32 - mean_p[0],
                        p[1] as f32 - mean_p[1],
                        p[2] as f32 - mean_p[2],
                    ];
                    let dw = [
                        w[0] as f32 - mean_w[0],
                        w[1] as f32 - mean_w[1],
                        w[2] as f32 - mean_w[2],
                    ];
                    g_num += dp[0] * dw[0] + dp[1] * dw[1] + dp[2] * dw[2];
                    g_den += dw[0] * dw[0] + dw[1] * dw[1] + dw[2] * dw[2];
                }
                (g_num, g_den)
            };
            let prior = (active_cells.len() as f32 * 4.0).max(1.0);
            if den > 1e-3 {
                gain = ((num + prior) / (den + prior)).clamp(0.0, 1.0);
            }
        }

        let all_masked = dbgr_unmasked_count == 0;
        let mut dbgr = Vec::with_capacity(tw * th);
        for y in 0..th {
            for x in 0..tw {
                let (chrome, wall, masked, _) = cell_data[y * tw + x];
                if masked && !all_masked {
                    dbgr.push([0i16; 3]);
                } else {
                    let diff = std::array::from_fn(|c| {
                        (chrome[c] as f32 - (gain * wall[c] as f32).round())
                            .round()
                            .clamp(-512.0, 512.0) as i16
                    });
                    dbgr.push(diff);
                }
            }
        }

        if !all_masked {
            let mut row_has_data = vec![false; th];
            let mut left_near = vec![None; tw];
            for y in 0..th {
                let mut cur = None;
                for x in 0..tw {
                    if !cell_data[y * tw + x].2 {
                        cur = Some(x);
                    }
                    left_near[x] = cur;
                }
                if cur.is_some() {
                    row_has_data[y] = true;
                    let mut right_near = None;
                    for x in (0..tw).rev() {
                        if !cell_data[y * tw + x].2 {
                            right_near = Some(x);
                        } else {
                            let src_x = match (left_near[x], right_near) {
                                (Some(lx), Some(rx)) => {
                                    if x - lx <= rx - x {
                                        lx
                                    } else {
                                        rx
                                    }
                                }
                                (Some(lx), None) => lx,
                                (None, Some(rx)) => rx,
                                (None, None) => unreachable!(),
                            };
                            dbgr[y * tw + x] = dbgr[y * tw + src_x];
                        }
                    }
                }
            }
            for y in 0..th {
                if row_has_data[y] {
                    continue;
                }
                if let Some(y_near) = (0..th)
                    .filter(|&ry| row_has_data[ry])
                    .min_by_key(|&ry| y.abs_diff(ry))
                {
                    for x in 0..tw {
                        dbgr[y * tw + x] = dbgr[y_near * tw + x];
                    }
                }
            }
        }

        soften_tile(
            &mut dbgr,
            tw,
            th,
            |v| v as i32,
            |sum| (sum as f32 / 4.0).round() as i16,
        );

        let gain_q = (gain * 65535.0).round().clamp(0.0, 65535.0) as u16;
        Self {
            width: tw,
            height: th,
            is_delta: true,
            is_overlay: false,
            bgr: Vec::new(),
            dbgr,
            overlay: Vec::new(),
            gain_q,
        }
    }
    pub fn sample(&self, window: [i32; 4], x: f32, y: f32) -> [f32; 3] {
        sample_grid(&self.bgr, self.width, self.height, window, x, y, |v| {
            v as f32
        })
    }
    pub fn sample_overlay(&self, window: [i32; 4], x: f32, y: f32) -> [f32; 4] {
        if self.is_overlay && !self.overlay.is_empty() {
            sample_grid(&self.overlay, self.width, self.height, window, x, y, |v| {
                v as f32
            })
        } else if self.is_opaque() && !self.bgr.is_empty() {
            let c = self.sample(window, x, y);
            [c[0], c[1], c[2], 255.0]
        } else {
            [0.0; 4]
        }
    }
    pub fn sample_delta(&self, window: [i32; 4], x: f32, y: f32) -> [f32; 3] {
        if !self.is_delta {
            return [0.0; 3];
        }
        sample_grid(&self.dbgr, self.width, self.height, window, x, y, |v| {
            v as f32
        })
    }
    pub fn avg_rgb_delta(&self) -> [f32; 3] {
        if !self.is_delta || self.dbgr.is_empty() {
            return [0.0; 3];
        }
        let mut sum = [0i64; 3];
        for p in &self.dbgr {
            sum[0] += p[2] as i64;
            sum[1] += p[1] as i64;
            sum[2] += p[0] as i64;
        }
        let denom = self.dbgr.len() as f32 * 255.0;
        [
            (sum[0] as f32 / denom).clamp(-1.0, 1.0),
            (sum[1] as f32 / denom).clamp(-1.0, 1.0),
            (sum[2] as f32 / denom).clamp(-1.0, 1.0),
        ]
    }
}
fn seam_distance(pane: &Pane, x: f32, y: f32) -> f32 {
    let [l, t, r, b] = pane.rect;
    let rad = (4.0 * pane.dpi).min((r - l).min(b - t) * 0.5);
    crate::compositor::distance(x - l, y - t, r - l, b - t, rad, 0.)
}
pub fn blend<'a>(
    sources: impl Iterator<Item = (&'a Pane, [i32; 4], &'a Material)>,
    x: f32,
    y: f32,
) -> Option<[f32; 3]> {
    let mut sum = 0.;
    let mut rgb = [0.; 3];
    for (pane, window, material) in sources {
        if material.is_delta || material.bgr.is_empty() {
            continue;
        }
        // Signed distance is negative inside. Outside, extend the edge color
        // smoothly into the neighboring pane; both sides evaluate one field.
        let weight = (-seam_distance(pane, x, y).max(0.) / (96. * pane.dpi)).exp();
        let sample = material.sample(window, x, y);
        for c in 0..3 {
            rgb[c] += sample[c] * weight;
        }
        sum += weight;
    }
    (sum > 0.).then(|| rgb.map(|v| v / sum))
}
pub fn blend_affine<'a>(
    sources: impl Iterator<Item = (&'a Pane, [i32; 4], &'a Material)>,
    x: f32,
    y: f32,
) -> Option<(f32, [f32; 3])> {
    let mut sum = 0.;
    let mut gain_sum = 0.;
    let mut rgb = [0.; 3];
    for (pane, window, material) in sources {
        if !material.is_delta || material.dbgr.is_empty() {
            continue;
        }
        let weight = (-seam_distance(pane, x, y).max(0.) / (96. * pane.dpi)).exp();
        let sample = material.sample_delta(window, x, y);
        let g = material.gain();
        for c in 0..3 {
            rgb[c] += sample[c] * weight;
        }
        gain_sum += g * weight;
        sum += weight;
    }
    (sum > 0.).then(|| {
        (
            gain_sum / sum,
            rgb.map(|v| v / sum),
        )
    })
}
pub fn blend_delta<'a>(
    sources: impl Iterator<Item = (&'a Pane, [i32; 4], &'a Material)>,
    x: f32,
    y: f32,
) -> Option<[f32; 3]> {
    blend_affine(sources, x, y).map(|(_, bias)| bias)
}
pub fn blend_overlay<'a>(
    sources: impl Iterator<Item = (&'a Pane, [i32; 4], &'a Material)>,
    x: f32,
    y: f32,
) -> Option<[f32; 4]> {
    let mut sum = 0.;
    let mut bgra = [0.; 4];
    for (pane, window, material) in sources {
        if (!material.is_overlay && !material.is_opaque())
            || (material.is_overlay && material.overlay.is_empty())
            || (material.is_opaque() && material.bgr.is_empty())
        {
            continue;
        }
        let weight = (-seam_distance(pane, x, y).max(0.) / (96. * pane.dpi)).exp();
        let sample = material.sample_overlay(window, x, y);
        for c in 0..4 {
            bgra[c] += sample[c] * weight;
        }
        sum += weight;
    }
    (sum > 0.).then(|| bgra.map(|v| v / sum))
}
pub fn blend_scalar<'a>(
    sources: impl Iterator<Item = (&'a Pane, f32)>,
    x: f32,
    y: f32,
) -> Option<f32> {
    let mut sum = 0.;
    let mut acc = 0.;
    for (pane, value) in sources {
        let weight = (-seam_distance(pane, x, y).max(0.) / (96. * pane.dpi)).exp();
        acc += value * weight;
        sum += weight;
    }
    (sum > 0.).then_some(acc / sum)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn downsampled_grain_does_not_turn_into_bright_reflection_cells() {
        let mut source = vec![0u8; 256 * 256 * 4];
        for y in 0..256 {
            for x in 0..256 {
                let v = if (x + y) % 2 == 0 { 52 } else { 28 };
                source[(y * 256 + x) * 4..(y * 256 + x) * 4 + 4].copy_from_slice(&[v, v, v, 255]);
            }
        }
        let tile = Material::from_bgra(256, 256, &source);
        assert!(
            tile.bgr.iter().all(|p| *p == [40; 3]),
            "area filtering must reject phase-locked grain aliasing"
        );
    }
    #[test]
    fn localized_reflection_softens_without_ringing_or_extra_color_quantization() {
        let mut source = vec![40u8; 64 * 64 * 4];
        source[(16 * 64 + 16) * 4..(16 * 64 + 16) * 4 + 3].fill(240);
        let tile = Material::from_bgra(64, 64, &source);
        let center = tile.bgr[16 * 64 + 16][0];
        assert!(center > 40 && center < 100);
        assert!(tile.bgr[16 * 64 + 17][0] < center);
        assert!(tile.bgr[16 * 64 + 18][0] <= tile.bgr[16 * 64 + 17][0]);
        assert!(tile.bgr.iter().flatten().all(|v| *v >= 40 && *v <= center));
        let field = Field {
            window: [0, 0, 32, 16],
            width: 2,
            height: 1,
            is_delta: false,
            bgr: vec![[100.1; 3], [100.3; 3]],
        };
        assert!(
            (field.sample(16., 8.)[0] - 100.2).abs() < 0.001,
            "field retains sub-byte lighting precision"
        );
    }
    #[test]
    fn joining_saturated_panels_mix_colors_without_losing_opacity_or_using_wallpaper() {
        let a = Pane {
            id: 1,
            rect: [0., 0., 200., 200.],
            radius: 34.,
            dpi: 1.,
            topmost: false,
        };
        let b = Pane {
            id: 2,
            rect: [0., 200., 200., 400.],
            ..a
        };
        let red = Material {
            width: 1,
            height: 1,
            is_delta: false,
            is_overlay: false,
            bgr: vec![[0, 0, 255]],
            dbgr: Vec::new(),
            overlay: Vec::new(),
            gain_q: u16::MAX,
        };
        let blue = Material {
            width: 1,
            height: 1,
            is_delta: false,
            is_overlay: false,
            bgr: vec![[255, 0, 0]],
            dbgr: Vec::new(),
            overlay: Vec::new(),
            gain_q: u16::MAX,
        };
        let field = |y| {
            blend(
                [
                    (&a, [0, 0, 200, 200], &red),
                    (&b, [0, 200, 200, 400], &blue),
                ]
                .into_iter(),
                100.,
                y,
            )
            .unwrap()
        };
        let seam = field(200.);
        assert!((seam[0] - 127.5).abs() < 0.01 && seam[1] == 0. && (seam[2] - 127.5).abs() < 0.01);
        assert!(field(140.)[2] > field(140.)[0]);
        assert!(field(260.)[0] > field(260.)[2]);
        let before = field(199.99);
        let after = field(200.01);
        assert!(before
            .into_iter()
            .zip(after)
            .all(|(a, b)| (a - b).abs() < 0.1));
    }
    #[test]
    fn source_gradient_is_bilinear_and_extends_its_edge_instead_of_fading_out() {
        let m = Material {
            width: 2,
            height: 1,
            is_delta: false,
            is_overlay: false,
            bgr: vec![[20, 40, 60], [100, 120, 140]],
            dbgr: Vec::new(),
            overlay: Vec::new(),
            gain_q: u16::MAX,
        };
        assert_eq!(m.sample([0, 0, 100, 100], 50., 50.), [60., 80., 100.]);
        assert_eq!(m.sample([0, 0, 100, 100], 200., 50.), [100., 120., 140.]);
    }
    #[test]
    fn affine_material_recovers_gain_and_bias_without_wallpaper_stripe_leakage() {
        let tw = 16;
        let th = 16;
        let mut wall_tile = vec![[0u8; 3]; tw * th];
        for y in 0..th {
            for x in 0..tw {
                let v = if (x / 2) % 2 == 0 { 200u8 } else { 50u8 };
                wall_tile[y * tw + x] = [v, v, v];
            }
        }
        let width = 64;
        let height = 64;
        let mut chrome_bgra = vec![0u8; width * height * 4];
        let mut rim_mask = vec![false; width * height];
        for y in 0..height {
            for x in 0..width {
                let idx = (y * width + x) * 4;
                let wx = (x * tw / width).clamp(0, tw - 1);
                let wy = (y * th / height).clamp(0, th - 1);
                let wv = wall_tile[wy * tw + wx][0] as f32;
                // chrome = 0.74 * wall + 0.26 * afterglow (e.g. 100)
                let cval = (0.74 * wv + 0.26 * 100.0).round().clamp(0.0, 255.0) as u8;
                chrome_bgra[idx] = cval;
                chrome_bgra[idx + 1] = cval;
                chrome_bgra[idx + 2] = cval;
                chrome_bgra[idx + 3] = 255;
                // mask out fake bright white rim border on outer edge
                if x < 4 || x >= width - 4 || y < 4 || y >= height - 4 {
                    rim_mask[y * width + x] = true;
                    chrome_bgra[idx] = 255;
                    chrome_bgra[idx + 1] = 255;
                    chrome_bgra[idx + 2] = 255;
                }
            }
        }
        let mat = Material::from_bgra_delta(width, height, &chrome_bgra, &wall_tile, tw, th, Some(&rim_mask));
        let g = mat.gain();
        assert!((g - 0.74).abs() < 0.05, "affine gain recovered should be close to 0.74, got {}", g);
    }
}
