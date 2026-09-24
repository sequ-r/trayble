//! Pixel-level helpers for tray icons.
//!
//! StatusNotifierItem pixmaps are packed ARGB32 in *network* byte order: for
//! every pixel the bytes are alpha, red, green, blue, and rows are
//! `width * 4` bytes apart. This layout is deliberately kept end to end — the
//! shell passes it to `St.ImageContent` as `Cogl.PixelFormat.ARGB_8888`, and
//! the settings app passes it to `gdk::MemoryTexture` as
//! `gdk::MemoryFormat::A8r8g8b8`, both unchanged. Only the two places that
//! actually touch pixels ([`composite`], [`to_rgba`]) ever interpret them.

use serde::{Deserialize, Serialize};

/// Bytes per pixel of a packed ARGB32 image.
pub const BYTES_PER_PIXEL: usize = 4;

/// One raw icon image as supplied by a StatusNotifierItem.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, zvariant::Type)]
pub struct Pixmap {
    pub width: i32,
    pub height: i32,
    pub data: Vec<u8>,
}

impl Pixmap {
    pub fn new(width: i32, height: i32, data: Vec<u8>) -> Self {
        Self {
            width,
            height,
            data,
        }
    }

    /// A pixmap is usable when it has a size and exactly one packed pixel per
    /// slot. Applications do ship broken pixmaps now and then; every consumer
    /// checks this first so a malformed image degrades to "no icon".
    pub fn is_valid(&self) -> bool {
        self.width > 0
            && self.height > 0
            && self.data.len() == self.width as usize * self.height as usize * BYTES_PER_PIXEL
    }

    /// Distance in bytes between two rows. The wire format has no padding.
    pub fn row_stride(&self) -> usize {
        self.width as usize * BYTES_PER_PIXEL
    }

    /// Pixel at (`x`, `y`) as `[alpha, red, green, blue]`, or `None` when out
    /// of bounds.
    pub fn pixel(&self, x: i32, y: i32) -> Option<[u8; 4]> {
        if !self.is_valid() || x < 0 || y < 0 || x >= self.width || y >= self.height {
            return None;
        }
        let offset = (y as usize * self.row_stride()) + (x as usize * BYTES_PER_PIXEL);
        let p = &self.data[offset..offset + BYTES_PER_PIXEL];
        Some([p[0], p[1], p[2], p[3]])
    }

    pub fn set_pixel(&mut self, x: i32, y: i32, pixel: [u8; 4]) {
        if !self.is_valid() || x < 0 || y < 0 || x >= self.width || y >= self.height {
            return;
        }
        let offset = (y as usize * self.row_stride()) + (x as usize * BYTES_PER_PIXEL);
        self.data[offset..offset + BYTES_PER_PIXEL].copy_from_slice(&pixel);
    }

    /// Straight (non-premultiplied) RGBA, the format `GdkPixbuf` and most
    /// image encoders speak. Used by tests and for saving icons to disk.
    pub fn to_rgba(&self) -> Option<Vec<u8>> {
        if !self.is_valid() {
            return None;
        }
        Some(
            self.data
                .chunks_exact(BYTES_PER_PIXEL)
                .flat_map(|p| [p[1], p[2], p[3], p[0]])
                .collect(),
        )
    }

    /// Build a pixmap from straight RGBA data. The inverse of [`Pixmap::to_rgba`].
    pub fn from_rgba(width: i32, height: i32, rgba: &[u8]) -> Self {
        let data = rgba
            .chunks_exact(BYTES_PER_PIXEL)
            .flat_map(|p| [p[3], p[0], p[1], p[2]])
            .collect();
        Self::new(width, height, data)
    }
}

/// Encode a pixmap as a PNG image.
///
/// This is the only form icons leave the daemon in. Handing raw pixel buffers
/// to a compositor is a footgun — the caller has to agree on pixel format,
/// stride and byte order, and a mistake crashes the whole session — while a
/// PNG is a self-describing image that gdk-pixbuf and GDK both decode in
/// managed code.
pub fn to_png(pixmap: &Pixmap) -> Option<Vec<u8>> {
    let rgba = pixmap.to_rgba()?;
    let mut png = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png, pixmap.width as u32, pixmap.height as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().ok()?;
        writer.write_image_data(&rgba).ok()?;
    }
    Some(png)
}

/// Decode a PNG image back into a pixmap. Used by tests and by tools that
/// want to look at what the panel shows.
pub fn from_png(png: &[u8]) -> Option<Pixmap> {
    let decoder = png::Decoder::new(std::io::Cursor::new(png));
    let mut reader = decoder.read_info().ok()?;
    let mut rgba = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut rgba).ok()?;
    rgba.truncate(info.buffer_size());
    Some(Pixmap::from_rgba(
        info.width as i32,
        info.height as i32,
        &rgba,
    ))
}

/// Pick the pixmap that best fills `target` pixels: the smallest one that is
/// at least `target` wide, so upscaling is only done when nothing large
/// enough was provided. Invalid entries are ignored; ties are broken by
/// height. Falls back to the largest available image.
pub fn pick_best(pixmaps: &[Pixmap], target: u32) -> Option<&Pixmap> {
    let usable = || pixmaps.iter().filter(|p| p.is_valid());
    let target = target.min(i32::MAX as u32) as i32;

    usable()
        .filter(|p| p.width >= target && p.height >= target)
        .min_by_key(|p| (p.width, p.height))
        .or_else(|| usable().max_by_key(|p| (p.width, p.height)))
}

/// Nearest-neighbour rescale. Icons are tiny and usually only ever scaled by
/// an integer factor, so this is both enough and cheap enough to run on every
/// update.
pub fn scale_nearest(pixmap: &Pixmap, width: i32, height: i32) -> Pixmap {
    if width <= 0 || height <= 0 {
        return Pixmap::default();
    }
    if !pixmap.is_valid() {
        return Pixmap::new(width, height, vec![0; width as usize * height as usize * BYTES_PER_PIXEL]);
    }
    if pixmap.width == width && pixmap.height == height {
        return pixmap.clone();
    }

    let mut out = Pixmap::new(
        width,
        height,
        vec![0; width as usize * height as usize * BYTES_PER_PIXEL],
    );
    for y in 0..height {
        let src_y = (y as i64 * pixmap.height as i64 / height as i64) as i32;
        for x in 0..width {
            let src_x = (x as i64 * pixmap.width as i64 / width as i64) as i32;
            if let Some(pixel) = pixmap.pixel(src_x, src_y) {
                out.set_pixel(x, y, pixel);
            }
        }
    }
    out
}

/// Draw `overlay` on top of `base`, centred when it is the smaller of the two
/// and rescaled when it is bigger. Used for the overlay icons of the
/// StatusNotifierItem spec (unread counters, status badges).
pub fn composite(base: &Pixmap, overlay: &Pixmap) -> Pixmap {
    if !base.is_valid() {
        return overlay.clone();
    }
    if !overlay.is_valid() {
        return base.clone();
    }

    let (fitted, offset_x, offset_y) = if overlay.width <= base.width && overlay.height <= base.height
    {
        (
            overlay.clone(),
            (base.width - overlay.width) / 2,
            (base.height - overlay.height) / 2,
        )
    } else {
        (scale_nearest(overlay, base.width, base.height), 0, 0)
    };

    let mut out = base.clone();
    for y in 0..fitted.height {
        for x in 0..fitted.width {
            let Some(src) = fitted.pixel(x, y) else { continue };
            let (dx, dy) = (x + offset_x, y + offset_y);
            let mut dst = out.pixel(dx, dy).unwrap_or([0, 0, 0, 0]);
            blend_over(&mut dst, src);
            out.set_pixel(dx, dy, dst);
        }
    }
    out
}

/// Alpha-blend `src` over `dst` in place. Both use straight (non-premultiplied)
/// ARGB, which is why the result alpha has to be divided back out.
pub fn blend_over(dst: &mut [u8; 4], src: [u8; 4]) {
    let src_a = src[0] as f32 / 255.0;
    if src_a <= 0.0 {
        return;
    }
    let dst_a = dst[0] as f32 / 255.0;
    let out_a = src_a + dst_a * (1.0 - src_a);
    if out_a <= 0.0 {
        *dst = [0, 0, 0, 0];
        return;
    }
    for channel in 1..4 {
        let blended = (src[channel] as f32 * src_a
            + dst[channel] as f32 * dst_a * (1.0 - src_a))
            / out_a;
        dst[channel] = blended.round().clamp(0.0, 255.0) as u8;
    }
    dst[0] = (out_a * 255.0).round().clamp(0.0, 255.0) as u8;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(width: i32, height: i32, pixel: [u8; 4]) -> Pixmap {
        let mut p = Pixmap::new(width, height, vec![0; (width * height) as usize * 4]);
        for y in 0..height {
            for x in 0..width {
                p.set_pixel(x, y, pixel);
            }
        }
        p
    }

    #[test]
    fn valid_pixmap_round_trips_rgba() {
        let p = solid(2, 2, [255, 1, 2, 3]);
        assert!(p.is_valid());
        assert_eq!(p.row_stride(), 8);
        assert_eq!(p.pixel(1, 1), Some([255, 1, 2, 3]));
        let rgba: Vec<u8> = (0..4).flat_map(|_| [1, 2, 3, 255]).collect();
        assert_eq!(p.to_rgba(), Some(rgba));
        assert_eq!(Pixmap::from_rgba(2, 2, &p.to_rgba().unwrap()), p);
    }

    #[test]
    fn malformed_pixmap_is_rejected() {
        assert!(!Pixmap::new(2, 2, vec![0; 15]).is_valid());
        assert!(!Pixmap::new(0, 2, vec![]).is_valid());
        assert!(!Pixmap::default().is_valid());
    }

    #[test]
    fn out_of_bounds_access_is_none() {
        let p = solid(1, 1, [255, 0, 0, 0]);
        assert_eq!(p.pixel(1, 0), None);
        assert_eq!(p.pixel(0, -1), None);
    }

    #[test]
    fn pick_best_prefers_smallest_sufficient_pixmap() {
        let small = solid(16, 16, [255, 0, 0, 0]);
        let good = solid(24, 24, [255, 0, 0, 0]);
        let huge = solid(64, 64, [255, 0, 0, 0]);
        let pixmaps = vec![small.clone(), good.clone(), huge.clone()];

        assert_eq!(pick_best(&pixmaps, 22), Some(&good));
        assert_eq!(pick_best(&pixmaps, 8), Some(&small));
    }

    #[test]
    fn pick_best_falls_back_to_largest() {
        let small = solid(16, 16, [255, 0, 0, 0]);
        let pixmaps = vec![small.clone(), Pixmap::new(8, 8, vec![])];
        assert_eq!(pick_best(&pixmaps, 48), Some(&small));
        assert_eq!(pick_best(&[], 22), None);
    }

    #[test]
    fn scale_nearest_keeps_colour_and_size() {
        let p = solid(2, 2, [255, 9, 8, 7]);
        let scaled = scale_nearest(&p, 4, 4);
        assert!(scaled.is_valid());
        assert_eq!((scaled.width, scaled.height), (4, 4));
        assert_eq!(scaled.pixel(3, 3), Some([255, 9, 8, 7]));
    }

    #[test]
    fn composite_blends_overlay_over_base() {
        let base = solid(4, 4, [255, 0, 0, 0]);
        let overlay = solid(4, 4, [255, 255, 255, 255]);
        let merged = composite(&base, &overlay);
        assert_eq!(merged.pixel(2, 2), Some([255, 255, 255, 255]));
    }

    #[test]
    fn composite_centres_small_overlay() {
        let base = solid(4, 4, [255, 0, 0, 0]);
        let overlay = solid(2, 2, [255, 255, 255, 255]);
        let merged = composite(&base, &overlay);
        assert_eq!(merged.pixel(0, 0), Some([255, 0, 0, 0]), "corner stays base");
        assert_eq!(merged.pixel(1, 1), Some([255, 255, 255, 255]));
        assert_eq!(merged.pixel(2, 2), Some([255, 255, 255, 255]));
        assert_eq!(merged.pixel(3, 3), Some([255, 0, 0, 0]));
    }

    #[test]
    fn png_round_trip_keeps_the_picture() {
        let p = solid(3, 2, [255, 10, 20, 30]);
        let png = to_png(&p).expect("encodes");
        assert!(png.starts_with(&[0x89, b'P', b'N', b'G']), "is a PNG: {png:?}");

        let back = from_png(&png).expect("decodes");
        assert_eq!((back.width, back.height), (3, 2));
        assert_eq!(back.pixel(2, 1), Some([255, 10, 20, 30]));
    }

    #[test]
    fn broken_images_encode_to_nothing() {
        assert!(to_png(&Pixmap::new(2, 2, vec![0; 3])).is_none());
        assert!(from_png(&[1, 2, 3]).is_none());
    }

    #[test]
    fn blend_over_uses_straight_alpha() {
        let mut dst = [255, 0, 0, 0];
        blend_over(&mut dst, [128, 255, 255, 255]);
        assert_eq!(dst[0], 255, "opaque destination stays opaque");
        assert!(dst[1] > 100 && dst[1] < 155, "half transparent white: {dst:?}");

        let mut clear = [0, 0, 0, 0];
        blend_over(&mut clear, [0, 1, 2, 3]);
        assert_eq!(clear, [0, 0, 0, 0], "fully transparent source is a no-op");
    }
}
