//! 截图标注：把前端传来的矢量操作画到抓取到的像素上。
//!
//! 前端只负责交互与预览（canvas），最终合成为准的是这里的实现——
//! 这样「所见即所得」不会因为前端导出画布时丢精度/透明通道而出问题。

use serde::Deserialize;

use crate::capture::Frame;

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Op {
    /// 矩形框选（空心）
    Rect { x: f32, y: f32, w: f32, h: f32, #[serde(default)] color: Option<String>, #[serde(default)] width: Option<f32> },
    /// 箭头
    Arrow { x1: f32, y1: f32, x2: f32, y2: f32, #[serde(default)] color: Option<String>, #[serde(default)] width: Option<f32> },
    /// 马赛克（区域均值块化）
    Mosaic { x: f32, y: f32, w: f32, h: f32, #[serde(default)] block: Option<u32> },
    /// 自由画笔（折线）
    Pen { points: Vec<[f32; 2]>, #[serde(default)] color: Option<String>, #[serde(default)] width: Option<f32> },
}

type Rgba = [u8; 3];

fn parse_color(raw: &Option<String>) -> Rgba {
    let Some(text) = raw.as_deref() else {
        return [235, 64, 52];
    };
    let hex = text.trim_start_matches('#');
    if hex.len() != 6 {
        return [235, 64, 52];
    }
    let value = u32::from_str_radix(hex, 16).unwrap_or(0xeb4034);
    [
        ((value >> 16) & 0xff) as u8,
        ((value >> 8) & 0xff) as u8,
        (value & 0xff) as u8,
    ]
}

/// 按顺序把标注画到帧上。
pub fn apply(frame: &mut Frame, ops: &[Op]) {
    for op in ops {
        match op {
            Op::Rect {
                x,
                y,
                w,
                h,
                color,
                width,
            } => {
                let rgba = parse_color(color);
                let thickness = width.unwrap_or(2.0).max(1.0);
                draw_rect(frame, *x, *y, *w, *h, thickness, rgba);
            }
            Op::Arrow {
                x1,
                y1,
                x2,
                y2,
                color,
                width,
            } => {
                let rgba = parse_color(color);
                let thickness = width.unwrap_or(3.0).max(1.0);
                draw_arrow(frame, *x1, *y1, *x2, *y2, thickness, rgba);
            }
            Op::Mosaic { x, y, w, h, block } => {
                mosaic(frame, *x, *y, *w, *h, block.unwrap_or(10).max(2));
            }
            Op::Pen {
                points,
                color,
                width,
            } => {
                let rgba = parse_color(color);
                let thickness = width.unwrap_or(2.0).max(1.0);
                for pair in points.windows(2) {
                    draw_line(frame, pair[0][0], pair[0][1], pair[1][0], pair[1][1], thickness, rgba);
                }
            }
        }
    }
}

fn put(frame: &mut Frame, x: i32, y: i32, rgba: Rgba) {
    if x < 0 || y < 0 || x >= frame.width as i32 || y >= frame.height as i32 {
        return;
    }
    let offset = ((y as u32 * frame.width + x as u32) * 4) as usize;
    if offset + 3 >= frame.bgra.len() {
        return;
    }
    // BGRA 存储：只改颜色，保留原 alpha
    frame.bgra[offset] = rgba[2];
    frame.bgra[offset + 1] = rgba[1];
    frame.bgra[offset + 2] = rgba[0];
}

fn stamp(frame: &mut Frame, cx: f32, cy: f32, radius: f32, rgba: Rgba) {
    let r = radius.max(0.5);
    let left = (cx - r).floor() as i32;
    let right = (cx + r).ceil() as i32;
    let top = (cy - r).floor() as i32;
    let bottom = (cy + r).ceil() as i32;

    for y in top..=bottom {
        for x in left..=right {
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            if dx * dx + dy * dy <= r * r {
                put(frame, x, y, rgba);
            }
        }
    }
}

fn draw_line(frame: &mut Frame, x1: f32, y1: f32, x2: f32, y2: f32, thickness: f32, rgba: Rgba) {
    let dx = x2 - x1;
    let dy = y2 - y1;
    let steps = dx.abs().max(dy.abs()).ceil().max(1.0) as i32;
    let radius = thickness / 2.0;

    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        stamp(frame, x1 + dx * t, y1 + dy * t, radius, rgba);
    }
}

fn draw_rect(frame: &mut Frame, x: f32, y: f32, w: f32, h: f32, thickness: f32, rgba: Rgba) {
    let x2 = x + w;
    let y2 = y + h;
    draw_line(frame, x, y, x2, y, thickness, rgba);
    draw_line(frame, x2, y, x2, y2, thickness, rgba);
    draw_line(frame, x2, y2, x, y2, thickness, rgba);
    draw_line(frame, x, y2, x, y, thickness, rgba);
}

fn draw_arrow(frame: &mut Frame, x1: f32, y1: f32, x2: f32, y2: f32, thickness: f32, rgba: Rgba) {
    draw_line(frame, x1, y1, x2, y2, thickness, rgba);

    let angle = (y2 - y1).atan2(x2 - x1);
    let head = (thickness * 5.0).max(10.0);
    let spread = 0.45f32;

    for offset in [-spread, spread] {
        let hx = x2 - head * (angle + offset).cos();
        let hy = y2 - head * (angle + offset).sin();
        draw_line(frame, x2, y2, hx, hy, thickness, rgba);
    }
}

fn mosaic(frame: &mut Frame, x: f32, y: f32, w: f32, h: f32, block: u32) {
    let left = x.max(0.0) as u32;
    let top = y.max(0.0) as u32;
    let right = ((x + w).max(0.0) as u32).min(frame.width);
    let bottom = ((y + h).max(0.0) as u32).min(frame.height);
    if right <= left || bottom <= top {
        return;
    }

    let mut by = top;
    while by < bottom {
        let mut bx = left;
        while bx < right {
            let ex = (bx + block).min(right);
            let ey = (by + block).min(bottom);
            let mut sum = [0u64; 3];
            let mut count = 0u64;

            for py in by..ey {
                for px in bx..ex {
                    let offset = ((py * frame.width + px) * 4) as usize;
                    if offset + 3 >= frame.bgra.len() {
                        continue;
                    }
                    sum[0] += frame.bgra[offset + 2] as u64;
                    sum[1] += frame.bgra[offset + 1] as u64;
                    sum[2] += frame.bgra[offset] as u64;
                    count += 1;
                }
            }
            if count > 0 {
                let avg = [
                    (sum[0] / count) as u8,
                    (sum[1] / count) as u8,
                    (sum[2] / count) as u8,
                ];
                for py in by..ey {
                    for px in bx..ex {
                        put(frame, px as i32, py as i32, avg);
                    }
                }
            }
            bx += block;
        }
        by += block;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blank(width: u32, height: u32) -> Frame {
        Frame {
            x: 0,
            y: 0,
            width,
            height,
            bgra: vec![0u8; (width * height * 4) as usize],
        }
    }

    fn pixel(frame: &Frame, x: u32, y: u32) -> [u8; 3] {
        let offset = ((y * frame.width + x) * 4) as usize;
        [
            frame.bgra[offset + 2],
            frame.bgra[offset + 1],
            frame.bgra[offset],
        ]
    }

    #[test]
    fn rect_draws_border_only() {
        let mut frame = blank(40, 40);
        apply(
            &mut frame,
            &[Op::Rect {
                x: 10.0,
                y: 10.0,
                w: 20.0,
                h: 20.0,
                color: Some("#ff0000".into()),
                width: Some(2.0),
            }],
        );
        assert_eq!(pixel(&frame, 10, 10), [255, 0, 0]);
        assert_eq!(pixel(&frame, 30, 30), [255, 0, 0]);
        // 中心不应该被涂
        assert_eq!(pixel(&frame, 20, 20), [0, 0, 0]);
    }

    #[test]
    fn mosaic_averages_region() {
        let mut frame = blank(20, 20);
        // 左半白、右半黑
        for y in 0..20 {
            for x in 0..20 {
                let offset = ((y * 20 + x) * 4) as usize;
                let value = if x < 10 { 255 } else { 0 };
                frame.bgra[offset] = value;
                frame.bgra[offset + 1] = value;
                frame.bgra[offset + 2] = value;
            }
        }
        apply(
            &mut frame,
            &[Op::Mosaic {
                x: 0.0,
                y: 0.0,
                w: 20.0,
                h: 20.0,
                block: Some(20),
            }],
        );
        // 整块平均后应接近 127
        let p = pixel(&frame, 5, 5)[0];
        assert!((120..=135).contains(&p), "平均值异常：{p}");
    }

    #[test]
    fn arrow_marks_endpoints() {
        let mut frame = blank(60, 60);
        apply(
            &mut frame,
            &[Op::Arrow {
                x1: 5.0,
                y1: 5.0,
                x2: 50.0,
                y2: 50.0,
                color: Some("#00ff00".into()),
                width: Some(3.0),
            }],
        );
        assert_eq!(pixel(&frame, 5, 5), [0, 255, 0]);
        assert_eq!(pixel(&frame, 50, 50), [0, 255, 0]);
    }

    #[test]
    fn out_of_bounds_is_safe() {
        let mut frame = blank(10, 10);
        apply(
            &mut frame,
            &[Op::Rect {
                x: -50.0,
                y: -50.0,
                w: 500.0,
                h: 500.0,
                color: None,
                width: None,
            }],
        );
    }
}
