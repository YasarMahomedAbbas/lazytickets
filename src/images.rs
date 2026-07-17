//! Inline-image state for the detail pane. Owns the terminal graphics `Picker`
//! and a URL-keyed cache of decoded images and their encoded protocols, so an
//! image is fetched and encoded once and re-rendered cheaply on every frame.

use image::DynamicImage;
use ratatui::layout::Size;
use ratatui_image::Resize;
use ratatui_image::picker::Picker;
use ratatui_image::protocol::Protocol;
use std::collections::HashMap;

/// A single attachment's lifecycle.
pub enum ImageEntry {
    /// Fetch/decode in flight.
    Loading,
    /// Fetch or decode failed; the reason is shown in place of the image.
    Failed(String),
    /// Decoded and ready. `proto` is the protocol encoded for the current
    /// `cols`×`rows` box, rebuilt lazily when the pane width changes.
    Ready {
        img: DynamicImage,
        proto: Option<Protocol>,
        cols: u16,
        rows: u16,
    },
}

/// The picker plus the per-URL cache. The cache persists across selections, so
/// revisiting a ticket re-shows its images instantly.
pub struct Images {
    pub picker: Option<Picker>,
    pub cache: HashMap<String, ImageEntry>,
}

impl Images {
    pub fn new(picker: Option<Picker>) -> Self {
        Self {
            picker,
            cache: HashMap::new(),
        }
    }

    /// Whether graphics are usable at all (a picker was created).
    pub fn enabled(&self) -> bool {
        self.picker.is_some()
    }

    /// Cell height an image of `img_w`×`img_h` pixels should occupy when drawn
    /// `cols` cells wide, preserving aspect and capped to `max_rows`. Uses the
    /// terminal's real font size when known, else assumes 1:2 cells.
    pub fn rows_for(&self, img_w: u32, img_h: u32, cols: u16, max_rows: u16) -> u16 {
        if img_w == 0 || img_h == 0 || cols == 0 {
            return 1;
        }
        let (fw, fh) = self
            .picker
            .as_ref()
            .map(|p| (p.font_size().width as u64, p.font_size().height as u64))
            .unwrap_or((1, 2));
        // rows = cols * (font_w / font_h) * (img_h / img_w)
        let rows = (cols as u64 * fw * img_h as u64) / (fh * img_w as u64);
        (rows.max(1) as u16).min(max_rows.max(1))
    }

    /// The protocol to render `url` in a `cols`×`rows` box, encoding (or
    /// re-encoding after a resize) on demand. `None` until the image is ready or
    /// if there's no picker.
    pub fn protocol_for(&mut self, url: &str, cols: u16, rows: u16) -> Option<&Protocol> {
        let Images { picker, cache } = self;
        let picker = picker.as_ref()?;
        let entry = cache.get_mut(url)?;
        let ImageEntry::Ready {
            img,
            proto,
            cols: pc,
            rows: pr,
        } = entry
        else {
            return None;
        };
        if proto.is_none() || *pc != cols || *pr != rows {
            match picker.new_protocol(img.clone(), Size::new(cols, rows), Resize::Fit(None)) {
                Ok(p) => {
                    *proto = Some(p);
                    *pc = cols;
                    *pr = rows;
                }
                Err(_) => {
                    *proto = None;
                    return None;
                }
            }
        }
        proto.as_ref()
    }
}
