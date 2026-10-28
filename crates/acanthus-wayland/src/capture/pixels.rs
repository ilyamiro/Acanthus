use wayland_client::protocol::wl_shm::Format;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PixelFormat {
    Xrgb8888,
    Argb8888,
    Xbgr8888,
    Abgr8888,
}

impl PixelFormat {
    pub const PREFERRED: [PixelFormat; 4] =
        [PixelFormat::Xrgb8888, PixelFormat::Argb8888, PixelFormat::Xbgr8888, PixelFormat::Abgr8888];

    pub fn from_wl(format: Format) -> Option<PixelFormat> {
        match format {
            Format::Xrgb8888 => Some(PixelFormat::Xrgb8888),
            Format::Argb8888 => Some(PixelFormat::Argb8888),
            Format::Xbgr8888 => Some(PixelFormat::Xbgr8888),
            Format::Abgr8888 => Some(PixelFormat::Abgr8888),
            _ => None,
        }
    }

    pub fn to_wl(self) -> Format {
        match self {
            PixelFormat::Xrgb8888 => Format::Xrgb8888,
            PixelFormat::Argb8888 => Format::Argb8888,
            PixelFormat::Xbgr8888 => Format::Xbgr8888,
            PixelFormat::Abgr8888 => Format::Abgr8888,
        }
    }

    fn red_first(self) -> bool {
        matches!(self, PixelFormat::Xbgr8888 | PixelFormat::Abgr8888)
    }
}

pub(crate) fn pick(offered: &[Format]) -> Option<PixelFormat> {
    let known: Vec<PixelFormat> = offered.iter().filter_map(|f| PixelFormat::from_wl(*f)).collect();
    PixelFormat::PREFERRED.into_iter().find(|wanted| known.contains(wanted))
}

pub(crate) fn to_rgba(
    format: PixelFormat,
    width: u32,
    height: u32,
    stride: u32,
    data: &[u8],
    y_invert: bool,
) -> Option<Vec<u8>> {
    let (w, h, stride) = (width as usize, height as usize, stride as usize);
    if w == 0 || h == 0 || stride < w * 4 || data.len() < stride * (h - 1) + w * 4 {
        return None;
    }
    let mut out = Vec::with_capacity(w * h * 4);
    for row in 0..h {
        let source = if y_invert { h - 1 - row } else { row };
        let line = &data[source * stride..source * stride + w * 4];
        let (pixels, _) = line.as_chunks::<4>();
        for px in pixels {
            let (r, g, b) = if format.red_first() { (px[0], px[1], px[2]) } else { (px[2], px[1], px[0]) };
            out.extend_from_slice(&[r, g, b, 255]);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn little_endian_argb_bytes_become_straight_opaque_rgba() {
        let data = [10, 20, 30, 0, 40, 50, 60, 7];
        let out = to_rgba(PixelFormat::Argb8888, 2, 1, 8, &data, false).unwrap();
        assert_eq!(out, [30, 20, 10, 255, 60, 50, 40, 255]);
    }

    #[test]
    fn abgr_keeps_the_byte_order_and_forces_alpha() {
        let data = [1, 2, 3, 0];
        assert_eq!(to_rgba(PixelFormat::Xbgr8888, 1, 1, 4, &data, false).unwrap(), [1, 2, 3, 255]);
    }

    #[test]
    fn stride_padding_is_skipped_and_rows_can_be_flipped() {
        let data = [1, 1, 1, 0, 9, 9, 9, 9, 2, 2, 2, 0, 9, 9, 9, 9];
        let upright = to_rgba(PixelFormat::Xrgb8888, 1, 2, 8, &data, false).unwrap();
        assert_eq!(upright, [1, 1, 1, 255, 2, 2, 2, 255]);
        let flipped = to_rgba(PixelFormat::Xrgb8888, 1, 2, 8, &data, true).unwrap();
        assert_eq!(flipped, [2, 2, 2, 255, 1, 1, 1, 255]);
    }

    #[test]
    fn short_or_empty_buffers_are_rejected() {
        assert!(to_rgba(PixelFormat::Xrgb8888, 2, 2, 8, &[0; 15], false).is_none());
        assert!(to_rgba(PixelFormat::Xrgb8888, 0, 2, 8, &[0; 16], false).is_none());
        assert!(to_rgba(PixelFormat::Xrgb8888, 2, 1, 4, &[0; 8], false).is_none());
    }

    #[test]
    fn the_preferred_supported_format_wins() {
        assert_eq!(pick(&[Format::Abgr8888, Format::Argb8888, Format::Rgb565]), Some(PixelFormat::Argb8888));
        assert_eq!(pick(&[Format::Rgb565]), None);
        assert_eq!(pick(&[]), None);
    }
}
