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
    SIZES
        .iter()
        .map(|&size| render(size, snap.status, &label))
        .collect()
}

/// Один пиксмап заданного размера.
pub fn render(size: i32, status: Status, label: &str) -> Icon {
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

    draw_number(&mut buf, size, label);
    Icon {
        width: size,
        height: size,
        data: buf,
    }
}

/// SDF скруглённого квадрата: отрицательно внутри, положительно снаружи.
fn rounded_rect_sd(x: f32, y: f32, half: f32, radius: f32) -> f32 {
    let qx = x.abs() - half + radius;
    let qy = y.abs() - half + radius;
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
    text.chars().map(|c| font.metrics(c, px).advance_width).sum()
}

/// Вписываем текст в квадрат и центрируем по обеим осям.
fn draw_number(buf: &mut [u8], size: i32, text: &str) {
    let font = font();
    // На маленьких размерах оставляем чуть больше воздуха по краям.
    let margin = if size <= 24 { 3.0 } else { 2.0 };
    let max_w = size as f32 - margin * 2.0;
    let mut px = size as f32 * 0.60;
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
    let baseline = (size as f32 - content_h) / 2.0 + top;
    let mut pen = (size as f32 - measure(font, text, px)) / 2.0;

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
                    let bg = if (x / 4 + y / 4) % 2 == 0 { 210.0 } else { 160.0 };
                    *slot = (src * a + bg * (1.0 - a)).round() as u8;
                }
                out.extend_from_slice(&rgb);
            }
        }
        fs::write(dir.join(format!("icon-{size}x{size}.ppm")), out)?;
    }
    Ok(())
}