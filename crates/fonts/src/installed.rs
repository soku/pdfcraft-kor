//! Explicitly selected installed fonts. No system font is bundled with the application.
use skrifa::{FontRef, MetadataProvider};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub struct Face {
    pub path: PathBuf,
    pub index: u32,
    pub name: String,
}

pub fn directories() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if cfg!(target_os = "macos") {
        roots.extend(["/System/Library/Fonts", "/Library/Fonts"].map(PathBuf::from));
        if let Some(home) = std::env::var_os("HOME") {
            roots.push(PathBuf::from(home).join("Library/Fonts"));
        }
    } else if cfg!(windows) {
        if let Some(w) = std::env::var_os("WINDIR") {
            roots.push(PathBuf::from(w).join("Fonts"));
        }
        if let Some(w) = std::env::var_os("LOCALAPPDATA") {
            roots.push(PathBuf::from(w).join("Microsoft/Windows/Fonts"));
        }
    } else {
        roots.extend(["/usr/share/fonts", "/usr/local/share/fonts"].map(PathBuf::from));
        if let Some(home) = std::env::var_os("HOME") {
            let home = PathBuf::from(home);
            roots.extend([home.join(".fonts"), home.join(".local/share/fonts")]);
        }
    }
    roots
}

fn read(path: &Path) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path).and_then(|f| f.take((64 << 20) + 1).read_to_end(&mut bytes)).map_err(|e| e.to_string())?;
    if bytes.len() > 64 << 20 {
        return Err("font exceeds 64 MiB".into());
    }
    Ok(bytes)
}

pub fn discover() -> Vec<Face> {
    let mut pending: Vec<_> = directories().into_iter().map(|p| (p, 0)).collect();
    let mut faces = Vec::new();
    let mut visited = std::collections::HashSet::new();
    for _ in 0..32768 {
        if faces.len() >= 16384 {
            break;
        }
        let Some((path, depth)) = pending.pop() else { break };
        let Ok(canonical) = path.canonicalize() else { continue };
        if !visited.insert(canonical) {
            continue;
        }
        if path.is_dir() {
            if depth < 8
                && let Ok(entries) = std::fs::read_dir(path)
            {
                pending.extend(
                    entries.take(32768usize.saturating_sub(visited.len() + pending.len())).filter_map(Result::ok).map(|e| (e.path(), depth + 1)),
                );
            }
            continue;
        }
        if !path.extension().and_then(|e| e.to_str()).is_some_and(|e| ["ttf", "otf", "ttc", "otc"].iter().any(|x| e.eq_ignore_ascii_case(x))) {
            continue;
        }
        let Ok(bytes) = read(&path) else { continue };
        for index in 0..256 {
            let Ok(font) = FontRef::from_index(&bytes, index) else { break };
            let name = font
                .localized_strings(skrifa::string::StringId::FULL_NAME)
                .find(|s| s.language() == Some("en"))
                .or_else(|| font.localized_strings(skrifa::string::StringId::FULL_NAME).next())
                .map(|s| s.to_string());
            if let Some(name) = name.filter(|n| !n.is_empty() && n.len() < 512) {
                faces.push(Face { path: path.clone(), index, name });
            }
        }
    }
    faces.sort_by_key(|a| a.name.to_lowercase());
    faces.dedup_by(|a, b| a.name == b.name);
    faces
}

/// Extract a TTC face into a standalone sfnt, retaining tables and recomputing checksums.
pub fn load(face: &Face) -> Result<Vec<u8>, String> {
    let bytes = read(&face.path)?;
    standalone(&bytes, face.index)
}

pub fn standalone(bytes: &[u8], index: u32) -> Result<Vec<u8>, String> {
    let font = FontRef::from_index(bytes, index).map_err(|e| e.to_string())?;
    // A TrueType variable font's base outlines/metrics are its default instance.
    // Drop variation tables when exporting that instance; CFF2-only programs remain unsupported.
    let records: Vec<_> = font
        .table_directory
        .table_records()
        .iter()
        .filter(|r| !matches!(&r.tag().to_be_bytes(), b"fvar" | b"gvar" | b"avar" | b"HVAR" | b"VVAR" | b"MVAR" | b"DSIG"))
        .collect();
    if records.is_empty() || records.len() > 256 {
        return Err("invalid font table count".into());
    }
    let count = records.len();
    let mut result = vec![0u8; 12 + count * 16];
    result[..4].copy_from_slice(&font.table_directory.sfnt_version().to_be_bytes());
    result[4..6].copy_from_slice(&(count as u16).to_be_bytes());
    let power = 1usize << count.ilog2();
    result[6..8].copy_from_slice(&((power * 16) as u16).to_be_bytes());
    result[8..10].copy_from_slice(&(count.ilog2() as u16).to_be_bytes());
    result[10..12].copy_from_slice(&(((count - power) * 16) as u16).to_be_bytes());
    let mut head = None;
    for (i, record) in records.iter().enumerate() {
        let tag = record.tag();
        let data = font.table_data(tag).ok_or("missing font table")?;
        let mut data = data.as_bytes().to_vec();
        if tag.to_be_bytes() == *b"head" && data.len() >= 12 {
            data[8..12].fill(0);
            head = Some(result.len() + 8);
        }
        let len = data.len();
        if result.len().saturating_add(len) > 64 << 20 {
            return Err("font exceeds 64 MiB".into());
        }
        while data.len() % 4 != 0 {
            data.push(0);
        }
        let row = 12 + i * 16;
        result[row..row + 4].copy_from_slice(&tag.to_be_bytes());
        result[row + 4..row + 8].copy_from_slice(&checksum(&data).to_be_bytes());
        let offset = result.len() as u32;
        result[row + 8..row + 12].copy_from_slice(&offset.to_be_bytes());
        result[row + 12..row + 16].copy_from_slice(&(len as u32).to_be_bytes());
        result.extend(data);
    }
    if let Some(head) = head {
        let adjustment = 0xB1B0AFBAu32.wrapping_sub(checksum(&result));
        result[head..head + 4].copy_from_slice(&adjustment.to_be_bytes());
    }
    Ok(result)
}
fn checksum(data: &[u8]) -> u32 {
    data.as_chunks::<4>().0.iter().fold(0u32, |sum, x| sum.wrapping_add(u32::from_be_bytes(*x)))
}

#[cfg(test)]
mod tests {
    #[test]
    fn standalone_keeps_font_names_and_coverage() {
        let original = crate::INTER_REGULAR;
        let rebuilt = super::standalone(original, 0).unwrap();
        assert_eq!(crate::embedding::names(original).unwrap(), crate::embedding::names(&rebuilt).unwrap());
        assert!(crate::embedding::covers(&rebuilt, "Hello"));
        assert_eq!(super::checksum(&rebuilt), 0xB1B0AFBA);
        assert!(super::standalone(&[0; 16], 0).is_err());
    }
}
