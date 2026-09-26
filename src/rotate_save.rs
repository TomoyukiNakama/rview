//! 回転した画像をメタデータを保ったまま保存する。
//!
//! 画素は回転して再エンコードするが、EXIF・ICC・XMP・テキストチャンク・コメントなどの
//! メタデータは元ファイルからバイト列のまま引き継ぐ。回転が画素に焼き込まれるので、
//! EXIF の Orientation は 1（回転なし）に直す。そのままだと Orientation を解釈する
//! ビューアで二重に回転してしまう。

use std::io::Cursor;
use std::path::{Path, PathBuf};

use image::{DynamicImage, ImageFormat};

type Result<T> = std::result::Result<T, String>;

// EXIF / TIFF のタグ番号
const TAG_ORIENTATION: u16 = 274;
const TAG_X_RESOLUTION: u16 = 282;
const TAG_Y_RESOLUTION: u16 = 283;
const TAG_EXIF_IFD: u16 = 34665;
const TAG_GPS_IFD: u16 = 34853;
const TAG_INTEROP_IFD: u16 = 40965;
const TAG_PIXEL_X_DIMENSION: u16 = 40962;
const TAG_PIXEL_Y_DIMENSION: u16 = 40963;
const TAG_STRIP_OFFSETS: u16 = 273;
const TAG_STRIP_BYTE_COUNTS: u16 = 279;
const TAG_PHOTOMETRIC: u16 = 262;
const TAG_ICC_PROFILE: u16 = 34675;
const TAG_COMPRESSION: u16 = 259;

/// 画素に施す変換。元画像を（`mirror` なら）左右反転してから、`angle` 度時計回りに回転する。
/// 上下反転は「左右反転 + 180 度回転」で表せるので、この 2 つであらゆる回転・反転を表せる
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Transform {
    pub angle: u32, // 0 / 90 / 180 / 270
    pub mirror: bool,
}

impl Transform {
    /// 画面上の見た目（`rotation` 度回転してから左右 / 上下反転）を変換に直す。
    /// 左右反転 F と回転 R には F∘R(r) = R(-r)∘F、上下反転は R(180)∘F の関係がある
    pub fn from_view(rotation: u32, flip_h: bool, flip_v: bool) -> Self {
        let r = rotation as i32;
        let (angle, mirror) = match (flip_h, flip_v) {
            (false, false) => (r, false),
            (true, true) => (r + 180, false), // 左右 + 上下反転 = 180 度回転
            (true, false) => (-r, true),
            (false, true) => (180 - r, true),
        };
        Self { angle: angle.rem_euclid(360) as u32, mirror }
    }

    /// 幅と高さが入れ替わるか
    pub fn swaps_xy(self) -> bool {
        self.angle % 180 != 0
    }

    /// 何もしない変換か
    pub fn is_identity(self) -> bool {
        self.angle == 0 && !self.mirror
    }

    /// `self` を施した後に `next` を施す合成変換。
    /// R(a2)∘F^m2∘R(a1)∘F^m1 = R(a2 ± a1)∘F^(m1 xor m2)（m2 なら F∘R(a1) = R(-a1)∘F で符号が反転）
    pub fn then(self, next: Transform) -> Self {
        let a1 = if next.mirror { 360 - self.angle } else { self.angle };
        Self { angle: (next.angle + a1) % 360, mirror: self.mirror != next.mirror }
    }

    /// 逆変換。反転を含む変換 R(a)∘F は自分自身が逆変換になる
    pub fn inverse(self) -> Self {
        if self.mirror {
            self
        } else {
            Self { angle: (360 - self.angle) % 360, mirror: false }
        }
    }
}

/// `src` に変換 `t` を施して `dst` に保存する。
/// `src == dst` なら上書き保存になる。一時ファイルに書いてから rename するので、
/// 途中で失敗しても元ファイルは壊れない。
pub fn save_transformed(src: &Path, dst: &Path, t: Transform) -> Result<()> {
    if !matches!(t.angle, 0 | 90 | 180 | 270) {
        return Err(format!("未対応の角度です: {}", t.angle));
    }
    if t.angle == 0 && !t.mirror {
        return Err("回転・反転していません".into());
    }
    let data = std::fs::read(src).map_err(|e| format!("読み込みエラー: {e}"))?;
    let out = transform_bytes(&data, t)?;
    write_atomic(src, dst, &out)
}

/// 新規保存用の保存先パスを作る（`元名_<suffix>.ext`、既存なら `元名_<suffix>_1.ext` …）
pub fn unique_path(src: &Path, suffix: &str) -> PathBuf {
    let dir = src.parent().unwrap_or(Path::new("."));
    let stem = src.file_stem().and_then(|s| s.to_str()).unwrap_or("image");
    let ext = src.extension().and_then(|e| e.to_str());
    let make = |n_suffix: String| {
        let name = match ext {
            Some(ext) => format!("{stem}_{base}{n}.{ext}", base = suffix, n = n_suffix),
            None => format!("{stem}_{base}{n}", base = suffix, n = n_suffix),
        };
        dir.join(name)
    };
    let first = make(String::new());
    if !first.exists() {
        return first;
    }
    (1..)
        .map(|n| make(format!("_{n}")))
        .find(|p| !p.exists())
        .unwrap() // 無限イテレータなので必ず見つかる
}

/// ファイルの中身（バイト列）を変換し、同じ形式のバイト列を返す
fn transform_bytes(data: &[u8], t: Transform) -> Result<Vec<u8>> {
    if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        rotate_png(data, t)
    } else if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        rotate_jpeg(data, t)
    } else if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        rotate_gif(data, t)
    } else if data.len() >= 12 && &data[0..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        rotate_webp(data, t)
    } else if data.starts_with(b"II*\0") || data.starts_with(b"MM\0*") {
        rotate_tiff(data, t)
    } else if data.starts_with(b"II+\0") || data.starts_with(b"MM\0+") {
        Err("BigTIFF は保存に未対応です".into())
    } else if data.starts_with(b"BM") {
        rotate_bmp(data, t)
    } else {
        Err("保存に未対応の形式です".into())
    }
}

/// 同じフォルダの一時ファイルに書き込んでから `dst` へ rename する
fn write_atomic(src: &Path, dst: &Path, bytes: &[u8]) -> Result<()> {
    let dir = dst.parent().unwrap_or(Path::new("."));
    let name = dst.file_name().and_then(|n| n.to_str()).unwrap_or("image");
    let tmp = dir.join(format!(".{name}.rview-tmp"));
    let result = (|| -> std::io::Result<()> {
        std::fs::write(&tmp, bytes)?;
        // 上書き・新規どちらも元ファイルのパーミッションに揃える
        if let Ok(meta) = std::fs::metadata(src) {
            std::fs::set_permissions(&tmp, meta.permissions())?;
        }
        std::fs::rename(&tmp, dst)
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("書き込みエラー: {e}"));
    }
    Ok(())
}

fn rotate_image(img: &DynamicImage, t: Transform) -> DynamicImage {
    let flipped;
    let img = if t.mirror {
        flipped = img.fliph();
        &flipped
    } else {
        img
    };
    match t.angle {
        90 => img.rotate90(),
        180 => img.rotate180(),
        270 => img.rotate270(),
        _ => img.clone(),
    }
}

fn decode(data: &[u8], format: ImageFormat) -> Result<DynamicImage> {
    image::load_from_memory_with_format(data, format).map_err(|e| format!("デコードエラー: {e}"))
}

fn encode(img: &DynamicImage, format: ImageFormat) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    img.write_to(&mut Cursor::new(&mut out), format)
        .map_err(|e| format!("エンコードエラー: {e}"))?;
    Ok(out)
}

fn be16(b: &[u8]) -> u16 {
    u16::from_be_bytes([b[0], b[1]])
}
fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}
fn le32(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

// ───────────────────────────── PNG ─────────────────────────────

struct PngChunk<'a> {
    kind: [u8; 4],
    data: &'a [u8],
    raw: &'a [u8], // 長さ・種別・データ・CRC を含むチャンク全体
}

fn png_chunks(data: &[u8]) -> Result<Vec<PngChunk<'_>>> {
    let mut chunks = Vec::new();
    let mut pos = 8;
    while pos + 12 <= data.len() {
        let len = be32(&data[pos..]) as usize;
        let end = pos + 12 + len;
        if end > data.len() {
            return Err("PNG のチャンクが壊れています".into());
        }
        let kind = [data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]];
        chunks.push(PngChunk { kind, data: &data[pos + 8..pos + 8 + len], raw: &data[pos..end] });
        pos = end;
        if &kind == b"IEND" {
            break;
        }
    }
    Ok(chunks)
}

fn png_chunk_bytes(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 12);
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut hasher = crc32fast::Hasher::new();
    hasher.update(kind);
    hasher.update(data);
    out.extend_from_slice(&hasher.finalize().to_be_bytes());
    out
}

fn rotate_png(data: &[u8], t: Transform) -> Result<Vec<u8>> {
    let orig = png_chunks(data)?;
    if orig.iter().any(|c| &c.kind == b"acTL") {
        return Err("アニメーション PNG は保存に未対応です".into());
    }
    let img = decode(data, ImageFormat::Png)?;
    let encoded = encode(&rotate_image(&img, t), ImageFormat::Png)?;
    let new = png_chunks(&encoded)?;

    // IHDR の色形式（ビット深度・カラータイプ）が変わらなければ、
    // それに依存する bKGD / sBIT も意味が変わらないのでそのまま残せる
    let ihdr_format = |chunks: &[PngChunk]| {
        chunks.iter().find(|c| &c.kind == b"IHDR").map(|c| (c.data[8], c.data[9]))
    };
    let same_format = ihdr_format(&orig) == ihdr_format(&new);
    let swap_xy = t.swaps_xy();

    // 画像構造そのもの。新しいエンコード結果のものを使う
    let is_structural = |k: &[u8; 4]| matches!(k, b"IHDR" | b"PLTE" | b"tRNS" | b"IDAT" | b"IEND");
    // 元の色形式に依存するチャンク。色形式が変わったら捨てる
    let is_format_dependent = |k: &[u8; 4]| matches!(k, b"bKGD" | b"sBIT" | b"hIST");

    let mut out = data[..8].to_vec();
    let push_new = |out: &mut Vec<u8>, pred: &dyn Fn(&[u8; 4]) -> bool| {
        for c in new.iter().filter(|c| pred(&c.kind)) {
            out.extend_from_slice(c.raw);
        }
    };
    push_new(&mut out, &|k| matches!(k, b"IHDR" | b"PLTE" | b"tRNS"));

    let mut idat_written = false;
    for c in &orig {
        if &c.kind == b"IDAT" {
            // 元の IDAT の位置に新しい IDAT をまとめて置く（前後のチャンク順を保つ）
            if !idat_written {
                push_new(&mut out, &|k| k == b"IDAT");
                idat_written = true;
            }
            continue;
        }
        if is_structural(&c.kind) || (is_format_dependent(&c.kind) && !same_format) {
            continue;
        }
        match &c.kind {
            b"pHYs" if swap_xy && c.data.len() == 9 => {
                let mut d = c.data.to_vec();
                d[0..8].rotate_left(4); // x と y の画素密度を入れ替える
                out.extend(png_chunk_bytes(&c.kind, &d));
            }
            b"eXIf" => {
                let mut d = c.data.to_vec();
                patch_exif_tiff(&mut d, swap_xy);
                out.extend(png_chunk_bytes(&c.kind, &d));
            }
            _ => out.extend_from_slice(c.raw),
        }
    }
    push_new(&mut out, &|k| k == b"IEND");
    Ok(out)
}

// ───────────────────────────── JPEG ─────────────────────────────

fn rotate_jpeg(data: &[u8], t: Transform) -> Result<Vec<u8>> {
    // SOS までのマーカーセグメントを走査し、APPn / COM を集める
    let mut meta: Vec<Vec<u8>> = Vec::new();
    let mut components = 3u8;
    let mut pos = 2;
    while pos + 4 <= data.len() {
        if data[pos] != 0xFF {
            return Err("JPEG のマーカーが壊れています".into());
        }
        let marker = data[pos + 1];
        if marker == 0xFF {
            pos += 1; // フィルバイト
            continue;
        }
        if marker == 0xDA || marker == 0xD9 {
            break; // SOS 以降はエントロピー符号化データ
        }
        let len = be16(&data[pos + 2..]) as usize;
        let end = pos + 2 + len;
        if len < 2 || end > data.len() {
            return Err("JPEG のセグメントが壊れています".into());
        }
        let seg = &data[pos..end];
        match marker {
            // SOF（DHT=C4 / JPG=C8 / DAC=CC を除く）: 成分数を控える
            0xC0..=0xCF if !matches!(marker, 0xC4 | 0xC8 | 0xCC) => {
                if seg.len() > 9 {
                    components = seg[9];
                }
            }
            // APP14 (Adobe) は色変換の指定であってメタデータではない。
            // 新しいエンコード結果は YCbCr なので、元の指定を残すと色が化ける
            0xEE => {}
            0xE0..=0xEF | 0xFE => meta.push(seg.to_vec()),
            _ => {}
        }
        pos = end;
    }

    let swap_xy = t.swaps_xy();
    for seg in &mut meta {
        if seg[1] == 0xE1 && seg[4..].starts_with(b"Exif\0\0") {
            patch_exif_tiff(&mut seg[10..], swap_xy);
        }
        // JFIF の画素密度（X: 12..14、Y: 14..16）も縦横を入れ替える
        if swap_xy && seg[1] == 0xE0 && seg[4..].starts_with(b"JFIF\0") && seg.len() >= 16 {
            let (x, y) = seg[12..16].split_at_mut(2);
            x.swap_with_slice(y);
        }
    }
    // CMYK の JPEG は RGB にデコードされるので、CMYK 用の ICC プロファイルは合わなくなる
    if components == 4 {
        meta.retain(|s| !(s[1] == 0xE2 && s[4..].starts_with(b"ICC_PROFILE\0")));
    }

    let img = decode(data, ImageFormat::Jpeg)?;
    let rotated = rotate_image(&img, t);
    let rotated = match rotated {
        DynamicImage::ImageLuma8(_) | DynamicImage::ImageRgb8(_) => rotated,
        other => DynamicImage::ImageRgb8(other.to_rgb8()),
    };
    let mut encoded = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut encoded, 95)
        .encode_image(&rotated)
        .map_err(|e| format!("エンコードエラー: {e}"))?;

    // エンコーダが付けた先頭の APPn（JFIF など）を飛ばす
    let mut body = 2;
    while body + 4 <= encoded.len()
        && encoded[body] == 0xFF
        && (0xE0..=0xEF).contains(&encoded[body + 1])
    {
        body += 2 + be16(&encoded[body + 2..]) as usize;
    }
    // 元に APP セグメントが 1 つも無ければ、エンコーダの JFIF をそのまま使う
    if !meta.iter().any(|s| (0xE0..=0xEF).contains(&s[1])) {
        body = 2;
    }

    let mut out = vec![0xFF, 0xD8];
    for seg in &meta {
        out.extend_from_slice(seg);
    }
    out.extend_from_slice(&encoded[body..]);
    Ok(out)
}

// ───────────────────────────── EXIF / TIFF 共通 ─────────────────────────────

#[derive(Clone, Copy)]
struct Endian {
    little: bool,
}

impl Endian {
    fn u16(self, b: &[u8]) -> u16 {
        if self.little { u16::from_le_bytes([b[0], b[1]]) } else { be16(b) }
    }
    fn u32(self, b: &[u8]) -> u32 {
        if self.little { le32(b) } else { be32(b) }
    }
    fn put_u16(self, b: &mut [u8], v: u16) {
        let bytes = if self.little { v.to_le_bytes() } else { v.to_be_bytes() };
        b[..2].copy_from_slice(&bytes);
    }
}

fn tiff_endian(tiff: &[u8]) -> Option<Endian> {
    match tiff.get(0..4)? {
        b"II*\0" => Some(Endian { little: true }),
        b"MM\0*" => Some(Endian { little: false }),
        _ => None,
    }
}

/// 埋め込み EXIF（TIFF 構造）をその場で書き換える。長さを変えないので、
/// MakerNote などの内部オフセットが壊れない。
/// - IFD0 の Orientation を 1 にする
/// - `swap_xy` なら ExifIFD の PixelXDimension / PixelYDimension と
///   IFD0 の XResolution / YResolution を入れ替える
///
/// 構造が読めない EXIF は何もしない（壊すよりはそのまま残す）
fn patch_exif_tiff(tiff: &mut [u8], swap_xy: bool) {
    let Some(e) = tiff_endian(tiff) else { return };
    if tiff.len() < 8 {
        return;
    }
    let ifd0 = e.u32(&tiff[4..]) as usize;
    // IFD 内のエントリ位置（タグ → エントリ先頭オフセット）を集める
    let entries = |tiff: &[u8], ifd: usize| -> Vec<(u16, usize)> {
        let Some(n) = tiff.get(ifd..ifd + 2).map(|b| e.u16(b) as usize) else { return vec![] };
        (0..n)
            .map(|i| ifd + 2 + i * 12)
            .take_while(|&p| p + 12 <= tiff.len())
            .map(|p| (e.u16(&tiff[p..]), p))
            .collect()
    };
    let ifd0_entries = entries(tiff, ifd0);
    let find = |list: &[(u16, usize)], tag| list.iter().find(|(t, _)| *t == tag).map(|&(_, p)| p);

    if let Some(p) = find(&ifd0_entries, TAG_ORIENTATION) {
        // SHORT 1 個はエントリ内の値フィールドに入っている
        if e.u16(&tiff[p + 2..]) == 3 {
            e.put_u16(&mut tiff[p + 8..], 1);
        }
    }
    if !swap_xy {
        return;
    }
    // X/YResolution は RATIONAL なので、値フィールドはデータへのオフセット。
    // オフセット同士を入れ替えれば値が入れ替わる
    if let (Some(px), Some(py)) =
        (find(&ifd0_entries, TAG_X_RESOLUTION), find(&ifd0_entries, TAG_Y_RESOLUTION))
    {
        swap_entry_values(tiff, px, py);
    }
    if let Some(p) = find(&ifd0_entries, TAG_EXIF_IFD) {
        let exif_ifd = e.u32(&tiff[p + 8..]) as usize;
        let exif_entries = entries(tiff, exif_ifd);
        if let (Some(px), Some(py)) = (
            find(&exif_entries, TAG_PIXEL_X_DIMENSION),
            find(&exif_entries, TAG_PIXEL_Y_DIMENSION),
        ) {
            // SHORT / LONG が混在しても型ごと入れ替えれば正しい
            swap_entry_values(tiff, px, py);
        }
    }
}

/// 2 つの IFD エントリの「型・個数・値」（タグ以外の 10 バイト）を入れ替える
fn swap_entry_values(tiff: &mut [u8], a: usize, b: usize) {
    let mut tmp = [0u8; 10];
    tmp.copy_from_slice(&tiff[a + 2..a + 12]);
    tiff.copy_within(b + 2..b + 12, a + 2);
    tiff[b + 2..b + 12].copy_from_slice(&tmp);
}

/// IFD エントリ。`data` は値のバイト列（元ファイルのバイト順のまま）
#[derive(Clone)]
struct IfdEntry {
    tag: u16,
    typ: u16,
    count: u32,
    data: Vec<u8>,
    // ExifIFD などサブ IFD へのポインタなら、その中身
    sub: Option<Vec<IfdEntry>>,
}

/// TIFF の型ごとの 1 要素のバイト数と、バイト順を入れ替える単位
fn tiff_type_size(typ: u16) -> Option<(usize, usize)> {
    Some(match typ {
        1 | 2 | 6 | 7 => (1, 1),
        3 | 8 => (2, 2),
        4 | 9 | 11 | 13 => (4, 4),
        5 | 10 => (8, 4), // RATIONAL は 4 バイト整数 2 個
        12 => (8, 8),
        _ => return None,
    })
}

fn read_ifd(tiff: &[u8], e: Endian, offset: usize, depth: u32) -> Result<(Vec<IfdEntry>, u32)> {
    let bad = || "TIFF の IFD が壊れています".to_string();
    let n = e.u16(tiff.get(offset..offset + 2).ok_or_else(bad)?) as usize;
    let table = tiff.get(offset + 2..offset + 2 + n * 12 + 4).ok_or_else(bad)?;
    let mut entries = Vec::with_capacity(n);
    for i in 0..n {
        let ent = &table[i * 12..i * 12 + 12];
        let tag = e.u16(ent);
        let typ = e.u16(&ent[2..]);
        let count = e.u32(&ent[4..]);
        // 未知の型は長さが分からないので読めない。値ごと捨てる
        let Some((size, _)) = tiff_type_size(typ) else { continue };
        let len = size.checked_mul(count as usize).ok_or_else(bad)?;
        let data = if len <= 4 {
            ent[8..8 + len].to_vec()
        } else {
            let off = e.u32(&ent[8..]) as usize;
            tiff.get(off..off + len).ok_or_else(bad)?.to_vec()
        };
        let is_pointer = matches!(tag, TAG_EXIF_IFD | TAG_GPS_IFD | TAG_INTEROP_IFD) && count == 1;
        let sub = if is_pointer && depth < 3 {
            Some(read_ifd(tiff, e, e.u32(&data) as usize, depth + 1)?.0)
        } else {
            None
        };
        entries.push(IfdEntry { tag, typ, count, data, sub });
    }
    Ok((entries, e.u32(&table[n * 12..])))
}

/// エントリの値をビッグエンディアンからリトルエンディアンへ（またはその逆へ）変換する
fn swap_entry_endian(entries: &mut [IfdEntry]) {
    for ent in entries {
        if let Some((_, unit)) = tiff_type_size(ent.typ) {
            if unit > 1 {
                for chunk in ent.data.chunks_mut(unit) {
                    chunk.reverse();
                }
            }
        }
        if let Some(sub) = &mut ent.sub {
            swap_entry_endian(sub);
        }
    }
}

/// エントリ列を IFD として `out` の末尾に書き出し、その IFD のオフセットを返す（リトルエンディアン）
fn write_ifd(out: &mut Vec<u8>, entries: &mut [IfdEntry]) -> u32 {
    // 先にサブ IFD を書いて、ポインタの値を確定させる
    for ent in entries.iter_mut() {
        if let Some(sub) = &mut ent.sub {
            let off = write_ifd(out, sub);
            ent.typ = 4;
            ent.count = 1;
            ent.data = off.to_le_bytes().to_vec();
        }
    }
    entries.sort_by_key(|e| e.tag); // TIFF 仕様上、タグ昇順が必須
    // 4 バイトに収まらない値は IFD の前に置く
    let mut value_offsets = Vec::with_capacity(entries.len());
    for ent in entries.iter() {
        if ent.data.len() > 4 {
            if out.len() % 2 == 1 {
                out.push(0); // 値はワード境界に置く
            }
            value_offsets.push(Some(out.len() as u32));
            out.extend_from_slice(&ent.data);
        } else {
            value_offsets.push(None);
        }
    }
    if out.len() % 2 == 1 {
        out.push(0);
    }
    let ifd_offset = out.len() as u32;
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    for (ent, off) in entries.iter().zip(value_offsets) {
        out.extend_from_slice(&ent.tag.to_le_bytes());
        out.extend_from_slice(&ent.typ.to_le_bytes());
        out.extend_from_slice(&ent.count.to_le_bytes());
        match off {
            Some(off) => out.extend_from_slice(&off.to_le_bytes()),
            None => {
                let mut v = [0u8; 4];
                v[..ent.data.len()].copy_from_slice(&ent.data);
                out.extend_from_slice(&v);
            }
        }
    }
    out.extend_from_slice(&0u32.to_le_bytes()); // 次の IFD なし
    ifd_offset
}

/// エントリの値を u32 の列として読む（SHORT / LONG、リトルエンディアン）
fn entry_u32s(ent: &IfdEntry) -> Vec<u32> {
    match ent.typ {
        3 => ent.data.chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]]) as u32).collect(),
        4 => ent.data.chunks(4).map(le32).collect(),
        _ => vec![],
    }
}

// ───────────────────────────── TIFF ─────────────────────────────

fn rotate_tiff(data: &[u8], t: Transform) -> Result<Vec<u8>> {
    let e = tiff_endian(data).ok_or("TIFF のヘッダが壊れています")?;
    let (mut orig, next) = read_ifd(data, e, e.u32(&data[4..]) as usize, 0)?;
    if next != 0 {
        return Err("複数ページの TIFF は保存に未対応です".into());
    }
    if !e.little {
        swap_entry_endian(&mut orig);
    }

    // 画素は image で回転・エンコードし、その結果の IFD から画像構造のタグと画素データを取る
    let img = decode(data, ImageFormat::Tiff)?;
    let rotated = rotate_image(&img, t);
    let compression = orig
        .iter()
        .find(|t| t.tag == TAG_COMPRESSION)
        .and_then(|t| entry_u32s(t).first().copied())
        .unwrap_or(1);
    let encoded = encode_tiff(&rotated, compression).map_err(|e| format!("エンコードエラー: {e}"))?;
    let ee = tiff_endian(&encoded).ok_or("TIFF のエンコードに失敗しました")?;
    let (mut new, _) = read_ifd(&encoded, ee, ee.u32(&encoded[4..]) as usize, 0)?;
    if !ee.little {
        swap_entry_endian(&mut new);
    }

    // 画像構造を表すタグ。元の値は新しい画素データと食い違うので捨てる
    const STRUCTURAL: &[u16] = &[
        256, 257, 258, 259, 262, 266, 273, 277, 278, 279, 284, 317, 320, 322, 323, 324, 325,
        330, 338, 339, 347, 513, 514, 530, 531, 532,
    ];
    let cmyk = orig
        .iter()
        .find(|t| t.tag == TAG_PHOTOMETRIC)
        .and_then(|t| entry_u32s(t).first().copied())
        == Some(5);
    let swap_xy = t.swaps_xy();

    let mut entries: Vec<IfdEntry> = new
        .iter()
        .filter(|t| STRUCTURAL.contains(&t.tag))
        .cloned()
        .collect();
    for mut t in orig {
        if STRUCTURAL.contains(&t.tag) || (cmyk && t.tag == TAG_ICC_PROFILE) {
            continue;
        }
        if t.tag == TAG_ORIENTATION && t.typ == 3 {
            t.data = 1u16.to_le_bytes().to_vec();
        }
        if swap_xy {
            t.tag = match t.tag {
                TAG_X_RESOLUTION => TAG_Y_RESOLUTION,
                TAG_Y_RESOLUTION => TAG_X_RESOLUTION,
                other => other,
            };
            if t.tag == TAG_EXIF_IFD {
                for s in t.sub.iter_mut().flatten() {
                    s.tag = match s.tag {
                        TAG_PIXEL_X_DIMENSION => TAG_PIXEL_Y_DIMENSION,
                        TAG_PIXEL_Y_DIMENSION => TAG_PIXEL_X_DIMENSION,
                        other => other,
                    };
                }
            }
        }
        entries.push(t);
    }
    // 元に無いタグ（解像度など）はエンコーダが付けたものを使う
    for t in &new {
        if !entries.iter().any(|x| x.tag == t.tag) {
            entries.push(t.clone());
        }
    }

    assemble_tiff(&encoded, &new, entries)
}

/// 画像を TIFF にエンコードする。圧縮方式は元ファイルの Compression タグに合わせる
/// （image の TIFF エンコーダは非圧縮しか書けず、LZW などの元ファイルより大きくなるため）。
/// 非可逆の JPEG 圧縮などは再現できないので非圧縮にする
fn encode_tiff(img: &DynamicImage, compression: u32) -> tiff::TiffResult<Vec<u8>> {
    use tiff::encoder::{colortype, compression::DeflateLevel, Compression, TiffEncoder};
    let compression = match compression {
        5 => Compression::Lzw,
        8 | 32946 => Compression::Deflate(DeflateLevel::default()),
        32773 => Compression::Packbits,
        _ => Compression::Uncompressed,
    };
    let mut buf = Cursor::new(Vec::new());
    let mut enc = TiffEncoder::new(&mut buf)?.with_compression(compression);
    let (w, h) = (img.width(), img.height());
    match img {
        DynamicImage::ImageLuma8(i) => enc.write_image::<colortype::Gray8>(w, h, i.as_raw())?,
        DynamicImage::ImageLuma16(i) => enc.write_image::<colortype::Gray16>(w, h, i.as_raw())?,
        DynamicImage::ImageRgb8(i) => enc.write_image::<colortype::RGB8>(w, h, i.as_raw())?,
        DynamicImage::ImageRgb16(i) => enc.write_image::<colortype::RGB16>(w, h, i.as_raw())?,
        DynamicImage::ImageRgb32F(i) => enc.write_image::<colortype::RGB32Float>(w, h, i.as_raw())?,
        DynamicImage::ImageRgba16(i) => enc.write_image::<colortype::RGBA16>(w, h, i.as_raw())?,
        DynamicImage::ImageRgba32F(i) => enc.write_image::<colortype::RGBA32Float>(w, h, i.as_raw())?,
        DynamicImage::ImageLumaA16(_) => {
            enc.write_image::<colortype::RGBA16>(w, h, img.to_rgba16().as_raw())?
        }
        _ => enc.write_image::<colortype::RGBA8>(w, h, img.to_rgba8().as_raw())?,
    }
    Ok(buf.into_inner())
}

/// エンコード済み TIFF（`encoded`、その IFD0 が `encoded_ifd`）の画素データと、
/// `entries` のタグから新しい TIFF（リトルエンディアン、1 ページ）を組み立てる
fn assemble_tiff(encoded: &[u8], encoded_ifd: &[IfdEntry], mut entries: Vec<IfdEntry>) -> Result<Vec<u8>> {
    // 画素データ（ストリップ）を新しいファイルの先頭側へ移し、オフセットを付け直す
    let find = |tag| encoded_ifd.iter().find(|t| t.tag == tag).map(entry_u32s).unwrap_or_default();
    let offsets = find(TAG_STRIP_OFFSETS);
    let counts = find(TAG_STRIP_BYTE_COUNTS);
    if offsets.is_empty() || offsets.len() != counts.len() {
        return Err("TIFF のエンコード結果が想定外です".into());
    }
    let mut out = b"II*\0\0\0\0\0".to_vec();
    let mut new_offsets = Vec::with_capacity(offsets.len());
    for (&off, &len) in offsets.iter().zip(&counts) {
        let strip = encoded
            .get(off as usize..off as usize + len as usize)
            .ok_or("TIFF のエンコード結果が想定外です")?;
        new_offsets.push(out.len() as u32);
        out.extend_from_slice(strip);
    }
    for t in entries.iter_mut() {
        if t.tag == TAG_STRIP_OFFSETS {
            t.typ = 4;
            t.count = new_offsets.len() as u32;
            t.data = new_offsets.iter().flat_map(|o| o.to_le_bytes()).collect();
        }
    }
    let ifd0 = write_ifd(&mut out, &mut entries);
    out[4..8].copy_from_slice(&ifd0.to_le_bytes());
    Ok(out)
}

// ───────────────────────────── WebP ─────────────────────────────

fn rotate_webp(data: &[u8], t: Transform) -> Result<Vec<u8>> {
    let mut icc = Vec::new();
    let mut exif = Vec::new();
    let mut xmp = Vec::new();
    let mut pos = 12;
    while pos + 8 <= data.len() {
        let kind = &data[pos..pos + 4];
        let len = le32(&data[pos + 4..]) as usize;
        let body = data.get(pos + 8..pos + 8 + len).ok_or("WebP のチャンクが壊れています")?;
        match kind {
            b"ANIM" | b"ANMF" => return Err("アニメーション WebP は保存に未対応です".into()),
            b"ICCP" => icc = body.to_vec(),
            b"EXIF" => exif = body.to_vec(),
            b"XMP " => xmp = body.to_vec(),
            _ => {}
        }
        pos += 8 + len + (len & 1); // 奇数長のチャンクは 1 バイトのパディングが付く
    }
    if !exif.is_empty() {
        // "Exif\0\0" が前置されている書き出し元もある
        let start = if exif.starts_with(b"Exif\0\0") { 6 } else { 0 };
        patch_exif_tiff(&mut exif[start..], t.swaps_xy());
    }

    let img = decode(data, ImageFormat::WebP)?;
    let rotated = rotate_image(&img, t);
    let (buf, color) = if rotated.color().has_alpha() {
        (rotated.to_rgba8().into_raw(), image_webp::ColorType::Rgba8)
    } else {
        (rotated.to_rgb8().into_raw(), image_webp::ColorType::Rgb8)
    };
    // image-webp は lossless エンコードのみ。画質は落ちないが、元が lossy だとサイズは増える
    let mut out = Vec::new();
    let mut encoder = image_webp::WebPEncoder::new(&mut out);
    if !icc.is_empty() {
        encoder.set_icc_profile(icc);
    }
    if !exif.is_empty() {
        encoder.set_exif_metadata(exif);
    }
    if !xmp.is_empty() {
        encoder.set_xmp_metadata(xmp);
    }
    encoder
        .encode(&buf, rotated.width(), rotated.height(), color)
        .map_err(|e| format!("エンコードエラー: {e}"))?;
    Ok(out)
}

// ───────────────────────────── GIF ─────────────────────────────

/// 1 バイト 1 画素のバッファを変換する（GIF のパレットインデックス用）
fn rotate_indices(buf: &[u8], w: usize, h: usize, t: Transform) -> Vec<u8> {
    let mut out = vec![0u8; buf.len()];
    for y in 0..h {
        for sx in 0..w {
            // 左右反転してから回転する
            let x = if t.mirror { w - 1 - sx } else { sx };
            let (nx, ny, nw) = match t.angle {
                90 => (h - 1 - y, x, h),
                180 => (w - 1 - x, h - 1 - y, w),
                270 => (y, w - 1 - x, h),
                _ => (x, y, w),
            };
            out[ny * nw + nx] = buf[y * w + sx];
        }
    }
    out
}

/// GIF のブロック列を走査して、コメント拡張とアプリケーション拡張（ループ指定以外）を
/// サブブロック単位で集める。gif クレートはこれらを読み飛ばすため自前で拾う
fn gif_extensions(data: &[u8]) -> Vec<(u8, Vec<Vec<u8>>)> {
    let mut found = Vec::new();
    let mut pos = 13;
    let flags = data.get(10).copied().unwrap_or(0);
    if flags & 0x80 != 0 {
        pos += 3 << ((flags & 0x07) + 1);
    }
    // サブブロック列を読み、終端の次の位置を返す
    let read_blocks = |mut p: usize, blocks: &mut Vec<Vec<u8>>| -> Option<usize> {
        loop {
            let n = *data.get(p)? as usize;
            if n == 0 {
                return Some(p + 1);
            }
            blocks.push(data.get(p + 1..p + 1 + n)?.to_vec());
            p += 1 + n;
        }
    };
    while let Some(&b) = data.get(pos) {
        match b {
            0x21 => {
                let Some(&label) = data.get(pos + 1) else { break };
                let mut blocks = Vec::new();
                let Some(next) = read_blocks(pos + 2, &mut blocks) else { break };
                let is_loop = blocks
                    .first()
                    .is_some_and(|id| id.starts_with(b"NETSCAPE2.0") || id.starts_with(b"ANIMEXTS1.0"));
                if label == 0xFE || (label == 0xFF && !is_loop) {
                    found.push((label, blocks));
                }
                pos = next;
            }
            0x2C => {
                let Some(&f) = data.get(pos + 9) else { break };
                let mut p = pos + 10;
                if f & 0x80 != 0 {
                    p += 3 << ((f & 0x07) + 1);
                }
                let mut skipped = Vec::new();
                let Some(next) = read_blocks(p + 1, &mut skipped) else { break };
                pos = next;
            }
            _ => break, // 0x3B（終端）または壊れたデータ
        }
    }
    found
}

fn rotate_gif(data: &[u8], t: Transform) -> Result<Vec<u8>> {
    let err = |e: gif::DecodingError| format!("デコードエラー: {e}");
    let mut opts = gif::DecodeOptions::new();
    opts.set_color_output(gif::ColorOutput::Indexed);
    let mut dec = opts.read_info(Cursor::new(data)).map_err(err)?;
    let (sw, sh) = (dec.width() as i32, dec.height() as i32);
    let global_palette = dec.global_palette().map(<[u8]>::to_vec).unwrap_or_default();
    let (nw, nh) = if t.swaps_xy() { (sh, sw) } else { (sw, sh) };

    let mut frames = Vec::new();
    while let Some(frame) = dec.read_next_frame().map_err(err)? {
        let (w, h) = (frame.width as i32, frame.height as i32);
        // フレームの位置を論理画面上で変換する（左右反転 → 回転）
        let l = if t.mirror { sw - frame.left as i32 - w } else { frame.left as i32 };
        let top = frame.top as i32;
        let (nl, nt) = match t.angle {
            90 => (sh - top - h, l),
            180 => (sw - l - w, sh - top - h),
            270 => (top, sw - l - w),
            _ => (l, top),
        };
        let (fw, fh) = if t.swaps_xy() { (h, w) } else { (w, h) };
        let mut f = frame.clone();
        // デコーダはインターレースを解いた順で返すので、書き出しも非インターレースにする
        f.buffer = rotate_indices(&frame.buffer, w as usize, h as usize, t).into();
        f.left = nl.max(0) as u16;
        f.top = nt.max(0) as u16;
        f.width = fw as u16;
        f.height = fh as u16;
        f.interlaced = false;
        frames.push(f);
    }
    if frames.is_empty() {
        return Err("GIF にフレームがありません".into());
    }
    // ループ指定はフレームを読み進める途中で見つかるので、全フレームを読んでから取る
    let repeat = dec.repeat();

    let eerr = |e: gif::EncodingError| format!("エンコードエラー: {e}");
    let mut out = Vec::new();
    {
        let mut enc = gif::Encoder::new(&mut out, nw as u16, nh as u16, &global_palette).map_err(eerr)?;
        // ループ指定は最初のフレームより前に置く必要がある
        if frames.len() > 1 {
            enc.set_repeat(repeat).map_err(eerr)?;
        }
        // コメント・XMP などのメタデータもヘッダ直後にまとめて置く
        for (label, blocks) in gif_extensions(data) {
            let slices: Vec<&[u8]> = blocks.iter().map(Vec::as_slice).collect();
            enc.write_raw_extension(gif::AnyExtension(label), &slices).map_err(eerr)?;
        }
        for f in &frames {
            enc.write_frame(f).map_err(eerr)?;
        }
    }
    Ok(out)
}

// ───────────────────────────── BMP ─────────────────────────────

fn rotate_bmp(data: &[u8], t: Transform) -> Result<Vec<u8>> {
    // BMP のメタデータは解像度くらいなので、回転して再エンコードし、解像度だけ引き継ぐ
    let img = decode(data, ImageFormat::Bmp)?;
    let mut out = encode(&rotate_image(&img, t), ImageFormat::Bmp)?;
    // BITMAPINFOHEADER 以降（ヘッダ長 40 以上）なら 38..42 が X、42..46 が Y の画素密度 (pixels/m)
    let has_resolution = |b: &[u8]| b.len() >= 46 && le32(&b[14..]) >= 40;
    if has_resolution(data) && has_resolution(&out) {
        let (x, y) = (&data[38..42], &data[42..46]);
        let (x, y) = if t.swaps_xy() { (y, x) } else { (x, y) };
        out[38..42].copy_from_slice(x);
        out[42..46].copy_from_slice(y);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{GenericImageView, Rgb, RgbImage};

    const R90: Transform = Transform { angle: 90, mirror: false };

    /// 3x2 のテスト画像。画素ごとに色が違うので回転の向きを検証できる
    fn sample() -> DynamicImage {
        let mut img = RgbImage::new(3, 2);
        for (x, y, p) in img.enumerate_pixels_mut() {
            *p = Rgb([x as u8 * 80, y as u8 * 120, 200]);
        }
        DynamicImage::ImageRgb8(img)
    }

    /// Orientation=6、XResolution=72/1、YResolution=300/1、PixelX/YDimension=3/2 を持つ EXIF（TIFF 構造）
    fn exif_blob() -> Vec<u8> {
        let rational = |n: u32| [n.to_le_bytes(), 1u32.to_le_bytes()].concat();
        let short = |v: u16| [v.to_le_bytes().as_slice(), &[0, 0]].concat();
        let mut ifd0 = vec![
            IfdEntry { tag: TAG_ORIENTATION, typ: 3, count: 1, data: short(6), sub: None },
            IfdEntry { tag: TAG_X_RESOLUTION, typ: 5, count: 1, data: rational(72), sub: None },
            IfdEntry { tag: TAG_Y_RESOLUTION, typ: 5, count: 1, data: rational(300), sub: None },
            IfdEntry {
                tag: TAG_EXIF_IFD,
                typ: 4,
                count: 1,
                data: vec![0; 4],
                sub: Some(vec![
                    IfdEntry { tag: TAG_PIXEL_X_DIMENSION, typ: 4, count: 1, data: 3u32.to_le_bytes().to_vec(), sub: None },
                    IfdEntry { tag: TAG_PIXEL_Y_DIMENSION, typ: 3, count: 1, data: short(2), sub: None },
                ]),
            },
        ];
        let mut out = b"II*\0\0\0\0\0".to_vec();
        let off = write_ifd(&mut out, &mut ifd0);
        out[4..8].copy_from_slice(&off.to_le_bytes());
        out
    }

    /// EXIF から (Orientation, XResolution 分子, YResolution 分子, PixelX, PixelY) を読む
    fn read_exif(tiff: &[u8]) -> (u32, u32, u32, u32, u32) {
        let e = tiff_endian(tiff).unwrap();
        let (mut ifd0, _) = read_ifd(tiff, e, e.u32(&tiff[4..]) as usize, 0).unwrap();
        if !e.little {
            swap_entry_endian(&mut ifd0);
        }
        let val = |list: &[IfdEntry], tag| {
            let t = list.iter().find(|t| t.tag == tag).unwrap();
            if t.typ == 5 { le32(&t.data) } else { entry_u32s(t)[0] }
        };
        let exif = ifd0.iter().find(|t| t.tag == TAG_EXIF_IFD).unwrap().sub.clone().unwrap();
        (
            val(&ifd0, TAG_ORIENTATION),
            val(&ifd0, TAG_X_RESOLUTION),
            val(&ifd0, TAG_Y_RESOLUTION),
            val(&exif, TAG_PIXEL_X_DIMENSION),
            val(&exif, TAG_PIXEL_Y_DIMENSION),
        )
    }

    fn assert_rotated(out: &[u8], format: ImageFormat) {
        let got = image::load_from_memory_with_format(out, format).unwrap();
        assert_eq!(got.dimensions(), (2, 3));
        let want = sample().rotate90();
        if format != ImageFormat::Jpeg {
            assert_eq!(got.to_rgb8(), want.to_rgb8());
        }
    }

    #[test]
    fn exif_patch_resets_orientation_and_swaps_dimensions() {
        let mut exif = exif_blob();
        patch_exif_tiff(&mut exif, true);
        assert_eq!(read_exif(&exif), (1, 300, 72, 2, 3));
        let mut exif = exif_blob();
        patch_exif_tiff(&mut exif, false);
        assert_eq!(read_exif(&exif), (1, 72, 300, 3, 2));
    }

    #[test]
    fn png_keeps_chunks() {
        let base = encode(&sample(), ImageFormat::Png).unwrap();
        let chunks = png_chunks(&base).unwrap();
        let mut phys = 1000u32.to_be_bytes().to_vec();
        phys.extend(2000u32.to_be_bytes());
        phys.push(1);
        let mut src = base[..8].to_vec();
        for c in &chunks {
            if &c.kind == b"IDAT" {
                src.extend(png_chunk_bytes(b"tEXt", b"Title\0before"));
                src.extend(png_chunk_bytes(b"pHYs", &phys));
                src.extend(png_chunk_bytes(b"eXIf", &exif_blob()));
            }
            if &c.kind == b"IEND" {
                src.extend(png_chunk_bytes(b"tEXt", b"Title\0after"));
            }
            src.extend_from_slice(c.raw);
        }

        let out = rotate_png(&src, R90).unwrap();
        assert_rotated(&out, ImageFormat::Png);
        let kinds: Vec<_> = png_chunks(&out).unwrap().iter().map(|c| c.kind).collect();
        let pos = |k: &[u8; 4]| kinds.iter().position(|x| x == k).unwrap();
        assert!(pos(b"tEXt") < pos(b"IDAT"), "IDAT 前のチャンクは IDAT 前に残る");
        assert_eq!(kinds.last(), Some(b"IEND"));
        let out_chunks = png_chunks(&out).unwrap();
        let texts: Vec<_> = out_chunks.iter().filter(|c| &c.kind == b"tEXt").map(|c| c.data).collect();
        assert_eq!(texts, [b"Title\0before".as_slice(), b"Title\0after"]);
        let phys_out = out_chunks.iter().find(|c| &c.kind == b"pHYs").unwrap().data;
        assert_eq!((be32(phys_out), be32(&phys_out[4..])), (2000, 1000));
        let exif_out = out_chunks.iter().find(|c| &c.kind == b"eXIf").unwrap().data;
        assert_eq!(read_exif(exif_out).0, 1);
    }

    #[test]
    fn apng_is_rejected() {
        let base = encode(&sample(), ImageFormat::Png).unwrap();
        let mut src = base[..33].to_vec(); // シグネチャ + IHDR
        src.extend(png_chunk_bytes(b"acTL", &[0, 0, 0, 1, 0, 0, 0, 0]));
        src.extend_from_slice(&base[33..]);
        assert!(rotate_png(&src, R90).is_err());
    }

    #[test]
    fn jpeg_keeps_segments() {
        let mut base = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut base, 95)
            .encode_image(&sample())
            .unwrap();
        let segment = |marker: u8, body: &[u8]| {
            let mut s = vec![0xFF, marker];
            s.extend(((body.len() + 2) as u16).to_be_bytes());
            s.extend_from_slice(body);
            s
        };
        let mut src = vec![0xFF, 0xD8];
        src.extend(segment(0xE1, &[b"Exif\0\0".as_slice(), &exif_blob()].concat()));
        src.extend(segment(0xFE, b"my comment"));
        src.extend(segment(0xEE, b"Adobe\0\x64\0\0\0\0\0"));
        src.extend_from_slice(&base[2..]);

        let out = rotate_jpeg(&src, R90).unwrap();
        assert_rotated(&out, ImageFormat::Jpeg);
        assert_eq!(&out[2..4], &[0xFF, 0xE1], "EXIF は SOI の直後");
        let exif_len = be16(&out[4..]) as usize;
        assert_eq!(read_exif(&out[12..4 + exif_len]), (1, 300, 72, 2, 3));
        let find = |pat: &[u8]| out.windows(pat.len()).any(|w| w == pat);
        assert!(find(b"my comment"));
        assert!(!find(b"Adobe"), "APP14 は捨てる");
        // 元の JFIF は引き継ぎ、エンコーダの JFIF は付けないので 1 個だけ
        assert_eq!(out.windows(4).filter(|w| w == b"JFIF").count(), 1);
    }

    #[test]
    fn gif_rotates_all_frames_and_keeps_extensions() {
        let mut src = Vec::new();
        {
            let palette = [0, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255, 9, 9, 9];
            let mut enc = gif::Encoder::new(&mut src, 3, 2, &palette).unwrap();
            enc.set_repeat(gif::Repeat::Infinite).unwrap();
            enc.write_raw_extension(gif::AnyExtension(0xFE), &[b"hello gif"]).unwrap();
            for buf in [[0u8, 1, 2, 3, 4, 5], [5, 4, 3, 2, 1, 0]] {
                let mut f = gif::Frame::default();
                f.width = 3;
                f.height = 2;
                f.delay = 7;
                f.buffer = buf.to_vec().into();
                enc.write_frame(&f).unwrap();
            }
        }

        let out = rotate_gif(&src, R90).unwrap();
        let mut opts = gif::DecodeOptions::new();
        opts.set_color_output(gif::ColorOutput::Indexed);
        let mut dec = opts.read_info(Cursor::new(&out)).unwrap();
        assert_eq!((dec.width(), dec.height()), (2, 3));
        let mut frames = Vec::new();
        while let Some(f) = dec.read_next_frame().unwrap() {
            assert_eq!((f.width, f.height, f.delay), (2, 3, 7));
            frames.push(f.buffer.to_vec());
        }
        // 3x2 の [0 1 2 / 3 4 5] を時計回りに 90 度 → 2x3 の [3 0 / 4 1 / 5 2]
        assert_eq!(frames, [vec![3, 0, 4, 1, 5, 2], vec![2, 5, 1, 4, 0, 3]]);
        assert_eq!(dec.repeat(), gif::Repeat::Infinite);
        assert_eq!(gif_extensions(&out), [(0xFE, vec![b"hello gif".to_vec()])]);
    }

    #[test]
    fn webp_keeps_metadata() {
        let mut src = Vec::new();
        let mut enc = image_webp::WebPEncoder::new(&mut src);
        enc.set_exif_metadata(exif_blob());
        enc.set_xmp_metadata(b"<x:xmpmeta/>".to_vec());
        enc.set_icc_profile(b"fake icc".to_vec());
        enc.encode(sample().as_bytes(), 3, 2, image_webp::ColorType::Rgb8).unwrap();

        let out = rotate_webp(&src, R90).unwrap();
        assert_rotated(&out, ImageFormat::WebP);
        let chunk = |kind: &[u8]| {
            let p = out.windows(4).position(|w| w == kind).unwrap();
            out[p + 8..p + 8 + le32(&out[p + 4..]) as usize].to_vec()
        };
        assert_eq!(read_exif(&chunk(b"EXIF")), (1, 300, 72, 2, 3));
        assert_eq!(chunk(b"XMP "), b"<x:xmpmeta/>");
        assert_eq!(chunk(b"ICCP"), b"fake icc");
    }

    /// Artist タグと EXIF サブ IFD を持つ TIFF を作る
    fn tiff_sample() -> Vec<u8> {
        let encoded = encode(&sample(), ImageFormat::Tiff).unwrap();
        let e = tiff_endian(&encoded).unwrap();
        let (ifd, _) = read_ifd(&encoded, e, e.u32(&encoded[4..]) as usize, 0).unwrap();
        let exif = exif_blob();
        let (exif_ifd0, _) = read_ifd(&exif, Endian { little: true }, le32(&exif[4..]) as usize, 0).unwrap();
        // エンコーダの解像度タグを EXIF 側の Orientation / 解像度 / ExifIFD で置き換える
        let mut tags: Vec<IfdEntry> = ifd
            .iter()
            .filter(|t| !matches!(t.tag, TAG_X_RESOLUTION | TAG_Y_RESOLUTION | TAG_ORIENTATION))
            .cloned()
            .collect();
        tags.extend(exif_ifd0);
        tags.push(IfdEntry { tag: 315, typ: 2, count: 6, data: b"artist".to_vec(), sub: None });
        tags.push(IfdEntry { tag: 33432, typ: 7, count: 3, data: b"(c)".to_vec(), sub: None });
        assemble_tiff(&encoded, &ifd, tags).unwrap()
    }

    #[test]
    fn tiff_keeps_tags() {
        let src = tiff_sample();
        let out = rotate_tiff(&src, R90).unwrap();
        assert_rotated(&out, ImageFormat::Tiff);
        assert_eq!(read_exif(&out), (1, 300, 72, 2, 3));
        let e = tiff_endian(&out).unwrap();
        let (ifd, _) = read_ifd(&out, e, e.u32(&out[4..]) as usize, 0).unwrap();
        let tag = |n| ifd.iter().find(|t| t.tag == n).unwrap();
        assert_eq!(tag(315).data, b"artist");
        assert_eq!((tag(33432).typ, tag(33432).data.as_slice()), (7, b"(c)".as_slice()), "UNDEFINED 型のまま残る");
    }

    #[test]
    fn multipage_tiff_is_rejected() {
        let mut src = tiff_sample();
        let e = tiff_endian(&src).unwrap();
        let ifd0 = e.u32(&src[4..]) as usize;
        let n = e.u16(&src[ifd0..]) as usize;
        let next = ifd0 + 2 + n * 12;
        src[next..next + 4].copy_from_slice(&(ifd0 as u32).to_le_bytes());
        assert!(rotate_tiff(&src, R90).is_err());
    }

    #[test]
    fn bmp_rotates() {
        let src = encode(&sample(), ImageFormat::Bmp).unwrap();
        assert_rotated(&rotate_bmp(&src, R90).unwrap(), ImageFormat::Bmp);
    }

    #[test]
    fn save_overwrite_and_new_name() {
        let dir = std::env::temp_dir().join(format!("rview-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("a.png");
        std::fs::write(&src, encode(&sample(), ImageFormat::Png).unwrap()).unwrap();

        let first = unique_path(&src, "rot90");
        assert_eq!(first, dir.join("a_rot90.png"));
        save_transformed(&src, &first, R90).unwrap();
        assert_eq!(unique_path(&src, "rot90"), dir.join("a_rot90_1.png"));
        assert!(save_transformed(&src, &src, Transform { angle: 0, mirror: false }).is_err());

        save_transformed(&src, &src, R90).unwrap();
        assert_eq!(image::open(&src).unwrap().dimensions(), (2, 3));
        let leftovers = std::fs::read_dir(&dir).unwrap().filter(|e| {
            e.as_ref().unwrap().file_name().to_string_lossy().ends_with(".rview-tmp")
        });
        assert_eq!(leftovers.count(), 0, "一時ファイルが残らない");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// 全 16 通りの「回転 → 左右 / 上下反転」の見た目が、変換（左右反転 → 回転）と一致する
    #[test]
    fn from_view_matches_screen_operations() {
        let img = sample();
        for r in [0, 90, 180, 270] {
            for (h, v) in [(false, false), (true, false), (false, true), (true, true)] {
                let mut want = rotate_image(&img, Transform { angle: r, mirror: false });
                if h {
                    want = want.fliph();
                }
                if v {
                    want = want.flipv();
                }
                let got = rotate_image(&img, Transform::from_view(r, h, v));
                assert_eq!(got.to_rgb8(), want.to_rgb8(), "r={r} h={h} v={v}");
            }
        }
    }

    /// GIF のインデックス変換とフレーム位置が、image の変換結果と一致する
    #[test]
    fn gif_mirror_and_rotate_match_image() {
        let mut src = Vec::new();
        {
            let palette: Vec<u8> = (0..8u8).flat_map(|i| [i * 30, 255 - i * 30, i * 10]).collect();
            let mut enc = gif::Encoder::new(&mut src, 4, 3, &palette).unwrap();
            // 論理画面 4x3 のうち、(1,1) から 3x2 の部分フレーム
            let mut f = gif::Frame::default();
            f.left = 1;
            f.top = 1;
            f.width = 3;
            f.height = 2;
            f.buffer = vec![1, 2, 3, 4, 5, 6].into();
            enc.write_frame(&f).unwrap();
        }
        let original = image::load_from_memory_with_format(&src, ImageFormat::Gif).unwrap();
        for angle in [0, 90, 180, 270] {
            for mirror in [false, true] {
                if angle == 0 && !mirror {
                    continue;
                }
                let t = Transform { angle, mirror };
                let out = rotate_gif(&src, t).unwrap();
                let got = image::load_from_memory_with_format(&out, ImageFormat::Gif).unwrap();
                assert_eq!(got.to_rgba8(), rotate_image(&original, t).to_rgba8(), "{t:?}");
            }
        }
    }

    #[test]
    fn png_mirror_only() {
        let src = encode(&sample(), ImageFormat::Png).unwrap();
        let out = rotate_png(&src, Transform { angle: 0, mirror: true }).unwrap();
        let got = image::load_from_memory_with_format(&out, ImageFormat::Png).unwrap();
        assert_eq!(got.to_rgb8(), sample().fliph().to_rgb8());
    }

    #[test]
    fn compose_and_inverse_match_image_operations() {
        let img = sample();
        let all: Vec<Transform> = [0, 90, 180, 270]
            .into_iter()
            .flat_map(|angle| [false, true].map(|mirror| Transform { angle, mirror }))
            .collect();
        for &t1 in &all {
            let inv = rotate_image(&rotate_image(&img, t1), t1.inverse());
            assert_eq!(inv.to_rgb8(), img.to_rgb8(), "inverse {t1:?}");
            for &t2 in &all {
                let want = rotate_image(&rotate_image(&img, t1), t2);
                assert_eq!(rotate_image(&img, t1.then(t2)).to_rgb8(), want.to_rgb8(), "{t1:?} then {t2:?}");
            }
        }
    }
}
