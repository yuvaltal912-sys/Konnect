//! Rank 25: board-layer and library-item renders reach the client as image
//! blocks, keep their size caps, and refuse instead of guessing.
//!
//! A stand-in kicad-cli serves SVGs that the real kicad-cli 10.0.8 exported
//! from the fixtures beside them, so the rasterization runs on real KiCad
//! output while the arguments KiCad receives stay checkable.

use super::*;
use crate::mcp::protocol::ToolContent;
use base64::Engine as _;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/item_render");

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(Path::new(FIXTURES).join(name)).unwrap()
}

struct Scene {
    dir: tempfile::TempDir,
    board: PathBuf,
    symbols: PathBuf,
    footprints: PathBuf,
    ctx: ToolContext,
}

impl Scene {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let served = dir.path().join("served");
        std::fs::create_dir(&served).unwrap();
        for name in [
            "board_area.svg",
            "board_sheet.svg",
            "DUAL_unit1.svg",
            "DUAL_unit2.svg",
            "TEST_PAD2.svg",
        ] {
            std::fs::write(served.join(name), fixture(name)).unwrap();
        }
        let board = dir.path().join("board.kicad_pcb");
        std::fs::copy(
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/specctra_two_resistors.kicad_pcb"
            ),
            &board,
        )
        .unwrap();
        let symbols = dir.path().join("konnect_test.kicad_sym");
        std::fs::write(&symbols, fixture("konnect_test.kicad_sym")).unwrap();
        let footprints = dir.path().join("konnect_test.pretty");
        std::fs::create_dir(&footprints).unwrap();
        std::fs::write(
            footprints.join("TEST_PAD2.kicad_mod"),
            fixture("konnect_test.pretty/TEST_PAD2.kicad_mod"),
        )
        .unwrap();
        // Behaviour switches are files in the script's directory: `fail`
        // exits 3, `empty` succeeds without writing (what kicad-cli does for
        // a footprint it cannot find), `visible_text` serves a font-dependent SVG.
        let cli = super::super::cli::test_support::write_script(
            &served,
            "fake-kicad-cli",
            r##"#!/bin/sh
d=$(dirname "$0")
printf '%s\n' "$@" > "$d/argv.txt"
[ -f "$d/fail" ] && { echo "Failed to load" >&2; exit 3; }
[ -f "$d/empty" ] && exit 0
out=""; mode=""; sym=""; prev=""
for a in "$@"; do
  [ "$prev" = "--output" ] && out="$a"
  [ "$prev" = "--page-size-mode" ] && mode="$a"
  [ "$prev" = "--symbol" ] && sym="$a"
  prev="$a"
done
if [ -f "$d/visible_text" ]; then
  svg='<svg xmlns="http://www.w3.org/2000/svg" width="10mm" height="10mm"><text x="1" y="8">R1</text></svg>'
  case "$1" in
    pcb) printf '%s' "$svg" > "$out" ;;
    sym) printf '%s' "$svg" > "$out/DUAL_unit1.svg" ;;
    fp) printf '%s' "$svg" > "$out/TEST_PAD2.svg" ;;
  esac
  exit 0
fi
case "$1" in
  pcb) if [ "$mode" = "1" ]; then cp "$d/board_sheet.svg" "$out"; else cp "$d/board_area.svg" "$out"; fi ;;
  sym) mkdir -p "$out"
       # KiCad 10 names every unit <name>_unit<n>.svg, a single unit too
       # (doSymExportSvg in eeschema_jobs_handler.cpp). For one-unit SINGLE
       # the stand-in serves DUAL's unit-1 drawing: only the name is tested.
       if [ "$sym" = "SINGLE" ]; then cp "$d/DUAL_unit1.svg" "$out/SINGLE_unit1.svg"
       else cp "$d"/DUAL_unit1.svg "$d"/DUAL_unit2.svg "$out"/; fi ;;
  fp) cp "$d/TEST_PAD2.svg" "$out/" ;;
esac
"##,
            "",
        );
        let ctx = ToolContext::new(
            super::super::ServerConfig {
                kicad_cli: cli.to_string_lossy().into_owned(),
                ..Default::default()
            },
            std::sync::Arc::new(crate::router::ToolRouter::new()),
        );
        Self {
            dir,
            board,
            symbols,
            footprints,
            ctx,
        }
    }

    fn served(&self) -> PathBuf {
        self.dir.path().join("served")
    }

    fn switch(&self, name: &str) {
        std::fs::write(self.served().join(name), "").unwrap();
    }

    /// Arguments of the last kicad-cli call, or None when it never ran.
    fn argv(&self) -> Option<Vec<String>> {
        let text = std::fs::read_to_string(self.served().join("argv.txt")).ok()?;
        Some(text.lines().map(str::to_owned).collect())
    }

    fn value_after(&self, flag: &str) -> Option<String> {
        let argv = self.argv()?;
        let index = argv.iter().position(|arg| arg == flag)?;
        argv.get(index + 1).cloned()
    }

    async fn board(&self, mut args: Value) -> CallToolResult {
        args["board"] = json!(self.board);
        handle_render_board_layers(&args, &self.ctx).await.unwrap()
    }

    async fn symbol(&self, mut args: Value) -> CallToolResult {
        args["library"] = json!(self.symbols);
        handle_render_library_item(&args, &self.ctx).await.unwrap()
    }

    async fn footprint(&self, mut args: Value) -> CallToolResult {
        if args.get("library").is_none() {
            args["library"] = json!(self.footprints);
        }
        handle_render_library_item(&args, &self.ctx).await.unwrap()
    }
}

fn text_of(result: &CallToolResult) -> &str {
    let ToolContent::Text { text } = &result.content[0] else {
        panic!("first block is text")
    };
    text
}

fn metadata(result: &CallToolResult) -> Value {
    assert!(!result.is_error, "{}", text_of(result));
    serde_json::from_str(text_of(result)).unwrap()
}

/// The image block's decoded PNG; asserts the result is metadata + image.
fn image(result: &CallToolResult) -> Vec<u8> {
    assert_eq!(result.content.len(), 2, "metadata plus one image block");
    let ToolContent::Image { data, mime_type } = &result.content[1] else {
        panic!("second block is an image")
    };
    assert_eq!(mime_type, "image/png");
    let png = base64::engine::general_purpose::STANDARD
        .decode(data)
        .unwrap();
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    png
}

fn refusal(result: &CallToolResult) -> String {
    assert!(result.is_error, "expected a refusal: {}", text_of(result));
    text_of(result).to_string()
}

// ─── render_board_layers ─────────────────────────────────────────────────────

#[cfg(unix)]
#[tokio::test]
async fn board_layers_render_inline_framed_on_the_board_area() {
    let scene = Scene::new();
    let result = scene
        .board(json!({"layers": ["F.Cu", "Edge.Cuts"], "width_px": 400}))
        .await;
    let meta = metadata(&result);
    let png = image(&result);
    let expected = konnect_render::svg_page_to_png(
        &fixture("board_area.svg"),
        400,
        konnect_render::BOARD_BACKGROUND,
    )
    .unwrap();
    assert_eq!(png, expected.png);
    assert_eq!(meta["width_px"], 400);
    assert_eq!(meta["height_px"], expected.height_px);
    assert_eq!(meta["png_bytes"], png.len());
    assert_eq!(meta["framing"]["kind"], "board_area");
    assert_eq!(meta["background"], "#001023");
    assert!(meta.get("rendered").is_none(), "nothing written by default");

    assert_eq!(
        scene.value_after("--layers").as_deref(),
        Some("F.Cu,Edge.Cuts")
    );
    assert_eq!(scene.value_after("--page-size-mode").as_deref(), Some("2"));
    let argv = scene.argv().unwrap();
    assert!(argv.contains(&"--exclude-drawing-sheet".to_string()));
    assert!(argv.contains(&"--mode-single".to_string()));
    assert!(!argv.contains(&"--black-and-white".to_string()));
    // Colours come from KiCad's built-in default theme, not this machine's.
    assert_eq!(
        scene.value_after("--theme").as_deref(),
        Some("_builtin_default")
    );
    assert_eq!(argv.last(), Some(&scene.board.display().to_string()));
    // The SVG went to a private directory, never next to the board.
    let svg_out = PathBuf::from(scene.value_after("--output").unwrap());
    assert_ne!(svg_out.parent(), scene.board.parent());
    assert!(!svg_out.exists(), "staging is removed after the call");
}

#[cfg(unix)]
#[tokio::test]
async fn board_region_crops_in_board_coordinates_at_the_requested_width() {
    let scene = Scene::new();
    let region = json!({"x": 80, "y": 30, "width": 50, "height": 40});
    let result = scene
        .board(
            json!({"layers": ["F.Cu", "F.SilkS", "Edge.Cuts"], "width_px": 500, "region": region}),
        )
        .await;
    let meta = metadata(&result);
    let expected = konnect_render::svg_region_to_png_on(
        &fixture("board_sheet.svg"),
        500,
        konnect_render::Region {
            x_mm: 80.0,
            y_mm: 30.0,
            width_mm: 50.0,
            height_mm: 40.0,
        },
        konnect_render::BOARD_BACKGROUND,
    )
    .unwrap();
    assert_eq!(image(&result), expected.png);
    assert_eq!(
        (meta["width_px"].clone(), meta["height_px"].clone()),
        (json!(500), json!(400))
    );
    assert_eq!(meta["framing"]["kind"], "region");
    assert_eq!(meta["framing"]["x_mm"], 80.0);
    assert_eq!(scene.value_after("--page-size-mode").as_deref(), Some("1"));
    // The crop holds the board, not empty background.
    let background = konnect_render::svg_region_to_png_on(
        br#"<svg xmlns="http://www.w3.org/2000/svg" width="297mm" height="210mm"/>"#,
        500,
        konnect_render::Region {
            x_mm: 80.0,
            y_mm: 30.0,
            width_mm: 50.0,
            height_mm: 40.0,
        },
        konnect_render::BOARD_BACKGROUND,
    )
    .unwrap();
    let drawn = konnect_render::diff_pngs(&background.png, &expected.png).unwrap();
    assert!(drawn.changed_pixels > 500, "{drawn:?}");
}

#[cfg(unix)]
#[tokio::test]
async fn board_black_and_white_reaches_kicad_and_uses_white_paper() {
    let scene = Scene::new();
    let meta = metadata(
        &scene
            .board(json!({"layers": ["F.Cu"], "width_px": 100, "black_and_white": true}))
            .await,
    );
    assert_eq!(meta["background"], "#ffffff");
    let argv = scene.argv().unwrap();
    assert!(argv.contains(&"--black-and-white".to_string()));
    assert!(!argv.contains(&"--theme".to_string()), "{argv:?}");
}

#[cfg(unix)]
#[tokio::test]
async fn board_output_file_and_text_only_delivery() {
    let scene = Scene::new();
    let output = scene.dir.path().join("renders/top.png");
    let result = scene
        .board(json!({"layers": ["F.Cu"], "width_px": 120, "inline": false, "output": output}))
        .await;
    assert_eq!(
        result.content.len(),
        1,
        "no image block when inline is false"
    );
    let meta = metadata(&result);
    let written = std::fs::read(&output).unwrap();
    assert_eq!(meta["rendered"], json!(output.display().to_string()));
    assert_eq!(meta["png_bytes"], written.len());

    let both = scene
        .board(json!({"layers": ["F.Cu"], "width_px": 120, "output": output}))
        .await;
    assert_eq!(image(&both), std::fs::read(&output).unwrap());
}

#[cfg(unix)]
#[tokio::test]
async fn board_refuses_bad_arguments_before_running_kicad() {
    let scene = Scene::new();
    for (args, field) in [
        (json!({"layers": []}), "layers"),
        (json!({"layers": ["F.Cu", 3]}), "layers[1]"),
        (json!({"layers": ["In7.Cu"]}), "layers"),
        (json!({"layers": ["F.Cu"], "width_px": 4097}), "width_px"),
        (json!({"layers": ["F.Cu"], "width_px": 0}), "width_px"),
        (json!({"layers": ["F.Cu"], "inline": false}), "output"),
        (
            json!({"layers": ["F.Cu"], "region": {"x": -1, "y": 0, "width": 5, "height": 5}}),
            "region",
        ),
    ] {
        let message = refusal(&scene.board(args.clone()).await);
        assert!(
            message.contains(&format!("'{field}'")) || message.contains(&format!("\"{field}\"")),
            "{args}: {message}"
        );
    }
    let unknown = refusal(&scene.board(json!({"layers": ["In7.Cu"]})).await);
    assert!(
        unknown.contains("does not declare layer 'In7.Cu'"),
        "{unknown}"
    );
    assert!(
        unknown.contains("Edge.Cuts") && unknown.contains("F.SilkS (F.Silkscreen)"),
        "lists the declared layers with their user names: {unknown}"
    );
    assert!(scene.argv().is_none(), "kicad-cli never ran");

    let missing = handle_render_board_layers(
        &json!({"board": scene.dir.path().join("nope.kicad_pcb"), "layers": ["F.Cu"]}),
        &scene.ctx,
    )
    .await
    .unwrap();
    assert!(refusal(&missing).contains("file_not_found"));
}

#[cfg(unix)]
#[tokio::test]
async fn board_layers_accept_user_names_and_pass_kicad_the_canonical_name() {
    let scene = Scene::new();
    let meta = metadata(
        &scene
            .board(json!({"layers": ["F.Silkscreen", "F.Courtyard", "Edge.Cuts"], "width_px": 200}))
            .await,
    );
    assert_eq!(
        scene.value_after("--layers").as_deref(),
        Some("F.SilkS,F.CrtYd,Edge.Cuts")
    );
    // The metadata reports what was asked.
    assert_eq!(
        meta["layers"],
        json!(["F.Silkscreen", "F.Courtyard", "Edge.Cuts"])
    );
}

#[cfg(unix)]
#[tokio::test]
async fn board_layer_names_kicad_would_read_as_another_layer_are_refused() {
    let scene = Scene::new();
    // F.CrtYd renamed to "F.SilkS": kicad-cli reads user names first, so
    // "F.SilkS" on its command line would plot the courtyard.
    let source = std::fs::read_to_string(&scene.board).unwrap();
    let renamed = source.replace(
        r#"(31 "F.CrtYd" user "F.Courtyard")"#,
        r#"(31 "F.CrtYd" user "F.SilkS")"#,
    );
    assert_ne!(renamed, source);
    std::fs::write(&scene.board, renamed).unwrap();

    let both = refusal(&scene.board(json!({"layers": ["F.SilkS"]})).await);
    assert!(
        both.contains("more than one board layer") && both.contains("F.CrtYd (F.SilkS)"),
        "{both}"
    );
    let shadowed = refusal(&scene.board(json!({"layers": ["F.Silkscreen"]})).await);
    assert!(
        shadowed.contains("layer F.CrtYd has the user name 'F.SilkS'"),
        "{shadowed}"
    );
    assert!(scene.argv().is_none(), "kicad-cli never ran");

    metadata(
        &scene
            .board(json!({"layers": ["F.CrtYd"], "width_px": 100}))
            .await,
    );
    assert_eq!(scene.value_after("--layers").as_deref(), Some("F.CrtYd"));
}

#[cfg(unix)]
#[tokio::test]
async fn inline_renders_over_the_client_limits_are_refused_and_files_are_named() {
    let scene = Scene::new();
    // A 1 x 40.5 mm sliver at 200 px is 200 x 8100 px: inside the 8192 px
    // render cap, over the 8000 px inline limit.
    let sliver = json!({"x": 80, "y": 30, "width": 1, "height": 40.5});
    let tall = refusal(
        &scene
            .board(json!({"layers": ["F.Cu"], "width_px": 200, "region": sliver}))
            .await,
    );
    assert!(tall.contains("200 x 8100 px"), "{tall}");
    assert!(tall.contains("8000 px per side"), "{tall}");
    assert!(tall.contains("about 197 or less"), "{tall}");
    assert!(
        tall.contains("'region'") && tall.contains("'inline' false"),
        "{tall}"
    );
    assert_eq!(tall.matches("written").count(), 0, "{tall}");

    // A file asked for is still written, and the refusal says where.
    let output = scene.dir.path().join("renders/sliver.png");
    let named = refusal(
        &scene
            .board(json!({"layers": ["F.Cu"], "width_px": 200, "region": sliver, "output": output}))
            .await,
    );
    assert!(
        named.contains(&format!("still written to {}", output.display())),
        "{named}"
    );
    assert!(output.is_file());

    // File-only output keeps the 8192 px render cap.
    let file_only = metadata(
        &scene
            .board(json!({"layers": ["F.Cu"], "width_px": 200, "region": sliver, "inline": false, "output": output}))
            .await,
    );
    assert_eq!(file_only["height_px"], 8100);
}

#[cfg(unix)]
#[tokio::test]
async fn board_region_off_the_sheet_and_oversized_output_are_refused() {
    let scene = Scene::new();
    let off_sheet = refusal(
        &scene
            .board(json!({"layers": ["F.Cu"], "region": {"x": 290, "y": 0, "width": 20, "height": 10}}))
            .await,
    );
    assert!(off_sheet.contains("outside the SVG page"), "{off_sheet}");
    assert!(off_sheet.contains("render without 'region'"), "{off_sheet}");

    // A sliver 4096 px wide would need 40,960 px of height: refused, not allocated.
    let tall = refusal(
        &scene
            .board(json!({"layers": ["F.Cu"], "width_px": 4096, "region": {"x": 80, "y": 30, "width": 1, "height": 10}}))
            .await,
    );
    assert!(tall.contains("height exceeds 8192"), "{tall}");
    assert!(
        !tall.contains("render without 'region'"),
        "hint only for off-sheet: {tall}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn visible_text_is_refused_for_every_render_kind() {
    let scene = Scene::new();
    scene.switch("visible_text");
    for result in [
        scene.board(json!({"layers": ["F.Cu"]})).await,
        scene.symbol(json!({"name": "DUAL"})).await,
        scene.footprint(json!({"name": "TEST_PAD2"})).await,
    ] {
        let message = refusal(&result);
        assert!(message.contains("visible <text>"), "{message}");
        assert_eq!(result.content.len(), 1, "no image on refusal");
    }
}

#[cfg(unix)]
#[tokio::test]
async fn a_failed_board_export_is_an_error_not_an_empty_picture() {
    let scene = Scene::new();
    scene.switch("empty");
    let err = handle_render_board_layers(
        &json!({"board": scene.board, "layers": ["F.Cu"]}),
        &scene.ctx,
    )
    .await
    .unwrap_err();
    assert!(format!("{err:#}").contains("did not create"), "{err:#}");
}

// ─── render_library_symbol ───────────────────────────────────────────────────

#[cfg(unix)]
#[tokio::test]
async fn symbol_unit_renders_inline_and_lists_the_units() {
    let scene = Scene::new();
    let result = scene.symbol(json!({"name": "DUAL"})).await;
    let meta = metadata(&result);
    let expected =
        konnect_render::svg_page_to_png(&fixture("DUAL_unit1.svg"), 800, konnect_render::WHITE)
            .unwrap();
    assert_eq!(image(&result), expected.png);
    assert_eq!(meta["unit"], 1);
    assert_eq!(meta["units"], json!([1, 2]));
    assert_eq!(meta["has_demorgan"], false);
    assert_eq!(meta["height_px"], expected.height_px);
    assert_eq!(scene.value_after("--symbol").as_deref(), Some("DUAL"));
    let argv = scene.argv().unwrap();
    assert_eq!(argv.last(), Some(&scene.symbols.display().to_string()));
    assert!(!argv.contains(&"--include-hidden-pins".to_string()));

    let second = scene
        .symbol(json!({"name": "DUAL", "unit": 2, "width_px": 300, "include_hidden_pins": true}))
        .await;
    assert_eq!(
        image(&second),
        konnect_render::svg_page_to_png(&fixture("DUAL_unit2.svg"), 300, konnect_render::WHITE)
            .unwrap()
            .png
    );
    assert!(scene
        .argv()
        .unwrap()
        .contains(&"--include-hidden-pins".to_string()));
}

#[cfg(unix)]
#[tokio::test]
async fn symbol_refuses_a_unit_or_body_style_it_does_not_have() {
    let scene = Scene::new();
    let unit = refusal(&scene.symbol(json!({"name": "DUAL", "unit": 3})).await);
    assert!(unit.contains("'unit'") && unit.contains("[1, 2]"), "{unit}");
    let style = refusal(
        &scene
            .symbol(json!({"name": "DUAL", "body_style": "demorgan"}))
            .await,
    );
    assert!(
        style.contains("'body_style'") && style.contains("standard body style only"),
        "{style}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn symbol_export_failures_are_reported() {
    let scene = Scene::new();
    let missing = handle_render_library_item(
        &json!({"library": scene.dir.path().join("none.kicad_sym"), "name": "DUAL"}),
        &scene.ctx,
    )
    .await
    .unwrap();
    assert!(refusal(&missing).contains("file_not_found"));
    assert!(scene.argv().is_none(), "kicad-cli never ran");
    assert!(refusal(&scene.symbol(json!({"name": " "})).await).contains("'name'"));

    scene.switch("fail");
    let failed = refusal(&scene.symbol(json!({"name": "NOPE"})).await);
    assert!(
        failed.contains("could not export symbol 'NOPE'"),
        "{failed}"
    );
    std::fs::remove_file(scene.served().join("fail")).unwrap();
    scene.switch("empty");
    let empty = refusal(&scene.symbol(json!({"name": "DUAL"})).await);
    assert!(empty.contains("wrote no SVG"), "{empty}");
}

#[cfg(unix)]
#[tokio::test]
async fn a_single_unit_symbol_is_found_by_kicads_unit1_file_name() {
    let scene = Scene::new();
    let result = scene.symbol(json!({"name": "SINGLE"})).await;
    let meta = metadata(&result);
    assert_eq!(
        image(&result),
        konnect_render::svg_page_to_png(&fixture("DUAL_unit1.svg"), 800, konnect_render::WHITE)
            .unwrap()
            .png
    );
    assert_eq!(meta["symbol"], "SINGLE");
    assert_eq!(meta["unit"], 1);
    assert_eq!(meta["units"], json!([1]));
    assert_eq!(meta["has_demorgan"], false);
    let unit = refusal(&scene.symbol(json!({"name": "SINGLE", "unit": 2})).await);
    assert!(unit.contains("'unit'") && unit.contains("[1]"), "{unit}");
}

// ─── render_library_footprint ────────────────────────────────────────────────

#[cfg(unix)]
#[tokio::test]
async fn footprint_layers_are_checked_against_kicads_layer_names() {
    let scene = Scene::new();
    let typo = refusal(
        &scene
            .footprint(json!({"name": "TEST_PAD2", "layers": ["F.Cu", "F.Silk"]}))
            .await,
    );
    assert!(
        typo.contains("'layers'") && typo.contains("'F.Silk' is not a KiCad layer name"),
        "{typo}"
    );
    assert!(
        typo.contains("F.SilkS") && typo.contains("F.Silkscreen"),
        "lists the accepted names: {typo}"
    );
    assert!(scene.argv().is_none(), "kicad-cli never ran");

    // KiCad's default user names are accepted and passed on canonically.
    let meta = metadata(
        &scene
            .footprint(json!({"name": "TEST_PAD2", "layers": ["F.Cu", "F.Silkscreen", "F.Courtyard", "User.Eco1"]}))
            .await,
    );
    assert_eq!(
        scene.value_after("--layers").as_deref(),
        Some("F.Cu,F.SilkS,F.CrtYd,Eco1.User")
    );
    assert_eq!(
        meta["layers"],
        json!(["F.Cu", "F.Silkscreen", "F.Courtyard", "User.Eco1"])
    );
}

#[cfg(unix)]
#[tokio::test]
async fn a_footprint_name_that_is_a_path_is_refused_before_any_join() {
    let scene = Scene::new();
    // A stale SVG beside a footprint outside the library: an absolute name
    // would find both, and kicad-cli, writing nothing, would leave the stale
    // file to be returned as the render.
    let elsewhere = scene.dir.path().join("elsewhere");
    std::fs::create_dir(&elsewhere).unwrap();
    std::fs::write(
        elsewhere.join("TEST_PAD2.kicad_mod"),
        fixture("konnect_test.pretty/TEST_PAD2.kicad_mod"),
    )
    .unwrap();
    std::fs::write(elsewhere.join("TEST_PAD2.svg"), fixture("TEST_PAD2.svg")).unwrap();
    scene.switch("empty");
    for name in [
        elsewhere.join("TEST_PAD2").display().to_string(),
        "../konnect_test.pretty/TEST_PAD2".to_string(),
        "..".to_string(),
        "a\\b".to_string(),
    ] {
        let message = refusal(&scene.footprint(json!({ "name": name })).await);
        assert!(
            message.contains("'name'") && message.contains("is a path"),
            "{name}: {message}"
        );
    }
    assert!(scene.argv().is_none(), "kicad-cli never ran");
}

#[cfg(unix)]
#[tokio::test]
async fn footprint_from_a_pretty_library_renders_inline() {
    let scene = Scene::new();
    let result = scene
        .footprint(json!({"name": "TEST_PAD2", "layers": ["F.Cu", "F.SilkS"], "width_px": 600}))
        .await;
    let meta = metadata(&result);
    let expected = konnect_render::svg_page_to_png(
        &fixture("TEST_PAD2.svg"),
        600,
        konnect_render::BOARD_BACKGROUND,
    )
    .unwrap();
    assert_eq!(image(&result), expected.png);
    assert_eq!(meta["footprint"], "TEST_PAD2");
    assert_eq!(meta["layers"], json!(["F.Cu", "F.SilkS"]));
    assert_eq!(
        scene.value_after("--footprint").as_deref(),
        Some("TEST_PAD2")
    );
    assert_eq!(
        scene.value_after("--layers").as_deref(),
        Some("F.Cu,F.SilkS")
    );
    assert_eq!(
        scene.argv().unwrap().last(),
        Some(&scene.footprints.display().to_string())
    );
    assert_eq!(
        scene.value_after("--theme").as_deref(),
        Some("_builtin_default")
    );

    let plain = scene
        .footprint(json!({"name": "TEST_PAD2", "black_and_white": true}))
        .await;
    assert_eq!(metadata(&plain)["layers"], "kicad_default");
    let argv = scene.argv().unwrap();
    assert!(!argv.contains(&"--layers".to_string()));
    assert!(argv.contains(&"--black-and-white".to_string()));
    assert!(!argv.contains(&"--theme".to_string()), "{argv:?}");
}

#[cfg(unix)]
#[tokio::test]
async fn a_single_kicad_mod_file_is_staged_as_a_private_library() {
    let scene = Scene::new();
    let file = scene.footprints.join("TEST_PAD2.kicad_mod");
    let result = scene.footprint(json!({"library": file})).await;
    assert_eq!(metadata(&result)["footprint"], "TEST_PAD2");
    image(&result);
    let staged = PathBuf::from(scene.argv().unwrap().last().unwrap());
    assert_ne!(staged, scene.footprints, "kicad-cli read a private copy");
    assert_eq!(staged.extension().and_then(|e| e.to_str()), Some("pretty"));

    let mismatch = refusal(
        &scene
            .footprint(json!({"library": file, "name": "OTHER"}))
            .await,
    );
    assert!(
        mismatch.contains("holds footprint 'TEST_PAD2'"),
        "{mismatch}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn footprint_refusals_name_what_is_missing() {
    let scene = Scene::new();
    assert!(refusal(&scene.footprint(json!({})).await).contains("list_library_footprints"));
    let missing = refusal(&scene.footprint(json!({"name": "NOPE"})).await);
    assert!(missing.contains("NOPE.kicad_mod"), "{missing}");
    let no_dir = refusal(
        &scene
            .footprint(json!({"library": scene.dir.path().join("x.pretty"), "name": "A"}))
            .await,
    );
    assert!(no_dir.contains("file_not_found"), "{no_dir}");
    assert!(scene.argv().is_none(), "kicad-cli never ran");

    // kicad-cli exits 0 and writes nothing for a footprint it cannot load.
    scene.switch("empty");
    let empty = refusal(&scene.footprint(json!({"name": "TEST_PAD2"})).await);
    assert!(empty.contains("wrote no SVG"), "{empty}");
}

#[cfg(unix)]
#[tokio::test]
async fn library_kind_comes_from_the_path_and_foreign_options_are_refused() {
    let scene = Scene::new();
    let wrong_type = refusal(
        &handle_render_library_item(&json!({"library": scene.board, "name": "R1"}), &scene.ctx)
            .await
            .unwrap(),
    );
    assert!(wrong_type.contains("'library'"), "{wrong_type}");
    for (args, field) in [
        (json!({"name": "DUAL", "layers": ["F.Cu"]}), "layers"),
        (
            json!({"name": "DUAL", "black_and_white": true}),
            "black_and_white",
        ),
    ] {
        let message = refusal(&scene.symbol(args).await);
        assert!(
            message.contains(&format!("'{field}'")) && message.contains("symbol"),
            "{message}"
        );
    }
    for (args, field) in [
        (json!({"name": "TEST_PAD2", "unit": 2}), "unit"),
        (
            json!({"name": "TEST_PAD2", "body_style": "demorgan"}),
            "body_style",
        ),
        (
            json!({"name": "TEST_PAD2", "include_hidden_pins": true}),
            "include_hidden_pins",
        ),
    ] {
        let message = refusal(&scene.footprint(args).await);
        assert!(
            message.contains(&format!("'{field}'")) && message.contains("footprint"),
            "{message}"
        );
    }
    assert!(scene.argv().is_none(), "kicad-cli never ran");
}

#[cfg(unix)]
#[tokio::test]
async fn foreign_options_at_null_or_their_default_are_accepted() {
    let scene = Scene::new();
    metadata(
        &scene
            .footprint(json!({"name": "TEST_PAD2", "unit": 1, "body_style": "standard", "include_hidden_pins": false}))
            .await,
    );
    assert!(scene.argv().is_some(), "kicad-cli ran");
    metadata(
        &scene
            .footprint(json!({"name": "TEST_PAD2", "unit": null, "body_style": null}))
            .await,
    );
    metadata(
        &scene
            .symbol(json!({"name": "DUAL", "black_and_white": false, "layers": null}))
            .await,
    );
    // `layers` documents no literal default, so a list is still refused.
    let listed = refusal(&scene.symbol(json!({"name": "DUAL", "layers": []})).await);
    assert!(listed.contains("'layers'"), "{listed}");
}

#[test]
fn an_inline_page_refusal_names_a_width_that_fits_inline() {
    // A 1 x 80 pin header's page is about 5.7 x 206 mm: 226 px fits the
    // 8192 px render cap (8168 px tall) but not the 8000 px inline side.
    let header = br##"<svg xmlns="http://www.w3.org/2000/svg" width="5.7mm" height="206mm" viewBox="0 0 5.7 206"><rect width="5.7" height="206"/></svg>"##;
    let at = |width_px, inline| Delivery {
        inline,
        output: None,
        width_px,
    };
    let err = render_page(&at(800, true), header, konnect_render::WHITE)
        .unwrap_err()
        .to_string();
    assert!(err.contains("inline limit of 8000 px"), "{err}");
    assert!(err.contains("fits inline at width_px 221 or less"), "{err}");
    assert!(!err.contains("region"), "{err}");
    let err = render_page(&at(226, true), header, konnect_render::WHITE)
        .unwrap_err()
        .to_string();
    assert!(err.contains("width_px 221 or less"), "{err}");
    let fits = render_page(&at(221, true), header, konnect_render::WHITE).unwrap();
    assert_eq!(fits.height_px, 7987);
    // A file-only render keeps the 8192 px render cap.
    let file = render_page(&at(226, false), header, konnect_render::WHITE).unwrap();
    assert_eq!(file.height_px, 8168);
    let err = render_page(&at(800, false), header, konnect_render::WHITE)
        .unwrap_err()
        .to_string();
    assert!(err.contains("width_px 226 or less"), "{err}");
}

// ─── Registration ────────────────────────────────────────────────────────────

#[test]
fn render_tools_are_registered_beside_their_families() {
    let pcb: Vec<&str> = super::super::pcb_export::tools()
        .iter()
        .map(|t| t.name)
        .collect();
    let at = pcb.iter().position(|n| *n == "export_svg").unwrap();
    assert_eq!(pcb[at + 1], "render_board_layers", "{pcb:?}");
    let library: Vec<&str> = super::super::library::tools()
        .iter()
        .map(|t| t.name)
        .collect();
    assert!(library.contains(&"render_library_item"), "{library:?}");
}
