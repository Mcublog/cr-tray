//! Отрисовка иконки трея: скруглённый квадрат + цифра суммы за сегодня.
//!
//! ksni ожидает `Icon.data` в формате ARGB32 (network byte order), то есть
//! байты идут в порядке `A, R, G, B`. Работаем сразу в этом порядке.

use std::sync::OnceLock;

use fontdue::{Font, FontSettings};
use ksni::Icon;

use crate::format;
use crate::state::{Snapshot, Status};

static FONT_BYTES: &[u8] = include_bytes!("../assets/DejaVuSans-Bold.ttf");

/// Размеры пиксмапов, которые отдаём в трей (обычный экран и HiDPI).
const SIZES: [i32; 3] = [22, 32, 48];

fn font() -> &'static Font {
    static FONT: OnceLock<Font> = OnceLock::new();
    FONT.get_or_init(|| {
        Font::from_bytes(FONT_BYTES, FontSettings::default()).expect("embedded font is valid")
    })
}

fn background(status: Status) -> [u8; 3] {
    match status {
        Status::Ok => [0x1F, 0x8A, 0x4C],
        Status::Warn => [0xC8, 0x86, 0x0D],
        Status::Error => [0xB3, 0x26, 0x1E],
    }
}

/// Все размеры иконки для текущего снимка.
pub fn icons(snap: &Snapshot) -> Vec<Icon> {
    let label = format::icon_label(snap.spent_rub);
    let unread = snap.has_unread();
    SIZES
        .iter()
        .map(|&size| render(size, snap.status, &label, unread))
        .collect()
}

/// Один пиксмап заданного размера.
pub fn render(size: i32, status: Status, label: &str, unread: bool) -> Icon {
    let size = size.max(8);
    let n = (size * size) as usize;
    let mut buf = vec![0u8; n * 4];

    let bg = background(status);
    let half = size as f32 / 2.0 - 0.75;
    let radius = (size as f32 * 0.24).min(half);

    for y in 0..size {
        for x in 0..size {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let sd = rounded_rect_sd(px - size as f32 / 2.0, py - size as f32 / 2.0, half, radius);
            let cov = (0.5 - sd).clamp(0.0, 1.0);
            if cov > 0.0 {
                blend(&mut buf, x, y, size, bg, cov);
            }
            // У ошибки — белое кольцо внутри квадрата, чтобы состояние
            // читалось не только цветом.
            if status == Status::Error {
                let ring = (1.25 - (sd + 2.5).abs()).clamp(0.0, 1.0);
                if ring > 0.0 {
                    blend(&mut buf, x, y, size, [0xFF, 0xFF, 0xFF], ring);
                }
            }
        }
    }

    draw_number(&mut buf, size, label, unread);
    if unread {
        draw_envelope(&mut buf, size);
    }
    Icon {
        width: size,
        height: size,
        data: buf,
    }
}

/// SDF скруглённого квадрата: отрицательно внутри, положительно снаружи.
fn rounded_rect_sd(x: f32, y: f32, half: f32, radius: f32) -> f32 {
    rounded_box_sd(x, y, half, half, radius)
}

/// SDF скруглённого прямоугольника: отрицательно внутри, положительно снаружи.
fn rounded_box_sd(x: f32, y: f32, half_w: f32, half_h: f32, radius: f32) -> f32 {
    let radius = radius.min(half_w).min(half_h);
    let qx = x.abs() - half_w + radius;
    let qy = y.abs() - half_h + radius;
    let ox = qx.max(0.0);
    let oy = qy.max(0.0);
    (ox * ox + oy * oy).sqrt() + qx.max(qy).min(0.0) - radius
}

/// Наложение цвета `rgb` с прозрачностью `a` поверх пикселя (source-over).
fn blend(buf: &mut [u8], x: i32, y: i32, size: i32, rgb: [u8; 3], a: f32) {
    if x < 0 || y < 0 || x >= size || y >= size {
        return;
    }
    let idx = ((y * size + x) * 4) as usize;
    let sa = a.clamp(0.0, 1.0);
    let da = buf[idx] as f32 / 255.0;
    let oa = sa + da * (1.0 - sa);
    if oa <= f32::EPSILON {
        buf[idx..idx + 4].copy_from_slice(&[0, 0, 0, 0]);
        return;
    }
    for (i, channel) in rgb.iter().enumerate() {
        let dst = buf[idx + 1 + i] as f32;
        let out = (*channel as f32 * sa + dst * da * (1.0 - sa)) / oa;
        buf[idx + 1 + i] = out.round().clamp(0.0, 255.0) as u8;
    }
    buf[idx] = (oa * 255.0).round().clamp(0.0, 255.0) as u8;
}

fn measure(font: &Font, text: &str, px: f32) -> f32 {
    text.chars()
        .map(|c| font.metrics(c, px).advance_width)
        .sum()
}

/// Вписываем текст в квадрат и центрируем по обеим осям.
fn draw_number(buf: &mut [u8], size: i32, text: &str, badge: bool) {
    let font = font();
    let (center_x, center_y, mut px, max_w) = if badge {
        // Конверт стоит по центру сверху: сумма сдвигается вниз и чуть
        // уменьшается, чтобы подписи не пересекались даже на 22 px.
        (
            size as f32 * 0.5,
            size as f32 * 0.62,
            size as f32 * 0.53,
            size as f32 * 0.84,
        )
    } else {
        let margin = if size <= 24 { 3.0 } else { 2.0 };
        (
            size as f32 * 0.5,
            size as f32 * 0.5,
            size as f32 * 0.60,
            size as f32 - margin * 2.0,
        )
    };
    let width = measure(font, text, px);
    if width > max_w {
        px *= max_w / width;
    }

    let mut top = f32::MIN;
    let mut bottom = f32::MAX;
    for ch in text.chars() {
        let m = font.metrics(ch, px);
        top = top.max(m.ymin as f32 + m.height as f32);
        bottom = bottom.min(m.ymin as f32);
    }
    if !top.is_finite() || !bottom.is_finite() {
        return;
    }

    let content_h = top - bottom;
    let baseline = center_y - content_h / 2.0 + top;
    let mut pen = center_x - measure(font, text, px) / 2.0;

    for ch in text.chars() {
        let (m, bitmap) = font.rasterize(ch, px);
        let x0 = pen + m.xmin as f32;
        let y0 = baseline - m.ymin as f32 - m.height as f32;
        for row in 0..m.height {
            for col in 0..m.width {
                let cov = bitmap[row * m.width + col] as f32 / 255.0;
                if cov <= 0.0 {
                    continue;
                }
                let x = x0.round() as i32 + col as i32;
                let y = y0.round() as i32 + row as i32;
                blend(buf, x, y, size, [0xFF, 0xFF, 0xFF], cov);
            }
        }
        pen += m.advance_width;
    }
}

/// Маленький белый конверт по центру сверху: есть непрочитанные.
fn draw_envelope(buf: &mut [u8], size: i32) {
    let s = size as f32;
    let margin = (s * 0.06).max(1.0);
    let half_w = (s * 0.17).max(2.0);
    let half_h = (s * 0.13).max(1.8);
    let cx = s / 2.0;
    let cy = margin + half_h;
    let stroke = (s * 0.045).clamp(0.9, 1.6);
    let radius = (half_h * 0.22).max(0.4);

    for y in 0..size {
        for x in 0..size {
            let px = x as f32 + 0.5 - cx;
            let py = y as f32 + 0.5 - cy;
            let sd = rounded_box_sd(px, py, half_w, half_h, radius);
            let outline = (stroke * 0.5 + 0.5 - sd.abs()).clamp(0.0, 1.0);
            if outline > 0.0 {
                blend(buf, x, y, size, [0xFF, 0xFF, 0xFF], outline);
            }
        }
    }

    // Клапан: две линии от верхних углов к середине.
    let inset = stroke * 0.8;
    let left = (cx - half_w + inset, cy - half_h + inset);
    let right = (cx + half_w - inset, cy - half_h + inset);
    let tip = (cx, cy + half_h * 0.30);
    draw_line(buf, size, left, tip, stroke);
    draw_line(buf, size, right, tip, stroke);
}

/// Анти-алиасинговая линия заданной толщины.
fn draw_line(buf: &mut [u8], size: i32, from: (f32, f32), to: (f32, f32), thickness: f32) {
    let pad = thickness * 0.5 + 1.0;
    let min_x = ((from.0.min(to.0) - pad).floor() as i32).max(0);
    let max_x = ((from.0.max(to.0) + pad).ceil() as i32).min(size - 1);
    let min_y = ((from.1.min(to.1) - pad).floor() as i32).max(0);
    let max_y = ((from.1.max(to.1) + pad).ceil() as i32).min(size - 1);

    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let point = (x as f32 + 0.5, y as f32 + 0.5);
            let distance = distance_to_segment(point, from, to);
            let coverage = (thickness * 0.5 + 0.5 - distance).clamp(0.0, 1.0);
            if coverage > 0.0 {
                blend(buf, x, y, size, [0xFF, 0xFF, 0xFF], coverage);
            }
        }
    }
}

fn distance_to_segment(point: (f32, f32), from: (f32, f32), to: (f32, f32)) -> f32 {
    let dx = to.0 - from.0;
    let dy = to.1 - from.1;
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 {
        (((point.0 - from.0) * dx + (point.1 - from.1) * dy) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let x = from.0 + t * dx - point.0;
    let y = from.1 + t * dy - point.1;
    (x * x + y * y).sqrt()
}

/// Отладочный дамп иконок: PPM на шахматном фоне, чтобы видеть прозрачность.
pub fn dump_ppm(snap: &Snapshot, dir: &std::path::Path) -> std::io::Result<()> {
    use std::fs;
    fs::create_dir_all(dir)?;
    for icon in icons(snap) {
        let size = icon.width;
        let mut out = format!("P6\n{size} {size}\n255\n").into_bytes();
        for y in 0..size {
            for x in 0..size {
                let idx = ((y * size + x) * 4) as usize;
                let a = icon.data[idx] as f32 / 255.0;
                let mut rgb = [0u8; 3];
                for (c, slot) in rgb.iter_mut().enumerate() {
                    let src = icon.data[idx + 1 + c] as f32;
                    let bg = if (x / 4 + y / 4) % 2 == 0 {
                        210.0
                    } else {
                        160.0
                    };
                    *slot = (src * a + bg * (1.0 - a)).round() as u8;
                }
                out.extend_from_slice(&rgb);
            }
        }
        fs::write(dir.join(format!("icon-{size}x{size}.ppm")), out)?;
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;

    fn bright_pixels(icon: &Icon, x0: i32, y0: i32, x1: i32, y1: i32) -> usize {
        let mut count = 0;
        for y in y0..y1 {
            for x in x0..x1 {
                let idx = ((y * icon.width + x) * 4) as usize;
                let [a, r, g, b] = [
                    icon.data[idx],
                    icon.data[idx + 1],
                    icon.data[idx + 2],
                    icon.data[idx + 3],
                ];
                if a > 128 && r > 200 && g > 200 && b > 200 {
                    count += 1;
                }
            }
        }
        count
    }

    #[test]
    fn envelope_appears_only_with_unread() {
        // Без подписи, чтобы в углу были только пиксели конверта.
        let plain = render(32, Status::Ok, "", false);
        let unread = render(32, Status::Ok, "", true);
        assert_ne!(plain.data, unread.data);
        assert_eq!(bright_pixels(&plain, 8, 0, 24, 12), 0);
        assert!(bright_pixels(&unread, 8, 0, 24, 12) > 0);
    }

    #[test]
    fn every_size_has_a_badge() {
        for size in SIZES {
            let icon = render(size, Status::Warn, "23", true);
            let left = (size as f32 * 0.30) as i32;
            let right = (size as f32 * 0.70) as i32;
            let bottom = (size as f32 * 0.40) as i32;
            assert!(
                bright_pixels(&icon, left, 0, right, bottom) > 0,
                "нет конверта на {size}px"
            );
        }
    }
}
