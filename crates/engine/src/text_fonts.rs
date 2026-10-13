//! Explicit document-font discovery. Interface fallbacks never enter this catalogue.
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct FontFile {
    pub path: PathBuf,
    pub family: String,
    pub face: String,
}

/// Standard user-installed font folders; discovery is performed only on request.
pub fn installed_directories() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if cfg!(target_os = "macos") {
        let mut dirs = vec![PathBuf::from("/Library/Fonts")];
        if let Some(home) = home {
            dirs.push(home.join("Library/Fonts"));
        }
        dirs
    } else if cfg!(windows) {
        let mut dirs = std::env::var_os("WINDIR").map(|p| PathBuf::from(p).join("Fonts")).into_iter().collect::<Vec<_>>();
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            dirs.push(PathBuf::from(local).join("Microsoft/Windows/Fonts"));
        }
        dirs
    } else {
        let mut dirs = vec![PathBuf::from("/usr/share/fonts"), PathBuf::from("/usr/local/share/fonts")];
        if let Some(home) = home {
            dirs.extend([home.join(".local/share/fonts"), home.join(".fonts")]);
        }
        dirs
    }
}

/// Bounded, symlink-free scan. Only standalone, openly licensed, embeddable faces are listed.
/// The caller controls roots (automation resolves confinement before calling this).
pub fn discover(roots: &[PathBuf]) -> Vec<FontFile> {
    use std::io::Read;
    let mut pending: Vec<_> = roots.iter().take(16).map(|r| (r.clone(), 0usize)).collect();
    let mut files = Vec::new();
    let mut entries = 0usize;
    let mut budget = 64u64 << 20;
    let mut seen = std::collections::HashSet::new();
    while let Some((path, depth)) = pending.pop() {
        if entries >= 4096 || budget == 0 {
            break;
        }
        entries += 1;
        let Ok(meta) = std::fs::symlink_metadata(&path) else { continue };
        if meta.file_type().is_symlink() || !seen.insert(path.clone()) {
            continue;
        }
        if meta.is_dir() && depth < 4 {
            if let Ok(children) = std::fs::read_dir(&path) {
                let remaining = 4096usize.saturating_sub(entries.saturating_add(pending.len()));
                pending.extend(children.take(remaining).filter_map(Result::ok).map(|p| (p.path(), depth + 1)));
            }
        } else if meta.is_file() && meta.len() <= (16 << 20) && font_extension(&path) {
            if meta.len() > budget {
                continue;
            }
            budget = budget.saturating_sub(meta.len());
            let mut bytes = Vec::new();
            let read = std::fs::File::open(&path).and_then(|f| f.take((16 << 20) + 1).read_to_end(&mut bytes));
            if read.is_err() || pdfcraft_fonts::embedding::validate_document_input(&bytes).is_err() {
                continue;
            }
            if let Ok((family, face)) = pdfcraft_fonts::embedding::names(&bytes) {
                files.push(FontFile { path, family, face });
            }
        }
    }
    files.sort_by(|a, b| a.face.cmp(&b.face).then(a.path.cmp(&b.path)));
    files
}
fn font_extension(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("ttf") || e.eq_ignore_ascii_case("otf"))
}

/// Load a face explicitly selected for a document, including a face in a collection.
/// Unlike the open-asset catalogue, this uses the selected font's embedding permission flags.
pub fn selected_program(path: &Path, index: u32) -> Result<Vec<u8>, String> {
    let bytes = pdfcraft_fonts::installed::load(&pdfcraft_fonts::installed::Face { path: path.into(), index, name: String::new() })?;
    pdfcraft_fonts::embedding::validate(&bytes)?;
    Ok(bytes)
}

/// Size a selected-font text annotation with the same shaping and wrapping as its appearance.
pub fn selected_text_rect(bytes: &[u8], text: &str, size: f64, at: [f64; 2]) -> Result<[f64; 4], String> {
    let mut longest = 0.0f64;
    for line in text.lines() {
        longest = longest.max(pdfcraft_fonts::source_shaping::shape(bytes, line)?.advance * size);
    }
    let width = (longest + 8.0).clamp(40.0, 600.0);
    let lines = pdfcraft_fonts::wrap_with(text, size, width - 4.0, |line, size| {
        pdfcraft_fonts::source_shaping::shape(bytes, line).map_or(f64::INFINITY, |r| r.advance * size)
    });
    let height = lines.len().max(1) as f64 * size * 1.2 + 6.0;
    Ok([at[0], at[1] - height, at[0] + width, at[1]])
}
