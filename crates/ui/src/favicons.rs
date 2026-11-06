//! Текстуры значков сайтов для вкладок. Значок загружается в GPU один раз и обновляется,
//! когда у вкладки появляется другой `Arc<Image>`.

use std::collections::HashMap;
use std::sync::Arc;

use egui::{ColorImage, TextureFilter, TextureHandle, TextureId, TextureOptions, TextureWrapMode};
use klin_core::browser::Tab;
use klin_shared::{Image, TabId};

#[derive(Default)]
pub(crate) struct Favicons {
    cache: HashMap<TabId, (Arc<Image>, TextureHandle)>,
}

/// Значки бывают и 16×16, и 256×256: mip-уровни дают чистое уменьшение до размера вкладки.
const OPTIONS: TextureOptions = TextureOptions {
    magnification: TextureFilter::Linear,
    minification: TextureFilter::Linear,
    wrap_mode: TextureWrapMode::ClampToEdge,
    mipmap_mode: Some(TextureFilter::Linear),
};

impl Favicons {
    pub fn texture(&mut self, ctx: &egui::Context, tab: &Tab) -> Option<TextureId> {
        let Some(icon) = &tab.favicon else {
            self.cache.remove(&tab.id);
            return None;
        };
        if let Some((cached, handle)) = self.cache.get(&tab.id)
            && Arc::ptr_eq(cached, icon)
        {
            return Some(handle.id());
        }
        let size = [icon.width as usize, icon.height as usize];
        if icon.rgba.len() != size[0] * size[1] * 4 || size[0] == 0 || size[1] == 0 {
            return None;
        }
        let image = ColorImage::from_rgba_unmultiplied(size, &icon.rgba);
        let handle = ctx.load_texture(format!("favicon-{}", tab.id), image, OPTIONS);
        let id = handle.id();
        self.cache.insert(tab.id, (icon.clone(), handle));
        Some(id)
    }

    /// Освободить текстуры закрытых вкладок.
    pub fn retain(&mut self, live: impl Fn(TabId) -> bool) {
        self.cache.retain(|tab, _| live(*tab));
    }
}
