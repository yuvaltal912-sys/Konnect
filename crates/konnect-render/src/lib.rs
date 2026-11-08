//! Deterministic SVG rasterization for Konnect's visual-feedback tools.
//!
//! Determinism is the design constraint, not a nice-to-have: rendered PNGs
//! become stored visual baselines that are compared pixel-for-pixel across
//! machines and sessions. Two rules follow. The renderer versions are pinned
//! exactly in the workspace manifest, and **no system font is ever
//! consulted** — the font database starts empty, so a schematic SVG that
//! depends on text elements (rather than KiCad's stroke-font path data)
//! fails loudly instead of rendering differently on every machine.

use anyhow::{bail, Context, Result};
use resvg::{tiny_skia, usvg};

/// Identity of the rendering stack, stored in every visual baseline and
/// compared before a baseline diff is trusted. The version half MUST move
/// with the exact pins in the workspace manifest — a resvg bump that changes
/// antialiasing by one ulp per edge would otherwise read as design drift.
pub const RENDERER_ID: &str = "resvg-0.48.1";

/// A rasterized image plus the facts a caller reports about it.
#[derive(Debug)]
pub struct Rendered {
    /// Encoded PNG bytes.
    pub png: Vec<u8>,
    pub width_px: u32,
    pub height_px: u32,
}

/// A schematic viewport in millimetres from the page's top-left corner.
#[derive(Clone, Copy, Debug)]
pub struct Region {
    pub x_mm: f64,
    pub y_mm: f64,
    pub width_mm: f64,
    pub height_mm: f64,
}

/// Rasterize a region of a KiCad schematic SVG without losing close-up detail.
///
/// KiCad exports the physical page width and height in `mm`. Those attributes
/// map page coordinates to the parsed SVG viewport; any SVG `viewBox` origin
/// and transform remain the renderer's responsibility. SVGs without explicit
/// millimetre page dimensions are refused rather than guessed at.
///
/// The output width is 1..=8192, height at most 8192, and area at most 16 Mi
/// pixels. A region must be finite, positive and entirely inside the page.
/// The full-sheet renderer remains separate so existing visual baselines keep
/// exactly the same rasterization path.
pub fn svg_region_to_png(svg: &[u8], width_px: u32, region: Region) -> Result<Rendered> {
    svg_region_to_png_on(svg, width_px, region, WHITE)
}

/// White paper, the schematic and symbol background.
pub const WHITE: [u8; 3] = [255, 255, 255];

/// KiCad's default board-editor background. Board and footprint plots use
/// pale silkscreen and white drill marks that vanish on white paper.
pub const BOARD_BACKGROUND: [u8; 3] = [0x00, 0x10, 0x23];

/// [`svg_region_to_png`] on an opaque `background` colour.
pub fn svg_region_to_png_on(
    svg: &[u8],
    width_px: u32,
    region: Region,
    background: [u8; 3],
) -> Result<Rendered> {
    if width_px == 0 || width_px > 8192 {
        bail!("width_px must be 1..=8192, got {width_px}");
    }
    if ![region.x_mm, region.y_mm, region.width_mm, region.height_mm]
        .iter()
        .all(|value| value.is_finite())
        || region.x_mm < 0.0
        || region.y_mm < 0.0
        || region.width_mm <= 0.0
        || region.height_mm <= 0.0
    {
        bail!("region must have finite nonnegative x_mm/y_mm and positive width_mm/height_mm");
    }

    let source = std::str::from_utf8(svg).context("SVG is not UTF-8")?;
    let document = parse_document(source)?;
    let (page_width_mm, page_height_mm) = page_size_of(&document)?;
    if region.x_mm + region.width_mm > page_width_mm
        || region.y_mm + region.height_mm > page_height_mm
    {
        bail!("region lies outside the SVG page ({page_width_mm} x {page_height_mm} mm)");
    }

    let height = (f64::from(width_px) * region.height_mm / region.width_mm)
        .round()
        .max(1.0);
    if !height.is_finite() || height > 8192.0 {
        bail!("region output height exceeds 8192 pixels; reduce width_px or widen the region");
    }
    let height_px = height as u32;
    if u64::from(width_px) * u64::from(height_px) > 16 * 1024 * 1024 {
        bail!("region output exceeds 16777216 pixels; reduce width_px");
    }

    // Apply the same font policy as full-sheet renders.
    if svg_contains_visible_text(svg) {
        bail!("SVG contains visible <text> elements; region rendering requires stroke-font paths");
    }
    let tree =
        usvg::Tree::from_data(svg, &usvg::Options::default()).context("SVG did not parse")?;
    let pixels_per_mm = f64::from(width_px) / region.width_mm;
    let transform = tiny_skia::Transform::from_row(
        (pixels_per_mm * page_width_mm / f64::from(tree.size().width())) as f32,
        0.0,
        0.0,
        (pixels_per_mm * page_height_mm / f64::from(tree.size().height())) as f32,
        (-region.x_mm * pixels_per_mm) as f32,
        (-region.y_mm * pixels_per_mm) as f32,
    );
    if !transform.is_finite() {
        bail!("region is too small for the renderer's coordinate precision");
    }

    let mut pixmap =
        tiny_skia::Pixmap::new(width_px, height_px).context("could not allocate pixmap")?;
    pixmap.fill(tiny_skia::Color::from_rgba8(
        background[0],
        background[1],
        background[2],
        255,
    ));
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    let png = pixmap.encode_png().context("PNG encoding failed")?;
    Ok(Rendered {
        png,
        width_px,
        height_px,
    })
}

/// KiCad includes an external DOCTYPE declaration. roxmltree does not fetch
/// external resources; allowing the declaration lets real exports be
/// inspected in the same way as they are parsed by usvg.
fn parse_document(source: &str) -> Result<usvg::roxmltree::Document<'_>> {
    usvg::roxmltree::Document::parse_with_options(
        source,
        usvg::roxmltree::ParsingOptions {
            allow_dtd: true,
            ..Default::default()
        },
    )
    .context("SVG did not parse")
}

fn page_size_of(document: &usvg::roxmltree::Document<'_>) -> Result<(f64, f64)> {
    let root = document.root_element();
    let page_mm = |attribute: &str| -> Result<f64> {
        root.attribute(attribute)
            .and_then(|value| value.trim().strip_suffix("mm"))
            .and_then(|value| value.trim().parse::<f64>().ok())
            .filter(|value| value.is_finite() && *value > 0.0)
            .with_context(|| format!("region rendering requires a positive SVG {attribute} in mm"))
    };
    Ok((page_mm("width")?, page_mm("height")?))
}

/// The physical page size `(width_mm, height_mm)` a KiCad SVG declares.
/// Refuses an SVG without explicit positive millimetre dimensions.
pub fn page_size_mm(svg: &[u8]) -> Result<(f64, f64)> {
    let source = std::str::from_utf8(svg).context("SVG is not UTF-8")?;
    page_size_of(&parse_document(source)?)
}

/// Rasterize a whole KiCad SVG page (a board plot, a library symbol or a
/// footprint) at `width_px` with the same guards as [`svg_region_to_png`]:
/// millimetre page dimensions are required, the output height is at most
/// 8192 px and the area at most 16 Mi pixels, and visible `<text>` is
/// refused. Unlike [`svg_to_png`], a very tall page cannot produce an
/// unbounded image. Visual baselines keep using [`svg_to_png`].
pub fn svg_page_to_png(svg: &[u8], width_px: u32, background: [u8; 3]) -> Result<Rendered> {
    let (width_mm, height_mm) = page_size_mm(svg)?;
    svg_region_to_png_on(
        svg,
        width_px,
        Region {
            x_mm: 0.0,
            y_mm: 0.0,
            width_mm,
            height_mm,
        },
        background,
    )
}

/// Rasterize SVG bytes to a PNG at the given pixel width (height follows the
/// SVG's aspect ratio).
///
/// Refuses an SVG containing VISIBLE `<text>` elements: with an empty fontdb
/// they would render differently per machine. kicad-cli schematic exports
/// draw all visible text as stroke-font path data and pair it with
/// `opacity="0"` text elements for searchability; those render nothing by
/// construction and are allowed (verified against real 10.0.5 exports:
/// every text element carries opacity="0" stroke-opacity="0").
pub fn svg_to_png(svg: &[u8], width_px: u32) -> Result<Rendered> {
    if width_px == 0 || width_px > 8192 {
        bail!("width_px must be 1..=8192, got {width_px}");
    }

    let options = usvg::Options::default();
    let tree = usvg::Tree::from_data(svg, &options).context("SVG did not parse")?;

    // usvg resolves text during parsing against options.fontdb (empty here).
    // A VISIBLE text node is a determinism hazard; refuse it. Invisible
    // (opacity 0) text, which kicad-cli emits for searchability alongside
    // its stroke-font paths, paints nothing regardless of fonts.
    if svg_contains_visible_text(svg) {
        bail!(
            "SVG contains visible <text> elements; rendering them requires              fonts and is not deterministic. KiCad schematic exports draw              text as stroke-font paths."
        );
    }

    let size = tree.size();
    if size.width() <= 0.0 || size.height() <= 0.0 {
        bail!("SVG has no drawable area");
    }
    let scale = width_px as f32 / size.width();
    let height_px = (size.height() * scale).round().max(1.0) as u32;

    let mut pixmap =
        tiny_skia::Pixmap::new(width_px, height_px).context("could not allocate pixmap")?;
    // White ground: schematic renders are compared as opaque images so alpha
    // differences cannot masquerade as "no change".
    pixmap.fill(tiny_skia::Color::WHITE);
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );

    let png = pixmap.encode_png().context("PNG encoding failed")?;
    Ok(Rendered {
        png,
        width_px,
        height_px,
    })
}

/// The result of comparing two renders.
#[derive(Debug)]
pub struct PixelDiff {
    pub changed_pixels: u64,
    pub total_pixels: u64,
    /// Pixels that are non-white in either image: the drawing, as opposed to
    /// the paper. Drift thresholds compare against this — a schematic sheet
    /// is mostly blank page, and a percentage of the page under-reports
    /// design change by an order of magnitude.
    pub content_pixels: u64,
    /// Percentage of the page in [0, 100], rounded to 3 decimals.
    pub changed_pct: f64,
    /// Percentage of the CONTENT in [0, 100], rounded to 3 decimals; 0 when
    /// both images are blank.
    pub changed_pct_of_content: f64,
    /// Bounding box of all changed pixels (x_min, y_min, x_max, y_max),
    /// None when nothing changed.
    pub changed_bbox: Option<(u32, u32, u32, u32)>,
}

/// Per-channel-max luminance threshold under which a pixel difference is
/// noise, ported from the reference implementation (8/255).
pub const DIFF_THRESHOLD: u8 = 8;

/// Compare two PNGs pixel-for-pixel on a shared canvas sized to the larger
/// of each dimension (missing area counts as changed).
pub fn diff_pngs(before: &[u8], after: &[u8]) -> Result<PixelDiff> {
    let a = image::load_from_memory(before)
        .context("before image did not decode")?
        .to_rgba8();
    let b = image::load_from_memory(after)
        .context("after image did not decode")?
        .to_rgba8();

    let width = a.width().max(b.width());
    let height = a.height().max(b.height());

    // The paper is whatever color dominates — kicad-cli paints its own
    // background rect, so "white" is not a safe assumption. Content is any
    // pixel that differs from the dominant color in either image.
    let background = dominant_color(&a);
    let mut changed: u64 = 0;
    let mut content: u64 = 0;
    let mut bbox: Option<(u32, u32, u32, u32)> = None;

    for y in 0..height {
        for x in 0..width {
            let pa = pixel_or_white(&a, x, y);
            let pb = pixel_or_white(&b, x, y);
            if pa != background || pb != background {
                content += 1;
            }
            let delta = pa
                .iter()
                .zip(pb.iter())
                .map(|(ca, cb)| ca.abs_diff(*cb))
                .max()
                .unwrap_or(0);
            if delta > DIFF_THRESHOLD {
                changed += 1;
                bbox = Some(match bbox {
                    None => (x, y, x, y),
                    Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
                });
            }
        }
    }

    let total = u64::from(width) * u64::from(height);
    let round3 = |v: f64| (v * 1000.0).round() / 1000.0;
    let pct = if total == 0 {
        0.0
    } else {
        round3(changed as f64 / total as f64 * 100.0)
    };
    let pct_of_content = if content == 0 {
        0.0
    } else {
        round3(changed as f64 / content as f64 * 100.0)
    };
    Ok(PixelDiff {
        changed_pixels: changed,
        total_pixels: total,
        content_pixels: content,
        changed_pct: pct,
        changed_pct_of_content: pct_of_content,
        changed_bbox: bbox,
    })
}

/// Colours of the generated difference image. Unchanged drawing is faded so
/// that the change stands out; the legend travels in the tool's metadata.
pub mod diff_colors {
    /// Drawn now, blank in the baseline.
    pub const ADDED: [u8; 3] = [0, 160, 0];
    /// Drawn in the baseline, blank now.
    pub const REMOVED: [u8; 3] = [220, 0, 0];
    /// Drawn in both, in a different colour.
    pub const MODIFIED: [u8; 3] = [230, 140, 0];
    /// Outline around the changed region's bounding box.
    pub const BBOX: [u8; 3] = [0, 90, 255];
}

/// Render a difference image of two PNGs on the same canvas [`diff_pngs`]
/// uses, with the same noise threshold and background rule. Changed pixels
/// are coloured by [`diff_colors`]; unchanged drawing is faded to a quarter
/// of its contrast; a bounding-box outline marks the changed region so a
/// small change is findable. The canvas is refused above 16 Mi pixels.
pub fn diff_image_png(before: &[u8], after: &[u8]) -> Result<Rendered> {
    let a = image::load_from_memory(before)
        .context("before image did not decode")?
        .to_rgba8();
    let b = image::load_from_memory(after)
        .context("after image did not decode")?
        .to_rgba8();
    let width = a.width().max(b.width());
    let height = a.height().max(b.height());
    if width == 0 || height == 0 {
        bail!("difference image has no area");
    }
    if u64::from(width) * u64::from(height) > 16 * 1024 * 1024 {
        bail!("difference image would exceed 16777216 pixels");
    }

    let background = dominant_color(&a);
    let mut out = image::RgbImage::new(width, height);
    let mut bbox: Option<(u32, u32, u32, u32)> = None;
    for y in 0..height {
        for x in 0..width {
            let pa = pixel_or_white(&a, x, y);
            let pb = pixel_or_white(&b, x, y);
            let delta = pa
                .iter()
                .zip(pb.iter())
                .map(|(ca, cb)| ca.abs_diff(*cb))
                .max()
                .unwrap_or(0);
            let colour = if delta > DIFF_THRESHOLD {
                bbox = Some(match bbox {
                    None => (x, y, x, y),
                    Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
                });
                match (pa == background, pb == background) {
                    (true, false) => diff_colors::ADDED,
                    (false, true) => diff_colors::REMOVED,
                    _ => diff_colors::MODIFIED,
                }
            } else if pb == background {
                [255, 255, 255]
            } else {
                pb.map(|c| 255 - (255 - c) / 4)
            };
            out.put_pixel(x, y, image::Rgb(colour));
        }
    }

    if let Some((x0, y0, x1, y1)) = bbox {
        let margin = 3;
        let (x0, y0) = (x0.saturating_sub(margin), y0.saturating_sub(margin));
        let (x1, y1) = ((x1 + margin).min(width - 1), (y1 + margin).min(height - 1));
        let mut paint = |x: u32, y: u32| out.put_pixel(x, y, image::Rgb(diff_colors::BBOX));
        for t in 0..2 {
            for x in x0..=x1 {
                paint(x, (y0 + t).min(y1));
                paint(x, y1.saturating_sub(t).max(y0));
            }
            for y in y0..=y1 {
                paint((x0 + t).min(x1), y);
                paint(x1.saturating_sub(t).max(x0), y);
            }
        }
    }

    let mut png = Vec::new();
    out.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .context("PNG encoding failed")?;
    Ok(Rendered {
        png,
        width_px: width,
        height_px: height,
    })
}

/// The most frequent flattened color in an image: the paper. A render is
/// mostly background by construction, so the mode is unambiguous.
fn dominant_color(img: &image::RgbaImage) -> [u8; 3] {
    let mut counts: std::collections::HashMap<[u8; 3], u64> = std::collections::HashMap::new();
    for y in 0..img.height() {
        for x in 0..img.width() {
            *counts.entry(pixel_or_white(img, x, y)).or_insert(0) += 1;
        }
    }
    counts
        .into_iter()
        .max_by_key(|(_, n)| *n)
        .map(|(c, _)| c)
        .unwrap_or([255, 255, 255])
}

/// Flatten onto white outside an image's bounds and under transparency, so
/// alpha deltas register as color deltas instead of disappearing.
fn pixel_or_white(img: &image::RgbaImage, x: u32, y: u32) -> [u8; 3] {
    if x >= img.width() || y >= img.height() {
        return [255, 255, 255];
    }
    let p = img.get_pixel(x, y).0;
    let alpha = u16::from(p[3]);
    let over = |c: u8| ((u16::from(c) * alpha + 255 * (255 - alpha)) / 255) as u8;
    [over(p[0]), over(p[1]), over(p[2])]
}

/// Probe for `<text` elements whose opening tag does NOT declare full
/// transparency. kicad-cli stamps `opacity="0"` on every text element it
/// emits; anything else is treated as visible and refused.
fn svg_contains_visible_text(svg: &[u8]) -> bool {
    let text = String::from_utf8_lossy(svg);
    let mut rest = text.as_ref();
    while let Some(i) = rest.find("<text") {
        let tag_body = &rest[i..];
        let end = tag_body.find('>').unwrap_or(tag_body.len());
        if !tag_body[..end].contains("opacity=\"0\"") {
            return true;
        }
        rest = &tag_body[end..];
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    const RECT_SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="50"><rect x="10" y="10" width="30" height="20" fill="#112233"/></svg>"##;
    const MM_SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="100mm" height="50mm" viewBox="10 20 200 100"><rect x="70" y="40" width="20" height="20" fill="#112233"/><rect x="110" y="40" width="20" height="20" fill="#ff0000"/></svg>"##;

    fn region() -> Region {
        Region {
            x_mm: 30.0,
            y_mm: 10.0,
            width_mm: 10.0,
            height_mm: 10.0,
        }
    }

    #[test]
    fn region_uses_page_mm_with_a_nonzero_svg_origin_and_keeps_closeup_detail() {
        let out = svg_region_to_png(MM_SVG, 200, region()).unwrap();
        assert_eq!((out.width_px, out.height_px), (200, 200));
        let img = image::load_from_memory(&out.png).unwrap().to_rgba8();
        for (x, y) in [(2, 2), (100, 100), (197, 197)] {
            assert_eq!(img.get_pixel(x, y).0, [0x11, 0x22, 0x33, 255]);
        }
        assert!(!img.pixels().any(|pixel| pixel.0 == [255, 0, 0, 255]));
    }

    #[test]
    fn region_of_a_real_kicad_export_renders_stroke_text() {
        let svg = include_bytes!("../tests/fixtures/placement_fixture.svg");
        let out = svg_region_to_png(
            svg,
            600,
            Region {
                x_mm: 170.0,
                y_mm: 160.0,
                width_mm: 120.0,
                height_mm: 40.0,
            },
        )
        .unwrap();
        assert_eq!((out.width_px, out.height_px), (600, 200));
        let img = image::load_from_memory(&out.png).unwrap().to_rgba8();
        let background = [0xf5, 0xf4, 0xef, 255];
        assert_eq!(img.get_pixel(0, 0).0, background);
        assert!(
            img.pixels().filter(|pixel| pixel.0 != background).count() > 1000,
            "the title block contains borders and stroke-font text"
        );
    }

    #[test]
    fn region_refuses_invalid_output_width() {
        for width in [0, 8193] {
            let err = svg_region_to_png(MM_SVG, width, region()).unwrap_err();
            assert!(err.to_string().contains("width_px"), "{err}");
        }
    }

    #[test]
    fn region_refuses_nonfinite_negative_and_empty_rectangles() {
        let valid = region();
        for invalid in [
            Region {
                x_mm: f64::NAN,
                ..valid
            },
            Region {
                y_mm: f64::INFINITY,
                ..valid
            },
            Region {
                width_mm: f64::NAN,
                ..valid
            },
            Region {
                height_mm: f64::INFINITY,
                ..valid
            },
            Region {
                x_mm: -1.0,
                ..valid
            },
            Region {
                y_mm: -1.0,
                ..valid
            },
            Region {
                width_mm: 0.0,
                ..valid
            },
            Region {
                height_mm: -1.0,
                ..valid
            },
        ] {
            let err = svg_region_to_png(MM_SVG, 100, invalid).unwrap_err();
            assert!(
                err.to_string().contains("region must have"),
                "{invalid:?}: {err}"
            );
        }
    }

    #[test]
    fn region_refuses_missing_or_invalid_physical_page_dimensions() {
        for (width, height) in [
            ("100", "50mm"),
            ("100mm", "50px"),
            ("0mm", "50mm"),
            ("100mm", "-50mm"),
            ("NaNmm", "50mm"),
            ("100mm", "infmm"),
        ] {
            let svg = format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}"/>"#
            );
            let err = svg_region_to_png(svg.as_bytes(), 100, region()).unwrap_err();
            assert!(
                err.to_string().contains("positive SVG"),
                "{width} {height}: {err}"
            );
        }
    }

    #[test]
    fn region_refuses_bounds_outside_either_page_axis() {
        for invalid in [
            Region {
                x_mm: 95.0,
                ..region()
            },
            Region {
                y_mm: 45.0,
                ..region()
            },
        ] {
            let err = svg_region_to_png(MM_SVG, 100, invalid).unwrap_err();
            assert!(err.to_string().contains("outside the SVG page"), "{err}");
        }
    }

    #[test]
    fn region_refuses_excessive_height_before_allocating() {
        let err = svg_region_to_png(
            MM_SVG,
            100,
            Region {
                width_mm: 0.01,
                ..region()
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("height exceeds"), "{err}");
    }

    #[test]
    fn region_refuses_excessive_pixel_area_before_allocating() {
        let err = svg_region_to_png(MM_SVG, 5000, region()).unwrap_err();
        assert!(err.to_string().contains("16777216 pixels"), "{err}");
    }

    #[test]
    fn region_refuses_visible_text_instead_of_omitting_it() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="100mm" height="50mm"><text x="30" y="20">hi</text></svg>"#;
        let err = svg_region_to_png(svg, 100, region()).unwrap_err();
        assert!(err.to_string().contains("visible <text>"), "{err}");
    }

    #[test]
    fn region_refuses_an_unrepresentable_transform() {
        let err = svg_region_to_png(
            MM_SVG,
            100,
            Region {
                width_mm: 1e-100,
                height_mm: 1e-100,
                ..region()
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("coordinate precision"), "{err}");
    }

    #[test]
    fn renders_a_rect_at_requested_width() {
        let out = svg_to_png(RECT_SVG, 200).unwrap();
        assert_eq!(out.width_px, 200);
        assert_eq!(out.height_px, 100, "aspect ratio preserved");
        let img = image::load_from_memory(&out.png).unwrap().to_rgba8();
        assert_eq!(
            img.get_pixel(50, 40).0,
            [0x11, 0x22, 0x33, 255],
            "rect body"
        );
        assert_eq!(img.get_pixel(5, 5).0, [255, 255, 255, 255], "white ground");
    }

    #[test]
    fn same_input_renders_byte_identically_twice() {
        // Smoke test only — the real determinism gate is the cross-OS CI
        // hash comparison added with the visual tools.
        let a = svg_to_png(RECT_SVG, 200).unwrap();
        let b = svg_to_png(RECT_SVG, 200).unwrap();
        assert_eq!(a.png, b.png);
    }

    #[test]
    fn visible_text_is_refused_not_silently_dropped() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><text x="0" y="8">hi</text></svg>"##;
        let err = svg_to_png(svg, 100).unwrap_err().to_string();
        assert!(err.contains("visible <text>"), "{err}");
    }

    /// kicad-cli pairs every visible string with an invisible text element
    /// (opacity 0) for searchability; those must render, not refuse.
    #[test]
    fn kicads_invisible_searchability_text_is_allowed() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"><text x="0" y="8" opacity="0" stroke-opacity="0">R1</text><rect x="2" y="2" width="10" height="5" fill="#000000"/></svg>"##;
        let out = svg_to_png(svg, 100).unwrap();
        assert_eq!(out.width_px, 100);
    }

    #[test]
    fn diff_reports_the_changed_region() {
        let before = svg_to_png(RECT_SVG, 100).unwrap();
        let moved: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="50"><rect x="60" y="10" width="30" height="20" fill="#112233"/></svg>"##;
        let after = svg_to_png(moved, 100).unwrap();
        let diff = diff_pngs(&before.png, &after.png).unwrap();
        assert!(diff.changed_pixels > 0);
        let (x0, _, x1, _) = diff.changed_bbox.unwrap();
        assert!(
            x0 >= 10 && x1 <= 90,
            "change confined to the two rect sites"
        );
    }

    #[test]
    fn identical_images_diff_to_zero_with_no_bbox() {
        let a = svg_to_png(RECT_SVG, 100).unwrap();
        let diff = diff_pngs(&a.png, &a.png).unwrap();
        assert_eq!(diff.changed_pixels, 0);
        assert_eq!(diff.changed_pct, 0.0);
        assert!(diff.changed_bbox.is_none());
    }

    #[test]
    fn page_render_uses_the_whole_mm_page_and_reports_its_size() {
        assert_eq!(page_size_mm(MM_SVG).unwrap(), (100.0, 50.0));
        let out = svg_page_to_png(MM_SVG, 200, WHITE).unwrap();
        assert_eq!((out.width_px, out.height_px), (200, 100));
        let img = image::load_from_memory(&out.png).unwrap().to_rgba8();
        // viewBox 10 20 200 100 on a 100 x 50 mm page: the dark square at
        // x 70..90 user units sits at 30..40 mm, i.e. pixels 60..80.
        assert_eq!(img.get_pixel(70, 30).0, [0x11, 0x22, 0x33, 255]);
        assert_eq!(img.get_pixel(110, 30).0, [255, 0, 0, 255]);
        assert_eq!(img.get_pixel(5, 5).0, [255, 255, 255, 255]);
        let dark = svg_page_to_png(MM_SVG, 200, BOARD_BACKGROUND).unwrap();
        let img = image::load_from_memory(&dark.png).unwrap().to_rgba8();
        assert_eq!(img.get_pixel(5, 5).0, [0x00, 0x10, 0x23, 255]);
        assert_eq!(img.get_pixel(70, 30).0, [0x11, 0x22, 0x33, 255]);
    }

    #[test]
    fn page_render_caps_a_tall_page_instead_of_allocating_it() {
        let tall = br##"<svg xmlns="http://www.w3.org/2000/svg" width="1mm" height="100mm"/>"##;
        let err = svg_page_to_png(tall, 100, WHITE).unwrap_err();
        assert!(err.to_string().contains("height exceeds"), "{err}");
    }

    #[test]
    fn page_render_refuses_visible_text_and_pages_without_mm() {
        let text = br#"<svg xmlns="http://www.w3.org/2000/svg" width="10mm" height="10mm"><text x="1" y="8">hi</text></svg>"#;
        let err = svg_page_to_png(text, 100, WHITE).unwrap_err();
        assert!(err.to_string().contains("visible <text>"), "{err}");
        let err = svg_page_to_png(RECT_SVG, 100, WHITE).unwrap_err();
        assert!(err.to_string().contains("positive SVG"), "{err}");
    }

    #[test]
    fn diff_image_colours_added_removed_and_unchanged_drawing() {
        let before = svg_to_png(RECT_SVG, 100).unwrap();
        let moved: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="50"><rect x="30" y="10" width="30" height="20" fill="#112233"/></svg>"##;
        let after = svg_to_png(moved, 100).unwrap();
        let out = diff_image_png(&before.png, &after.png).unwrap();
        assert_eq!((out.width_px, out.height_px), (100, 50));
        let img = image::load_from_memory(&out.png).unwrap().to_rgb8();
        // Old site only (x 10..30): removed. New site only (x 40..60): added.
        assert_eq!(img.get_pixel(15, 20).0, diff_colors::REMOVED);
        assert_eq!(img.get_pixel(50, 20).0, diff_colors::ADDED);
        // Overlap (x 30..40) is unchanged drawing, faded, not a change colour.
        let overlap = img.get_pixel(35, 20).0;
        assert_eq!(overlap, [0x11, 0x22, 0x33].map(|c: u8| 255 - (255 - c) / 4));
        // Paper stays white and the bbox outline brackets the change.
        assert_eq!(img.get_pixel(90, 45).0, [255, 255, 255]);
        assert_eq!(img.get_pixel(7, 20).0, diff_colors::BBOX);
        assert_eq!(img.get_pixel(62, 20).0, diff_colors::BBOX);
    }

    #[test]
    fn diff_image_of_identical_renders_has_no_change_colours() {
        let a = svg_to_png(RECT_SVG, 100).unwrap();
        let out = diff_image_png(&a.png, &a.png).unwrap();
        let img = image::load_from_memory(&out.png).unwrap().to_rgb8();
        for colour in [
            diff_colors::ADDED,
            diff_colors::REMOVED,
            diff_colors::MODIFIED,
            diff_colors::BBOX,
        ] {
            assert!(!img.pixels().any(|p| p.0 == colour), "{colour:?}");
        }
    }

    #[test]
    fn diff_image_marks_a_recoloured_drawing_as_modified() {
        let before = svg_to_png(RECT_SVG, 100).unwrap();
        let red: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="50"><rect x="10" y="10" width="30" height="20" fill="#ff0000"/></svg>"##;
        let after = svg_to_png(red, 100).unwrap();
        let img = image::load_from_memory(&diff_image_png(&before.png, &after.png).unwrap().png)
            .unwrap()
            .to_rgb8();
        assert_eq!(img.get_pixel(25, 20).0, diff_colors::MODIFIED);
    }

    #[test]
    fn diff_image_refuses_undecodable_input() {
        let a = svg_to_png(RECT_SVG, 10).unwrap();
        let err = diff_image_png(b"not a png", &a.png).unwrap_err();
        assert!(err.to_string().contains("before image"), "{err}");
    }
}
