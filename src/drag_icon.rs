//! ファイルを他のアプリへドラッグするときにポインタの横に出すアイコン（サムネイル＋ファイル名）を描く。
//! サムネイルの左上には、ドロップでコピーするのか移動するのかを示すバッジを付ける。

use ab_glyph::{Font, FontArc, PxScale, ScaleFont};
use image::{imageops::FilterType, Rgba, RgbaImage};

/// デコード時に作っておくサムネイルの長辺 (ピクセル)。高 DPI でも足りるよう表示サイズの 2 倍にする
pub const THUMBNAIL_SOURCE_SIZE: u32 = 256;

const THUMBNAIL_SIZE: f32 = 128.0;   // アイコン内のサムネイルの長辺（論理ピクセル）
const LABEL_FONT_SIZE: f32 = 13.0;   // ファイル名の文字の大きさ（論理ピクセル）
const LABEL_MIN_WIDTH: f32 = 160.0;  // サムネイルが細くても、ファイル名はこの幅までは並べる
const LABEL_PADDING: f32 = 3.0;      // ファイル名の帯の内側の余白
const GAP: f32 = 4.0;                // サムネイルとファイル名の帯の間隔
const THUMBNAIL_OPACITY: f32 = 0.9;  // サムネイルを少し透かして、下のドロップ先が見えるようにする

/// ドロップしたときにファイルをどうするか
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DragAction {
    Copy, // 既定
    Move, // Ctrl を押しているとき
}

impl DragAction {
    fn badge(self) -> (&'static str, Rgba<u8>) {
        match self {
            DragAction::Copy => ("＋コピー", Rgba([46, 160, 67, 240])),
            DragAction::Move => ("→移動", Rgba([230, 120, 20, 240])),
        }
    }
}

/// サムネイル `thumb` とファイル名 `name` からドラッグアイコンを描く（ストレートアルファの RGBA）。
/// `scale` は画面の拡大率。`font` がなければファイル名とバッジの文字は描かない（バッジは色だけ）
pub fn render(thumb: &RgbaImage, name: &str, font: Option<&FontArc>, scale: f32, action: DragAction) -> RgbaImage {
    let s = scale.max(1.0);
    let px = |v: f32| (v * s).round().max(1.0) as u32;

    // サムネイルを長辺 THUMBNAIL_SIZE に収める（拡大はしない）
    let max_side = px(THUMBNAIL_SIZE);
    let (tw, th) = thumb.dimensions();
    let thumb = if tw.max(th) > max_side && tw > 0 && th > 0 {
        let r = max_side as f32 / tw.max(th) as f32;
        let (w, h) = (((tw as f32 * r).round() as u32).max(1), ((th as f32 * r).round() as u32).max(1));
        image::imageops::resize(thumb, w, h, FilterType::Triangle)
    } else {
        thumb.clone()
    };
    let (tw, th) = thumb.dimensions();
    let border = px(1.0);
    let box_w = tw + border * 2;
    let box_h = th + border * 2;

    // ファイル名（幅に収まらなければ末尾を … で省略）
    let label = font.and_then(|font| {
        let font = font.as_scaled(PxScale::from(LABEL_FONT_SIZE * s));
        let pad = px(LABEL_PADDING);
        let max_text_w = (box_w.max(px(LABEL_MIN_WIDTH)) - pad * 2) as f32;
        let text = ellipsize(&font, name, max_text_w);
        if text.is_empty() {
            return None;
        }
        let text_w = text_width(&font, &text).ceil() as u32;
        let text_h = (font.ascent() - font.descent()).ceil() as u32;
        Some((text, text_w + pad * 2, text_h + pad * 2, pad))
    });

    let (label_w, label_h) = label.as_ref().map_or((0, 0), |(_, w, h, _)| (*w, *h));
    let gap = if label.is_some() { px(GAP) } else { 0 };
    let width = box_w.max(label_w);
    let height = box_h + gap + label_h;
    let mut canvas = RgbaImage::new(width, height);

    // サムネイル（白い枠付き）
    let bx = (width - box_w) / 2;
    fill_rect(&mut canvas, bx, 0, box_w, box_h, Rgba([255, 255, 255, 230]));
    for (x, y, p) in thumb.enumerate_pixels() {
        let a = (p[3] as f32 * THUMBNAIL_OPACITY) as u8;
        // 枠の上に重ねるのではなく置き換える（透過画像でも枠の色が透けないように）
        canvas.put_pixel(bx + border + x, border + y, Rgba([p[0], p[1], p[2], a]));
    }

    // ファイル名（半透明の黒い帯に白い文字）
    if let (Some((text, w, h, pad)), Some(font)) = (label, font) {
        let lx = (width - w) / 2;
        let ly = box_h + gap;
        fill_rect(&mut canvas, lx, ly, w, h, Rgba([0, 0, 0, 180]));
        let font = font.as_scaled(PxScale::from(LABEL_FONT_SIZE * s));
        draw_text(&mut canvas, &font, &text, (lx + pad) as f32, (ly + pad) as f32 + font.ascent());
    }

    // コピー / 移動のバッジ（サムネイルの左上）
    let (badge_text, badge_color) = action.badge();
    let (bx0, by0) = (bx + border + px(2.0), border + px(2.0));
    match font {
        Some(font) => {
            let font = font.as_scaled(PxScale::from(LABEL_FONT_SIZE * s));
            let pad = px(LABEL_PADDING);
            let w = text_width(&font, badge_text).ceil() as u32 + pad * 2;
            let h = (font.ascent() - font.descent()).ceil() as u32 + pad;
            fill_rect(&mut canvas, bx0, by0, w, h, badge_color);
            draw_text(&mut canvas, &font, badge_text, (bx0 + pad) as f32, by0 as f32 + pad as f32 / 2.0 + font.ascent());
        }
        None => {
            let side = px(12.0);
            fill_rect(&mut canvas, bx0, by0, side, side, badge_color);
        }
    }
    canvas
}

fn text_width<F: Font>(font: &ab_glyph::PxScaleFont<&F>, text: &str) -> f32 {
    let mut width = 0.0;
    let mut prev = None;
    for c in text.chars() {
        let id = font.glyph_id(c);
        if let Some(p) = prev {
            width += font.kern(p, id);
        }
        width += font.h_advance(id);
        prev = Some(id);
    }
    width
}

/// 幅 `max_width` に収まるよう、必要なら末尾を削って `…` を付ける
fn ellipsize<F: Font>(font: &ab_glyph::PxScaleFont<&F>, text: &str, max_width: f32) -> String {
    if text_width(font, text) <= max_width {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    (0..chars.len())
        .rev()
        .map(|n| chars[..n].iter().collect::<String>() + "…")
        .find(|t| text_width(font, t) <= max_width)
        .unwrap_or_default()
}

/// ベースライン `baseline` から 1 行の文字を白で描く
fn draw_text<F: Font>(canvas: &mut RgbaImage, font: &ab_glyph::PxScaleFont<&F>, text: &str, x: f32, baseline: f32) {
    let mut caret = x;
    let mut prev = None;
    for c in text.chars() {
        let id = font.glyph_id(c);
        if let Some(p) = prev {
            caret += font.kern(p, id);
        }
        let glyph = id.with_scale_and_position(font.scale(), ab_glyph::point(caret, baseline));
        caret += font.h_advance(id);
        prev = Some(id);
        let Some(outline) = font.outline_glyph(glyph) else { continue };
        let bounds = outline.px_bounds();
        outline.draw(|gx, gy, coverage| {
            let (px, py) = (bounds.min.x as i32 + gx as i32, bounds.min.y as i32 + gy as i32);
            if px >= 0 && py >= 0 && (px as u32) < canvas.width() && (py as u32) < canvas.height() {
                blend(canvas, px as u32, py as u32, Rgba([255, 255, 255, (coverage.min(1.0) * 255.0) as u8]));
            }
        });
    }
}

fn fill_rect(canvas: &mut RgbaImage, x: u32, y: u32, w: u32, h: u32, color: Rgba<u8>) {
    for yy in y..(y + h).min(canvas.height()) {
        for xx in x..(x + w).min(canvas.width()) {
            canvas.put_pixel(xx, yy, color);
        }
    }
}

/// ストレートアルファの `src` を `(x, y)` の画素の上に重ねる
fn blend(canvas: &mut RgbaImage, x: u32, y: u32, src: Rgba<u8>) {
    let dst = *canvas.get_pixel(x, y);
    let sa = src[3] as f32 / 255.0;
    let da = dst[3] as f32 / 255.0;
    let out_a = sa + da * (1.0 - sa);
    if out_a <= 0.0 {
        return;
    }
    let ch = |i: usize| {
        ((src[i] as f32 * sa + dst[i] as f32 * da * (1.0 - sa)) / out_a).round() as u8
    };
    canvas.put_pixel(x, y, Rgba([ch(0), ch(1), ch(2), (out_a * 255.0).round() as u8]));
}

/// Wayland の ARGB8888（乗算済みアルファ、リトルエンディアンなので B, G, R, A の順）に変換する
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn to_premultiplied_bgra(img: &RgbaImage) -> Vec<u8> {
    img.pixels()
        .flat_map(|p| {
            let a = p[3] as u16;
            let m = |c: u8| ((c as u16 * a + 127) / 255) as u8;
            [m(p[2]), m(p[1]), m(p[0]), p[3]]
        })
        .collect()
}

/// X11 の 24 ビット画面用に、背景色 `bg` の上に重ねた B, G, R, X の列に変換する（X11 の窓は透過できないため）
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn to_opaque_bgrx(img: &RgbaImage, bg: [u8; 3]) -> Vec<u8> {
    img.pixels()
        .flat_map(|p| {
            let a = p[3] as u16;
            let mix = |c: u8, b: u8| ((c as u16 * a + b as u16 * (255 - a) + 127) / 255) as u8;
            [mix(p[2], bg[2]), mix(p[1], bg[1]), mix(p[0], bg[0]), 0]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn thumb(w: u32, h: u32) -> RgbaImage {
        RgbaImage::from_pixel(w, h, Rgba([200, 10, 10, 255]))
    }

    #[test]
    fn thumbnail_is_fit_and_framed() {
        let icon = render(&thumb(256, 128), "a.png", None, 1.0, DragAction::Copy);
        // 長辺 128 に縮小 + 枠 1px ずつ、ファイル名なし
        assert_eq!(icon.dimensions(), (130, 66));
        assert_eq!(icon.get_pixel(0, 0)[3], 230, "枠");
        assert_eq!(icon.get_pixel(60, 40).0, [200, 10, 10, 229], "少し透けたサムネイル");
        assert_eq!(icon.get_pixel(4, 4).0, [46, 160, 67, 240], "コピーのバッジ");
        let moving = render(&thumb(256, 128), "a.png", None, 1.0, DragAction::Move);
        assert_eq!(moving.get_pixel(4, 4).0, [230, 120, 20, 240], "移動のバッジ");

        // 小さい画像は拡大しない。高 DPI では倍の大きさで描く
        assert_eq!(render(&thumb(20, 10), "a.png", None, 1.0, DragAction::Copy).dimensions(), (22, 12));
        assert_eq!(render(&thumb(256, 128), "a.png", None, 2.0, DragAction::Copy).dimensions(), (260, 132));
    }

    #[test]
    fn color_conversions() {
        let img = RgbaImage::from_pixel(1, 1, Rgba([255, 128, 0, 128]));
        assert_eq!(to_premultiplied_bgra(&img), [0, 64, 128, 128]);
        assert_eq!(to_opaque_bgrx(&img, [0, 0, 0]), [0, 64, 128, 0]);
        assert_eq!(to_opaque_bgrx(&img, [255, 255, 255]), [127, 191, 255, 0]);
    }

    /// 同梱フォントがあれば、ファイル名の帯が付き、長い名前は省略される
    #[test]
    fn label_is_drawn_and_ellipsized() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/fonts/HackGen_NF_v2.10.0/HackGenConsoleNF-Regular.ttf");
        let Ok(bytes) = std::fs::read(path) else { return };
        let font = FontArc::try_from_vec(bytes).unwrap();
        let short = render(&thumb(128, 128), "a.png", Some(&font), 1.0, DragAction::Copy);
        assert!(short.height() > 130, "帯の分だけ高くなる");
        assert_eq!(short.width(), 130, "短い名前ならサムネイルの幅のまま");

        let long = render(&thumb(128, 128), &"とても長いファイル名".repeat(10), Some(&font), 1.0, DragAction::Move);
        assert!((150..=160).contains(&long.width()), "長い名前は LABEL_MIN_WIDTH で打ち切る: {}", long.width());
        let scaled = font.as_scaled(PxScale::from(LABEL_FONT_SIZE));
        let text = ellipsize(&scaled, &"とても長いファイル名".repeat(10), 154.0);
        assert!(text.ends_with('…') && text_width(&scaled, &text) <= 154.0);
    }
}
