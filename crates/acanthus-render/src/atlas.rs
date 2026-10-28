use std::collections::HashMap;

use cosmic_text::{CacheKey, FontSystem, SwashCache, SwashContent};

pub const PAGE_SIZE: u32 = 1024;
const MAX_MASK_PAGES: usize = 8;
const MAX_COLOR_PAGES: usize = 2;
const PADDING: u32 = 1;

#[derive(Debug, Clone, Copy)]
pub struct AtlasGlyph {
    pub page: usize,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub left: i32,
    pub top: i32,
    pub color: bool,
}

struct Shelf {
    y: u32,
    height: u32,
    cursor: u32,
}

#[derive(Default)]
struct ShelfPacker {
    shelves: Vec<Shelf>,
    next_y: u32,
}

impl ShelfPacker {
    fn alloc(&mut self, w: u32, h: u32) -> Option<(u32, u32)> {
        let (w, h) = (w + PADDING, h + PADDING);
        if w > PAGE_SIZE {
            return None;
        }
        if let Some(shelf) = self
            .shelves
            .iter_mut()
            .filter(|s| s.height >= h && s.height <= h + h / 2 + 2 && s.cursor + w <= PAGE_SIZE)
            .min_by_key(|s| s.height)
        {
            let pos = (shelf.cursor, shelf.y);
            shelf.cursor += w;
            return Some(pos);
        }
        if self.next_y + h > PAGE_SIZE {
            return None;
        }
        let y = self.next_y;
        self.next_y += h;
        self.shelves.push(Shelf { y, height: h, cursor: w });
        Some((0, y))
    }
}

pub enum Lookup {
    Hit(AtlasGlyph),
    Empty,
    Full,
}

pub struct Page {
    pub bind_group: wgpu::BindGroup,
    texture: wgpu::Texture,
    color: bool,
    packer: ShelfPacker,
    used: u64,
}

pub struct GlyphAtlas {
    layout: wgpu::BindGroupLayout,
    pages: Vec<Page>,
    glyphs: HashMap<CacheKey, Option<AtlasGlyph>>,
    swash: SwashCache,
    scale: swash::scale::ScaleContext,
    generation: u64,
    frame: u64,
}

impl GlyphAtlas {
    pub fn new(layout: &wgpu::BindGroupLayout) -> Self {
        GlyphAtlas {
            layout: layout.clone(),
            pages: Vec::new(),
            glyphs: HashMap::new(),
            swash: SwashCache::new(),
            scale: swash::scale::ScaleContext::new(),
            generation: 0,
            frame: 1,
        }
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn page(&self, index: usize) -> &Page {
        &self.pages[index]
    }

    pub fn begin_frame(&mut self) {
        self.frame += 1;
    }

    pub fn clear(&mut self) {
        self.generation += 1;
        self.pages.clear();
        self.glyphs.clear();
    }

    fn new_page(&self, device: &wgpu::Device, color: bool) -> Page {
        let format = if color { wgpu::TextureFormat::Rgba8Unorm } else { wgpu::TextureFormat::R8Unorm };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("serp glyph page"),
            size: wgpu::Extent3d { width: PAGE_SIZE, height: PAGE_SIZE, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("serp glyph page"),
            layout: &self.layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) }],
        });
        Page { bind_group, texture, color, packer: ShelfPacker::default(), used: 0 }
    }

    fn place(&mut self, device: &wgpu::Device, color: bool, w: u32, h: u32) -> Option<(usize, u32, u32)> {
        let frame = self.frame;
        for (index, page) in self.pages.iter_mut().enumerate().filter(|(_, p)| p.color == color) {
            if let Some((x, y)) = page.packer.alloc(w, h) {
                page.used = frame;
                return Some((index, x, y));
            }
        }
        let limit = if color { MAX_COLOR_PAGES } else { MAX_MASK_PAGES };
        let count = self.pages.iter().filter(|p| p.color == color).count();
        if count < limit {
            let mut page = self.new_page(device, color);
            let (x, y) = page.packer.alloc(w, h)?;
            page.used = frame;
            self.pages.push(page);
            return Some((self.pages.len() - 1, x, y));
        }
        let victim = self
            .pages
            .iter()
            .enumerate()
            .filter(|(_, p)| p.color == color && p.used < frame)
            .min_by_key(|(_, p)| p.used)
            .map(|(i, _)| i)?;
        self.generation += 1;
        self.glyphs.retain(|_, g| g.is_none_or(|g| g.page != victim));
        let page = &mut self.pages[victim];
        page.packer = ShelfPacker::default();
        let (x, y) = page.packer.alloc(w, h)?;
        page.used = frame;
        Some((victim, x, y))
    }

    pub fn get(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, fonts: &mut FontSystem, key: CacheKey) -> Lookup {
        if let Some(entry) = self.glyphs.get(&key).copied() {
            if let Some(glyph) = entry {
                self.pages[glyph.page].used = self.frame;
            }
            return entry.map_or(Lookup::Empty, Lookup::Hit);
        }
        let image = match acanthus_scene::axes_lookup(key.flags) {
            Some(axes) => render_with_axes(&mut self.scale, fonts, key, &axes),
            None => self.swash.get_image_uncached(fonts, key),
        };
        let Some(image) = image else {
            self.glyphs.insert(key, None);
            return Lookup::Empty;
        };
        let (w, h) = (image.placement.width, image.placement.height);
        if w == 0 || h == 0 || w + PADDING > PAGE_SIZE || h + PADDING > PAGE_SIZE {
            self.glyphs.insert(key, None);
            return Lookup::Empty;
        }
        let color = matches!(image.content, SwashContent::Color);
        let Some((page, x, y)) = self.place(device, color, w, h) else { return Lookup::Full };

        let pixels = (w * h) as usize;
        let (data, bytes_per_pixel) = match image.content {
            SwashContent::Mask => (image.data, 1),
            SwashContent::SubpixelMask => (
                image
                    .data
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .take(pixels)
                    .map(|c| ((c[0] as u16 + c[1] as u16 + c[2] as u16) / 3) as u8)
                    .collect(),
                1,
            ),
            SwashContent::Color => (image.data, 4),
        };

        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.pages[page].texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x, y, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            &data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w * bytes_per_pixel),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );

        let glyph =
            AtlasGlyph { page, x, y, width: w, height: h, left: image.placement.left, top: image.placement.top, color };
        self.glyphs.insert(key, Some(glyph));
        Lookup::Hit(glyph)
    }
}

fn render_with_axes(
    context: &mut swash::scale::ScaleContext,
    fonts: &mut FontSystem,
    key: CacheKey,
    axes: &[acanthus_scene::Axis],
) -> Option<cosmic_text::SwashImage> {
    use swash::scale::{Render, Source, StrikeWith};
    use swash::zeno::{Angle, Format, Transform, Vector};

    let font = fonts.get_font(key.font_id, key.font_weight)?;
    let swash_font = font.as_swash();
    let wght = swash::Tag::from_be_bytes(*b"wght");
    let weight = f32::from(key.font_weight.0);
    let settings = std::iter::once((wght, weight))
        .filter(|_| !axes.iter().any(|(tag, _)| swash::Tag::from_be_bytes(*tag) == wght))
        .chain(axes.iter().map(|(tag, value)| (swash::Tag::from_be_bytes(*tag), *value)));
    let coords = swash_font.variations().normalized_coords(settings);
    let mut scaler = context
        .builder(swash_font)
        .size(f32::from_bits(key.font_size_bits))
        .hint(!key.flags.contains(cosmic_text::CacheKeyFlags::DISABLE_HINTING))
        .normalized_coords(coords)
        .build();
    let offset = Vector::new(key.x_bin.as_float(), key.y_bin.as_float());
    let skew = key
        .flags
        .contains(cosmic_text::CacheKeyFlags::FAKE_ITALIC)
        .then(|| Transform::skew(Angle::from_degrees(14.0), Angle::from_degrees(0.0)));
    Render::new(&[Source::ColorOutline(0), Source::ColorBitmap(StrikeWith::BestFit), Source::Outline])
        .format(Format::Alpha)
        .offset(offset)
        .transform(skew)
        .render(&mut scaler, key.glyph_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packer_fills_rows_then_reports_full() {
        let mut p = ShelfPacker::default();
        assert_eq!(p.alloc(10, 10), Some((0, 0)));
        assert_eq!(p.alloc(10, 10), Some((11, 0)));
        assert_eq!(p.alloc(10, 30), Some((0, 11)));
        let mut count = 0;
        while p.alloc(100, 100).is_some() {
            count += 1;
        }
        assert!(count > 75);
        assert_eq!(p.alloc(PAGE_SIZE + 1, 1), None);
    }

    #[test]
    fn variation_axes_change_the_rasterised_glyph() {
        use cosmic_text::{Attrs, Buffer, Family, Metrics, Shaping, Weight};

        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/fonts/GoogleSansFlex.ttf");
        let Ok(data) = std::fs::read(path) else { return };
        let mut fonts = FontSystem::new_with_fonts([cosmic_text::fontdb::Source::Binary(std::sync::Arc::new(data))]);
        let mut buffer = Buffer::new(&mut fonts, Metrics::new(40.0, 48.0));
        let attrs = Attrs::new().family(Family::Name("Google Sans Flex")).weight(Weight(700));
        buffer.set_text("a1", &attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut fonts, false);
        let glyph = buffer.layout_runs().next().unwrap().glyphs[0].physical((0.0, 0.0), 1.0);
        let mut context = swash::scale::ScaleContext::new();
        let mut render = |value: f32| {
            let axes = [(*b"ROND", value)];
            render_with_axes(&mut context, &mut fonts, glyph.cache_key, &axes).unwrap().data
        };
        assert_ne!(render(0.0), render(100.0));
    }
}
