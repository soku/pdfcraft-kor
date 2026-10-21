//! Fill text drawn with a font explicitly selected by the user.
use super::{AnnotError, appearance, n, num_array};
use pdfcraft_cos::{Dict, Document, Object, PdfString, Stream};

pub(crate) fn program(doc: &Document, d: &Dict) -> Result<Vec<u8>, AnnotError> {
    let object = doc.resolve(d.get(b"PCTextFont").ok_or_else(|| AnnotError::Invalid("missing text font".into()))?);
    let Object::Stream(stream) = object.as_ref() else { return Err(AnnotError::Invalid("invalid text font".into())) };
    let bytes = stream.decoded_within(16 << 20).map_err(|e| AnnotError::Invalid(e.to_string()))?;
    pdfcraft_fonts::embedding::validate(&bytes).map_err(AnnotError::Invalid)?;
    Ok(bytes)
}

pub(crate) fn build(doc: &mut Document, d: &Dict) -> Result<Stream, AnnotError> {
    let bytes = program(doc, d)?;
    let (family, _) = pdfcraft_fonts::embedding::names(&bytes).map_err(AnnotError::Invalid)?;
    let text = d.get(b"Contents").and_then(Object::as_string).map(PdfString::to_text).unwrap_or_default();
    if text.len() > 262_144 {
        return Err(AnnotError::Invalid("text exceeds 256 KiB".into()));
    }
    let rect = d.get(b"Rect").and_then(Object::as_array).ok_or_else(|| AnnotError::Invalid("missing text rectangle".into()))?;
    let values: Vec<_> = rect.iter().filter_map(Object::as_f64).collect();
    let [x0, y0, x1, y1] = values.as_slice() else { return Err(AnnotError::Invalid("invalid text rectangle".into())) };
    if !values.iter().all(|x| x.is_finite() && x.abs() < 1e7) || x1 <= x0 || y1 <= y0 {
        return Err(AnnotError::Invalid("invalid text rectangle".into()));
    }
    // Match the standard-font appearance: draw upright in displayed coordinates,
    // then map the form back into the annotation's user-space rectangle.
    let turn = appearance::text_turn(d);
    let (drawing_rect, matrix) = if turn == 0 {
        ([*x0, *y0, *x1, *y1], None)
    } else {
        let (rw, rh) = (x1 - x0, y1 - y0);
        let (width, height) = if matches!(turn, 90 | 270) { (rh, rw) } else { (rw, rh) };
        ([0.0, 0.0, width, height], Some(pdfcraft_model::view_matrix_for(turn, [*x0, *y0, *x1, *y1])))
    };
    let [x0, y0, x1, y1] = &drawing_rect;
    let (color, size) = appearance::parse_da(d);
    let width = (x1 - x0 - 4.0).max(1.0);
    let mut failed = None;
    let rows = pdfcraft_fonts::wrap_with(&text, size, width, |text, size| {
        pdfcraft_fonts::source_shaping::shape(&bytes, text).map_or(f64::INFINITY, |r| r.advance * size)
    });
    let mut mappings = Vec::new();
    let mut positioned = Vec::new();
    let mut y = y1 - 2.0 - size;
    for row in rows {
        match pdfcraft_fonts::source_shaping::shape(&bytes, &row) {
            Ok(run) => {
                for g in run.glyphs {
                    if mappings.len() >= 65534 {
                        return Err(AnnotError::Invalid("text exceeds glyph limit".into()));
                    }
                    positioned.push((x0 + 2.0 + g.position[0] * size, y + g.position[1] * size));
                    mappings.push((g.text, g.id));
                }
            }
            Err(e) => {
                failed = Some(e);
                break;
            }
        }
        y -= size * 1.2;
    }
    if let Some(e) = failed {
        return Err(AnnotError::Invalid(e));
    }
    let mut fonts = Dict::new();
    let resource = pdfcraft_fonts::embedding::embed_mapping(doc, &mut fonts, &bytes, &family, &mappings).map_err(AnnotError::Invalid)?;
    let mut content = format!(
        "q /GS gs {} {} {} rg {} {} {} {} re W n BT /{} {} Tf\n",
        n(color[0]),
        n(color[1]),
        n(color[2]),
        n(*x0),
        n(*y0),
        n(x1 - x0),
        n(y1 - y0),
        resource,
        n(size)
    );
    use std::fmt::Write;
    for (i, (x, y)) in positioned.into_iter().enumerate() {
        let _ = writeln!(content, "1 0 0 1 {} {} Tm <{:04X}> Tj", n(x), n(y), i + 1);
    }
    content.push_str("ET Q\n");
    let mut resources = Dict::new();
    resources.set(b"Font".to_vec(), Object::Dict(fonts));
    let mut gs = Dict::new();
    gs.set(b"ca".to_vec(), Object::Real(d.get(b"CA").and_then(Object::as_f64).unwrap_or(1.0).clamp(0.0, 1.0)));
    let mut states = Dict::new();
    states.set(b"GS".to_vec(), Object::Dict(gs));
    resources.set(b"ExtGState".to_vec(), Object::Dict(states));
    let mut dict = Dict::new();
    dict.set(b"Type".to_vec(), Object::name("XObject"));
    dict.set(b"Subtype".to_vec(), Object::name("Form"));
    dict.set(b"BBox".to_vec(), num_array(&[*x0, *y0, *x1, *y1]));
    dict.set(b"Resources".to_vec(), Object::Dict(resources));
    if let Some(matrix) = matrix {
        dict.set(b"Matrix".to_vec(), num_array(&matrix));
    }
    Ok(Stream::from_raw(dict, content.into_bytes()))
}
