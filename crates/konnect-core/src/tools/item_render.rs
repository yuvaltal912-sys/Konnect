//! Inline renders of board layers and library items (creation rank 25).
//!
//! Each tool asks kicad-cli for an SVG in a private temporary directory and
//! rasterizes it in-process with `konnect-render`, so no system font is read
//! and no file next to the design is touched unless the caller names an
//! `output`. KiCad 10 draws every visible string in these plots as stroke or
//! outline paths and adds an invisible `opacity="0"` copy for search; the
//! renderer's refusal of visible `<text>` therefore stays in force unchanged.
//! The PNG reaches the client as an MCP image block next to JSON metadata,
//! through the same helper the schematic renders use.

use crate::mcp::error::ToolErrorKind;
use crate::mcp::protocol::{inline_over_budget, CallToolResult, INLINE_IMAGE_MAX_SIDE_PX};
use crate::tool;
use crate::tools::{get_path, ToolContext, ToolDef};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

use super::cli;

const MAX_WIDTH_PX: u64 = 4096;

pub(super) fn board_tools() -> Vec<ToolDef> {
    vec![tool!(
        "render_board_layers",
        "Plot selected board layers with kicad-cli (2D, like the PCB editor) and return a PNG \
         as an MCP image block next to JSON metadata. Reads the saved .kicad_pcb file. Without \
         'region' the image frames the board-edge bounding box; with 'region' it crops a \
         rectangle given in board millimetres (the coordinates the editor and the board tools \
         use) at the requested width, for a close-up. Colours are KiCad's default plot colours \
         on its default dark board background; 'black_and_white' plots black on white. Width is \
         capped at 4096 px, height at 8192 px and area at 16,777,216 pixels. An inline image is \
         refused above 8000 px on a side or 3,750,000 PNG bytes (the Claude API's per-image \
         limits); 'output' with 'inline' false has no such limit. Use get_board_2d_view for a \
         3D picture.",
        json!({
            "type": "object",
            "properties": {
                "board": { "type": "string", "description": "Path to the saved .kicad_pcb file" },
                "layers": {
                    "type": "array",
                    "description": "Layer names to plot, such as [\"F.Cu\", \"F.SilkS\", \"Edge.Cuts\"]. Each must be declared by the board, by its canonical name or its user name (such as F.Silkscreen)",
                    "items": { "type": "string" },
                    "minItems": 1
                },
                "region": { "type": "object", "description": "Rectangle in board mm (x, y from the drawing sheet's top-left, as the editor shows them); must lie on the drawing sheet", "properties": {
                    "x": { "type": "number", "minimum": 0 },
                    "y": { "type": "number", "minimum": 0 },
                    "width": { "type": "number", "exclusiveMinimum": 0 },
                    "height": { "type": "number", "exclusiveMinimum": 0 }
                }, "required": ["x", "y", "width", "height"] },
                "width_px": { "type": "integer", "description": "Output width in pixels (of the region, when given)", "default": 1600, "minimum": 1, "maximum": 4096 },
                "black_and_white": { "type": "boolean", "description": "Plot black on white", "default": false },
                "inline": { "type": "boolean", "description": "Return the PNG as an MCP image block", "default": true },
                "output": { "type": "string", "description": "Also write the PNG to this path. Required when 'inline' is false" }
            },
            "required": ["board", "layers"]
        }),
        |args, ctx| async move { handle_render_board_layers(args, ctx).await }
    )]
}

pub(super) fn library_tools() -> Vec<ToolDef> {
    vec![tool!(
        "render_library_item",
        "Render one library symbol or footprint with kicad-cli and return a PNG as an MCP image \
         block next to JSON metadata, so a new or edited part can be checked by eye before it is \
         placed: pin sides and numbers on a symbol; pads, pitch, courtyard and silkscreen on a \
         footprint. The library path picks the kind: a .kicad_sym file renders symbol 'name' \
         (one unit and body style; the metadata lists the others); a .pretty directory renders \
         footprint 'name'; a single .kicad_mod file, such as create_footprint writes, renders \
         that footprint. Symbols draw on white; footprints use KiCad's default plot colours on \
         its default dark board background unless 'black_and_white'. An option for the other \
         kind is refused unless it is null or its default. Width is capped at 4096 px, height \
         at 8192 px and area at 16,777,216 pixels. An inline image is refused above 8000 px on \
         a side or 3,750,000 PNG bytes (the Claude API's per-image limits); 'output' with \
         'inline' false has no such limit.",
        json!({
            "type": "object",
            "properties": {
                "library": { "type": "string", "description": "A .kicad_sym file, a .pretty directory, or one .kicad_mod file" },
                "name": { "type": "string", "description": "Symbol or footprint name in the library. Optional for a .kicad_mod file, whose footprint is its file stem. A footprint name is a bare name, not a path" },
                "unit": { "type": "integer", "description": "Symbols only: unit number to render", "default": 1, "minimum": 1 },
                "body_style": { "type": "string", "enum": ["standard", "demorgan"], "description": "Symbols only", "default": "standard" },
                "include_hidden_pins": { "type": "boolean", "description": "Symbols only", "default": false },
                "layers": {
                    "type": "array",
                    "description": "Footprints only: layer names to plot (default: KiCad's footprint plot set). Canonical names such as F.SilkS, or KiCad's default user names such as F.Silkscreen",
                    "items": { "type": "string" }
                },
                "black_and_white": { "type": "boolean", "description": "Footprints only: plot black on white", "default": false },
                "width_px": { "type": "integer", "description": "Output width in pixels", "default": 800, "minimum": 1, "maximum": 4096 },
                "inline": { "type": "boolean", "description": "Return the PNG as an MCP image block", "default": true },
                "output": { "type": "string", "description": "Also write the PNG to this path. Required when 'inline' is false" }
            },
            "required": ["library"]
        }),
        |args, ctx| async move { handle_render_library_item(args, ctx).await }
    )]
}

const SYMBOL_ONLY: [&str; 3] = ["unit", "body_style", "include_hidden_pins"];
const FOOTPRINT_ONLY: [&str; 2] = ["layers", "black_and_white"];

/// An option of the other kind asks for nothing when it is null or the
/// default the schema documents, so a client that fills in defaults is not
/// refused. `layers` documents no literal default; only null passes.
fn asks_for_nothing(field: &str, value: &Value) -> bool {
    value.is_null()
        || match field {
            "unit" => value.as_u64() == Some(1),
            "body_style" => value.as_str() == Some("standard"),
            "include_hidden_pins" | "black_and_white" => value.as_bool() == Some(false),
            _ => false,
        }
}

/// Pick the renderer by the library's file type, and refuse an option that
/// belongs to the other kind instead of ignoring it, unless it asks for
/// nothing ([`asks_for_nothing`]).
pub(crate) async fn handle_render_library_item(
    args: &Value,
    ctx: &ToolContext,
) -> anyhow::Result<CallToolResult> {
    let library = get_path(args, "library")?;
    let symbol = match library.extension().and_then(|e| e.to_str()) {
        Some("kicad_sym") => true,
        Some("pretty") | Some("kicad_mod") => false,
        _ => {
            return Ok(invalid(
                "library",
                "must be a .kicad_sym file, a .pretty directory or a .kicad_mod file",
            ))
        }
    };
    let (kind, foreign) = if symbol {
        ("symbol", &FOOTPRINT_ONLY[..])
    } else {
        ("footprint", &SYMBOL_ONLY[..])
    };
    if let Some(field) = foreign.iter().find(|field| {
        args.get(**field)
            .is_some_and(|value| !asks_for_nothing(field, value))
    }) {
        return Ok(invalid(
            field,
            format!(
                "does not apply to a {kind}; {} is a {kind} library",
                library.display()
            ),
        ));
    }
    if symbol {
        handle_render_library_symbol(args, ctx).await
    } else {
        handle_render_library_footprint(args, ctx).await
    }
}

// ─── Shared argument handling and response ───────────────────────────────────

fn invalid(field: &str, reason: impl Into<String>) -> CallToolResult {
    let reason = reason.into();
    CallToolResult::error_kind(
        ToolErrorKind::InvalidArgument {
            field: field.into(),
            reason: reason.clone(),
        },
        format!("Argument '{field}' is invalid: {reason}"),
    )
}

fn not_found(path: &Path, what: &str) -> CallToolResult {
    CallToolResult::error_kind(
        ToolErrorKind::FileNotFound {
            path: path.display().to_string(),
        },
        format!("{what} not found: {}", path.display()),
    )
}

/// Where the PNG goes: back to the client, to a file, or both.
struct Delivery {
    inline: bool,
    output: Option<PathBuf>,
    width_px: u32,
}

fn delivery(args: &Value, default_width: u32) -> Result<Delivery, CallToolResult> {
    let width_px = match args.get("width_px") {
        None => default_width,
        Some(value) => match value.as_u64() {
            Some(width) if (1..=MAX_WIDTH_PX).contains(&width) => width as u32,
            _ => return Err(invalid("width_px", "must be an integer in 1..=4096")),
        },
    };
    let inline = match args.get("inline") {
        None => true,
        Some(value) => value
            .as_bool()
            .ok_or_else(|| invalid("inline", "must be a boolean"))?,
    };
    let output = match args.get("output") {
        None => None,
        Some(value) => Some(PathBuf::from(
            value
                .as_str()
                .ok_or_else(|| invalid("output", "must be a path string"))?,
        )),
    };
    if !inline && output.is_none() {
        return Err(invalid(
            "output",
            "is required when 'inline' is false, or the render would go nowhere",
        ));
    }
    Ok(Delivery {
        inline,
        output,
        width_px,
    })
}

fn flag(args: &Value, field: &str) -> Result<bool, CallToolResult> {
    match args.get(field) {
        None => Ok(false),
        Some(value) => value
            .as_bool()
            .ok_or_else(|| invalid(field, "must be a boolean")),
    }
}

fn string_list(args: &Value, field: &str) -> Result<Vec<String>, CallToolResult> {
    super::pcb_export::optional_string_array(args, field)
}

/// Rasterize, optionally write, and answer. Dimensions and byte counts come
/// from the produced image, never from the request. An image over the
/// inline budget is refused with its remedies; `narrow` adds the tool's own
/// (such as a region), and a file already written is named, not hidden.
fn respond(
    delivery: &Delivery,
    rendered: anyhow::Result<konnect_render::Rendered>,
    mut metadata: Value,
    narrow: &str,
) -> anyhow::Result<CallToolResult> {
    let rendered = match rendered {
        Ok(rendered) => rendered,
        Err(error) => {
            return Ok(CallToolResult::error(format!(
                "kicad-cli exported the SVG but rasterization refused it: {error:#}"
            )))
        }
    };
    if let Some(output) = &delivery.output {
        if let Some(parent) = output.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(output, &rendered.png)?;
        metadata["rendered"] = json!(output.display().to_string());
    }
    metadata["width_px"] = json!(rendered.width_px);
    metadata["height_px"] = json!(rendered.height_px);
    metadata["png_bytes"] = json!(rendered.png.len());
    metadata["renderer"] = json!(konnect_render::RENDERER_ID);
    if !delivery.inline {
        return Ok(CallToolResult::json(&metadata));
    }
    if let Some(over) =
        inline_over_budget(rendered.width_px, rendered.height_px, rendered.png.len())
    {
        return Ok(over.refusal(narrow, delivery.output.as_deref()));
    }
    Ok(CallToolResult::json_with_png(&metadata, &rendered.png))
}

/// Rasterize a whole page. A page bound for an inline block is first held
/// to the inline side limit, so its refusal names a `width_px` that also
/// fits inline, not one that only fits the 8192 px render cap.
fn render_page(
    delivery: &Delivery,
    svg: &[u8],
    paper: [u8; 3],
) -> anyhow::Result<konnect_render::Rendered> {
    if delivery.inline {
        if let Ok((width_mm, height_mm)) = konnect_render::page_size_mm(svg) {
            let height_px = (f64::from(delivery.width_px) * height_mm / width_mm).round();
            if height_px > f64::from(INLINE_IMAGE_MAX_SIDE_PX) {
                anyhow::bail!(
                    "page output height would be {height_px} px at width_px {}, over the inline \
                     limit of {INLINE_IMAGE_MAX_SIDE_PX} px per side; a {width_mm} x {height_mm} \
                     mm page fits inline at width_px {} or less, or set 'inline' false with an \
                     'output' path",
                    delivery.width_px,
                    konnect_render::page_fit_width_px(
                        width_mm,
                        height_mm,
                        INLINE_IMAGE_MAX_SIDE_PX
                    )
                );
            }
        }
    }
    konnect_render::svg_page_to_png(svg, delivery.width_px, paper)
}

fn background(black_and_white: bool) -> [u8; 3] {
    if black_and_white {
        konnect_render::WHITE
    } else {
        konnect_render::BOARD_BACKGROUND
    }
}

fn hex(colour: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", colour[0], colour[1], colour[2])
}

// ─── render_board_layers ─────────────────────────────────────────────────────

pub(crate) async fn handle_render_board_layers(
    args: &Value,
    ctx: &ToolContext,
) -> anyhow::Result<CallToolResult> {
    let board = get_path(args, "board")?;
    let layers = match string_list(args, "layers") {
        Ok(layers) if !layers.is_empty() => layers,
        Ok(_) => return Ok(invalid("layers", "must name at least one layer")),
        Err(error) => return Ok(error),
    };
    let region = match super::sch_export::render_region(args) {
        Ok(region) => region,
        Err(error) => return Ok(error),
    };
    let delivery = match delivery(args, 1600) {
        Ok(delivery) => delivery,
        Err(error) => return Ok(error),
    };
    let black_and_white = match flag(args, "black_and_white") {
        Ok(value) => value,
        Err(error) => return Ok(error),
    };

    let source = match std::fs::read_to_string(&board) {
        Ok(source) => source,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(not_found(&board, "Board file"))
        }
        Err(error) => return Err(error.into()),
    };
    // A layer the board does not declare would plot as nothing, which reads
    // as "the layer is empty". Refuse it by name instead. A layer may be
    // named by its canonical or its user name.
    let declared = konnect_sexp::layers::layers(&konnect_sexp::parser::parse_sexp(&source)?);
    let mut kicad_layers = Vec::with_capacity(layers.len());
    for requested in &layers {
        match kicad_board_layer(&declared, requested) {
            Ok(name) => kicad_layers.push(name),
            Err(reason) => return Ok(invalid("layers", reason)),
        }
    }

    let page = if region.is_some() {
        cli::BoardSvgPage::BoardCoordinates
    } else {
        cli::BoardSvgPage::BoardArea
    };
    let staging = tempfile::tempdir()?;
    let svg_path = staging.path().join("board.svg");
    let layer_refs: Vec<&str> = kicad_layers.iter().map(String::as_str).collect();
    cli::export_pcb_layers_svg(
        &ctx.config.kicad_cli,
        &board,
        &svg_path,
        &layer_refs,
        page,
        black_and_white,
    )
    .await?;
    let svg = std::fs::read(&svg_path)?;

    let paper = background(black_and_white);
    let rendered = match region {
        Some(region) => {
            konnect_render::svg_region_to_png_on(&svg, delivery.width_px, region, paper).map_err(
                |error| {
                    if error.to_string().contains("outside the SVG page") {
                        error.context(
                            "a region crops the drawing sheet in board mm; for a board placed \
                         off the sheet, render without 'region'",
                        )
                    } else {
                        error
                    }
                },
            )
        }
        None => render_page(&delivery, &svg, paper),
    };
    let page_mm = konnect_render::page_size_mm(&svg).ok();
    let framing = match (&region, page_mm) {
        (Some(region), _) => json!({
            "kind": "region",
            "x_mm": region.x_mm, "y_mm": region.y_mm,
            "width_mm": region.width_mm, "height_mm": region.height_mm,
        }),
        (None, Some((width_mm, height_mm))) => json!({
            "kind": "board_area",
            "width_mm": width_mm, "height_mm": height_mm,
        }),
        (None, None) => json!({ "kind": "board_area" }),
    };
    respond(
        &delivery,
        rendered,
        json!({
            "board": board.display().to_string(),
            "layers": layers,
            "framing": framing,
            "background": hex(paper),
            "black_and_white": black_and_white,
            "source": "saved file",
        }),
        ", narrow or add a 'region'",
    )
}

/// `F.SilkS (F.Silkscreen)`: a declared layer's canonical name, with the
/// user name the editor shows when it differs.
fn describe_layer(layer: &konnect_sexp::layers::Layer) -> String {
    match &layer.user_name {
        Some(user) if *user != layer.name => format!("{} ({user})", layer.name),
        _ => layer.name.clone(),
    }
}

/// The name to hand kicad-cli for a requested board layer, or why there is
/// none. A request may use a layer's canonical name or its user name.
///
/// kicad-cli 10 (`PCBNEW_JOBS_HANDLER::convertLayerArg`) looks a name up
/// among the board's user names first and its canonical names second, so
/// Konnect passes the canonical name, and refuses when another layer
/// carries that canonical name as its user name: kicad-cli would plot that
/// other layer instead.
fn kicad_board_layer(
    declared: &[konnect_sexp::layers::Layer],
    requested: &str,
) -> Result<String, String> {
    let named: Vec<&konnect_sexp::layers::Layer> = declared
        .iter()
        .filter(|layer| layer.name == requested || layer.user_name.as_deref() == Some(requested))
        .collect();
    let layer = match named.as_slice() {
        [layer] => *layer,
        [] => {
            let all: Vec<String> = declared.iter().map(describe_layer).collect();
            return Err(format!(
                "board does not declare layer '{requested}'; declared: {}",
                all.join(", ")
            ));
        }
        several => {
            let all: Vec<String> = several.iter().map(|layer| describe_layer(layer)).collect();
            return Err(format!(
                "'{requested}' names more than one board layer: {}",
                all.join(", ")
            ));
        }
    };
    if let Some(other) = declared.iter().find(|other| {
        other.name != layer.name && other.user_name.as_deref() == Some(layer.name.as_str())
    }) {
        return Err(format!(
            "'{requested}' is layer {}, but layer {} has the user name '{}', and kicad-cli \
             reads user names first; rename that layer in the board setup",
            layer.name, other.name, layer.name
        ));
    }
    Ok(layer.name.clone())
}

// ─── render_library_symbol ───────────────────────────────────────────────────

/// One file kicad-cli wrote for a symbol: `<name>_unit<n>[_demorgan].svg`.
/// The prefix is ignored, so a name KiCad sanitised still matches; the
/// staging directory holds this symbol's files only.
fn symbol_unit_of(path: &Path) -> Option<(u32, bool)> {
    let stem = path.file_stem()?.to_str()?;
    let (stem, demorgan) = match stem.strip_suffix("_demorgan") {
        Some(stem) => (stem, true),
        None => (stem, false),
    };
    let unit = stem.rsplit_once("_unit")?.1.parse().ok()?;
    Some((unit, demorgan))
}

async fn handle_render_library_symbol(
    args: &Value,
    ctx: &ToolContext,
) -> anyhow::Result<CallToolResult> {
    let library = get_path(args, "library")?;
    let Some(symbol) = args.get("name").and_then(Value::as_str) else {
        return Ok(invalid("name", "must be the symbol's name in the library"));
    };
    if symbol.trim().is_empty() {
        return Ok(invalid("name", "must not be empty"));
    }
    let unit = match args.get("unit") {
        None => 1,
        Some(value) => match value.as_u64().and_then(|u| u32::try_from(u).ok()) {
            Some(unit) if unit >= 1 => unit,
            _ => return Ok(invalid("unit", "must be an integer >= 1")),
        },
    };
    let demorgan = match args.get("body_style").map(Value::as_str) {
        None | Some(Some("standard")) => false,
        Some(Some("demorgan")) => true,
        _ => return Ok(invalid("body_style", "must be 'standard' or 'demorgan'")),
    };
    let include_hidden_pins = match flag(args, "include_hidden_pins") {
        Ok(value) => value,
        Err(error) => return Ok(error),
    };
    let delivery = match delivery(args, 800) {
        Ok(delivery) => delivery,
        Err(error) => return Ok(error),
    };
    if !library.is_file() {
        return Ok(not_found(&library, "Symbol library"));
    }

    let staging = tempfile::tempdir()?;
    if let Err(error) = cli::export_symbol_svg(
        &ctx.config.kicad_cli,
        &library,
        symbol,
        staging.path(),
        include_hidden_pins,
    )
    .await
    {
        return Ok(CallToolResult::error(format!(
            "kicad-cli could not export symbol '{symbol}' from {}: {error:#}",
            library.display()
        )));
    }

    let mut files: Vec<(u32, bool, PathBuf)> = std::fs::read_dir(staging.path())?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter_map(|path| symbol_unit_of(&path).map(|(u, d)| (u, d, path)))
        .collect();
    files.sort();
    if files.is_empty() {
        return Ok(CallToolResult::error(format!(
            "kicad-cli wrote no SVG for symbol '{symbol}' in {}; check the name with \
             list_symbols_in_library",
            library.display()
        )));
    }
    let mut units: Vec<u32> = files.iter().map(|(u, _, _)| *u).collect();
    units.dedup();
    let has_demorgan = files.iter().any(|(_, d, _)| *d);
    let Some((_, _, svg_path)) = files.iter().find(|(u, d, _)| *u == unit && *d == demorgan) else {
        return Ok(invalid(
            if units.contains(&unit) {
                "body_style"
            } else {
                "unit"
            },
            format!(
                "symbol '{symbol}' has units {units:?}{}",
                if has_demorgan {
                    " with standard and demorgan body styles"
                } else {
                    " with the standard body style only"
                }
            ),
        ));
    };
    let svg = std::fs::read(svg_path)?;
    respond(
        &delivery,
        render_page(&delivery, &svg, konnect_render::WHITE),
        json!({
            "library": library.display().to_string(),
            "symbol": symbol,
            "unit": unit,
            "body_style": if demorgan { "demorgan" } else { "standard" },
            "units": units,
            "has_demorgan": has_demorgan,
            "include_hidden_pins": include_hidden_pins,
            "background": hex(konnect_render::WHITE),
        }),
        "",
    )
}

// ─── render_library_footprint ────────────────────────────────────────────────

/// KiCad's default user names for the fixed layers whose canonical name
/// differs, as `LayerName()` in KiCad 10's `common/layer_id.cpp` gives them.
/// For a footprint, which has no board and so no renamed layers, kicad-cli
/// reads `--layers` against canonical names and these names only.
const DEFAULT_USER_NAMES: [(&str, &str); 10] = [
    ("F.Adhesive", "F.Adhes"),
    ("B.Adhesive", "B.Adhes"),
    ("F.Silkscreen", "F.SilkS"),
    ("B.Silkscreen", "B.SilkS"),
    ("User.Drawings", "Dwgs.User"),
    ("User.Comments", "Cmts.User"),
    ("User.Eco1", "Eco1.User"),
    ("User.Eco2", "Eco2.User"),
    ("F.Courtyard", "F.CrtYd"),
    ("B.Courtyard", "B.CrtYd"),
];

/// The canonical name of a footprint layer given by its canonical or its
/// default user name. kicad-cli only reports an unknown name and plots the
/// rest, so an unchecked typo would read as "this layer is empty".
fn footprint_layer(requested: &str) -> Option<&str> {
    if konnect_sexp::layers::is_canonical_name(requested) {
        return Some(requested);
    }
    DEFAULT_USER_NAMES
        .iter()
        .find(|(user, _)| *user == requested)
        .map(|(_, canonical)| *canonical)
}

/// A footprint name is joined into paths, so it must be one plain path
/// component: not absolute, no separator, no `..`.
fn is_bare_name(name: &str) -> bool {
    !name.contains(['/', '\\'])
        && !name.contains("..")
        && !Path::new(name).is_absolute()
        && matches!(
            Path::new(name).components().collect::<Vec<_>>().as_slice(),
            [std::path::Component::Normal(_)]
        )
}

async fn handle_render_library_footprint(
    args: &Value,
    ctx: &ToolContext,
) -> anyhow::Result<CallToolResult> {
    let library = get_path(args, "library")?;
    let named = match args.get("name") {
        None => None,
        Some(value) => match value.as_str().map(str::trim) {
            Some(name) if !name.is_empty() => Some(name.to_string()),
            _ => return Ok(invalid("name", "must be a non-empty footprint name")),
        },
    };
    if let Some(name) = named.as_ref().filter(|name| !is_bare_name(name)) {
        return Ok(invalid(
            "name",
            format!("'{name}' is a path; give the footprint's bare name in the library"),
        ));
    }
    let layers = match string_list(args, "layers") {
        Ok(layers) => layers,
        Err(error) => return Ok(error),
    };
    let mut kicad_layers = Vec::with_capacity(layers.len());
    for requested in &layers {
        let Some(canonical) = footprint_layer(requested) else {
            let user_names: Vec<&str> = DEFAULT_USER_NAMES.iter().map(|(user, _)| *user).collect();
            return Ok(invalid(
                "layers",
                format!(
                    "'{requested}' is not a KiCad layer name; accepted: {}, In1.Cu to In30.Cu, \
                     User.1 to User.45, or a default user name: {}",
                    konnect_sexp::layers::FIXED_NAMES.join(", "),
                    user_names.join(", ")
                ),
            ));
        };
        kicad_layers.push(canonical);
    }
    let delivery = match delivery(args, 800) {
        Ok(delivery) => delivery,
        Err(error) => return Ok(error),
    };
    let black_and_white = match flag(args, "black_and_white") {
        Ok(value) => value,
        Err(error) => return Ok(error),
    };

    let staging = tempfile::tempdir()?;
    let single_file = library.extension().and_then(|e| e.to_str()) == Some("kicad_mod");
    let (library_dir, footprint) = if single_file {
        if !library.is_file() {
            return Ok(not_found(&library, "Footprint file"));
        }
        let stem = library
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        if let Some(name) = named.as_ref().filter(|name| **name != stem) {
            return Ok(invalid(
                "name",
                format!("a .kicad_mod file holds footprint '{stem}', not '{name}'"),
            ));
        }
        // kicad-cli reads libraries, not single files: stage a private copy.
        let dir = staging.path().join("staged.pretty");
        std::fs::create_dir(&dir)?;
        std::fs::copy(&library, dir.join(format!("{stem}.kicad_mod")))?;
        (dir, stem)
    } else {
        if !library.is_dir() {
            return Ok(not_found(&library, "Footprint library"));
        }
        let Some(name) = named else {
            return Ok(invalid(
                "name",
                "is required for a .pretty library; list names with list_library_footprints",
            ));
        };
        let file = library.join(format!("{name}.kicad_mod"));
        if !file.is_file() {
            return Ok(not_found(&file, "Footprint"));
        }
        (library.clone(), name)
    };

    let out_dir = staging.path().join("svg");
    std::fs::create_dir(&out_dir)?;
    if let Err(error) = cli::export_footprint_svg(
        &ctx.config.kicad_cli,
        &library_dir,
        &footprint,
        &out_dir,
        &kicad_layers,
        black_and_white,
    )
    .await
    {
        return Ok(CallToolResult::error(format!(
            "kicad-cli could not export footprint '{footprint}': {error:#}"
        )));
    }
    // kicad-cli exits 0 without writing when it cannot find the footprint,
    // so the file itself is the evidence.
    let svg_path = out_dir.join(format!("{footprint}.svg"));
    let svg = match std::fs::read(&svg_path) {
        Ok(svg) if !svg.is_empty() => svg,
        _ => {
            return Ok(CallToolResult::error(format!(
                "kicad-cli reported success but wrote no SVG for footprint '{footprint}'"
            )))
        }
    };
    let paper = background(black_and_white);
    respond(
        &delivery,
        render_page(&delivery, &svg, paper),
        json!({
            "library": library.display().to_string(),
            "footprint": footprint,
            "layers": if layers.is_empty() { json!("kicad_default") } else { json!(layers) },
            "background": hex(paper),
            "black_and_white": black_and_white,
        }),
        "",
    )
}

#[cfg(test)]
#[path = "item_render_tests.rs"]
mod tests;
