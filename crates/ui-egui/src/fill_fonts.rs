//! Installed face selection for Fill & Sign, loaded only on explicit selection.
use pdfcraft_fonts::installed::Face;
use std::sync::{Arc, Mutex, OnceLock};

#[derive(Clone, Debug, PartialEq)]
pub struct SelectedFont {
    pub face: Face,
    pub bytes: Arc<Vec<u8>>,
}
impl SelectedFont {
    pub fn load(face: Face) -> Result<Self, String> {
        let bytes = pdfcraft_fonts::installed::load(&face)?;
        pdfcraft_fonts::embedding::validate(&bytes)?;
        Ok(Self { face, bytes: Arc::new(bytes) })
    }
    pub fn preview(&self, ctx: &egui::Context) -> egui::FontFamily {
        let key = format!("fill-font-{}-{}", self.face.path.display(), self.face.index);
        let family = egui::FontFamily::Name(key.clone().into());
        if !ctx.fonts(|f| f.definitions().families.contains_key(&family)) {
            let mut fonts = ctx.fonts(|f| f.definitions().clone());
            fonts.font_data.retain(|name, _| !name.starts_with("fill-font-"));
            fonts.families.retain(|name, _| !matches!(name, egui::FontFamily::Name(n) if n.starts_with("fill-font-")));
            fonts.font_data.insert(key.clone(), Arc::new(egui::FontData::from_owned(self.bytes.as_ref().clone())));
            fonts.families.insert(family.clone(), vec![key]);
            ctx.set_fonts(fonts);
            return egui::FontFamily::Proportional;
        }
        family
    }
}

pub fn installed(ctx: &egui::Context) -> Option<Arc<Vec<Face>>> {
    type Cache = Arc<Mutex<Option<Arc<Vec<Face>>>>>;
    static CACHE: OnceLock<Cache> = OnceLock::new();
    let cache = CACHE.get_or_init(|| {
        let cache: Cache = Arc::new(Mutex::new(None));
        let result = cache.clone();
        let ctx = ctx.clone();
        if std::thread::Builder::new()
            .name("fill-fonts".into())
            .spawn(move || {
                let faces = pdfcraft_fonts::installed::discover();
                if let Ok(mut result) = result.lock() {
                    *result = Some(Arc::new(faces));
                }
                ctx.request_repaint();
            })
            .is_err()
            && let Ok(mut result) = cache.lock()
        {
            *result = Some(Arc::new(Vec::new()));
        }
        cache
    });
    cache.lock().ok().and_then(|c| c.clone())
}
