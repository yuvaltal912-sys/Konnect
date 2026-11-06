//! Creation PR 4: board camera options reach KiCad, image bytes reach the
//! client, and rendering never overwrites a neighbouring project artifact.

use super::*;
#[cfg(unix)]
use crate::mcp::protocol::ToolContent;
#[cfg(unix)]
use base64::Engine;
#[cfg(unix)]
use serde_json::Value;

#[cfg(unix)]
struct Scene {
    dir: tempfile::TempDir,
    board: PathBuf,
    argv: PathBuf,
    png: Vec<u8>,
    ctx: ToolContext,
}

#[cfg(unix)]
impl Scene {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let board = dir.path().join("board.kicad_pcb");
        std::fs::copy(
            concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/test.kicad_pcb"),
            &board,
        )
        .unwrap();
        let png = konnect_render::svg_to_png(
            br##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="10" height="10" fill="#112233"/></svg>"##,
            10,
        )
        .unwrap()
        .png;
        let image = dir.path().join("served.png");
        std::fs::write(&image, &png).unwrap();
        let argv = dir.path().join("argv.txt");
        let quote = |path: &Path| format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"));
        let cli = super::super::cli::test_support::write_script(
            dir.path(),
            "fake-kicad-cli",
            &format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > {}\ncp {} \"$4\"\n",
                quote(&argv),
                quote(&image),
            ),
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
            argv,
            png,
            ctx,
        }
    }

    async fn render(&self, mut args: Value) -> CallToolResult {
        args["board"] = json!(self.board);
        handle_get_board_2d_view(&args, &self.ctx).await.unwrap()
    }

    fn args(&self) -> Vec<String> {
        std::fs::read_to_string(&self.argv)
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect()
    }
}

#[cfg(unix)]
fn flag<'a>(args: &'a [String], name: &str) -> &'a str {
    let position = args.iter().position(|value| value == name).unwrap();
    &args[position + 1]
}

#[cfg(unix)]
fn assert_invalid(result: &CallToolResult, field: &str) {
    assert!(result.is_error, "{result:?}");
    let ToolContent::Text { text } = &result.content[0] else {
        panic!("expected an argument error");
    };
    let body: Value = serde_json::from_str(text).unwrap();
    assert_eq!(body["error"]["kind"], "invalid_argument");
    assert_eq!(body["error"]["field"], field);
}

#[cfg(unix)]
#[tokio::test]
async fn board_render_defaults_stay_an_image_and_fit_the_top_side() {
    let scene = Scene::new();
    let result = scene.render(json!({})).await;
    assert!(!result.is_error, "{result:?}");
    assert_eq!(result.content.len(), 1);
    let ToolContent::Image { data, mime_type } = &result.content[0] else {
        panic!("board render must be an image block");
    };
    assert_eq!(mime_type, "image/png");
    assert_eq!(
        base64::engine::general_purpose::STANDARD
            .decode(data)
            .unwrap(),
        scene.png
    );
    let args = scene.args();
    assert_eq!(&args[..2], ["pcb", "render"]);
    assert_eq!(flag(&args, "--width"), "800");
    assert_eq!(flag(&args, "--height"), "600");
    assert_eq!(flag(&args, "--side"), "top");
    assert_eq!(flag(&args, "--zoom"), "1");
    assert_eq!(args.last().unwrap(), &scene.board.to_string_lossy());
    assert!(!args.iter().any(|arg| arg == "--layers"));
}

#[cfg(unix)]
#[tokio::test]
async fn board_render_forwards_bottom_and_zoom_and_preserves_size_clamping() {
    let scene = Scene::new();
    let result = scene
        .render(json!({"side": "bottom", "zoom": 2.5, "width": 7, "height": 9000}))
        .await;
    assert!(!result.is_error, "{result:?}");
    let args = scene.args();
    assert_eq!(flag(&args, "--side"), "bottom");
    assert_eq!(flag(&args, "--zoom"), "2.5");
    assert_eq!(flag(&args, "--width"), "100");
    assert_eq!(flag(&args, "--height"), "4000");
}

#[cfg(unix)]
#[tokio::test]
async fn board_render_rejects_invalid_sides_before_starting_cli() {
    let scene = Scene::new();
    for side in [json!("back"), json!("invalid"), json!(12), Value::Null] {
        assert_invalid(&scene.render(json!({"side": side})).await, "side");
        assert!(!scene.argv.exists(), "invalid side started the CLI");
    }
}

#[cfg(unix)]
#[tokio::test]
async fn board_render_rejects_malformed_zoom_before_starting_cli() {
    let scene = Scene::new();
    for zoom in [json!("2"), json!(false), Value::Null] {
        assert_invalid(&scene.render(json!({"zoom": zoom})).await, "zoom");
        assert!(!scene.argv.exists(), "malformed zoom started the CLI");
    }
}

#[cfg(unix)]
#[tokio::test]
async fn board_render_rejects_non_positive_zoom_before_starting_cli() {
    let scene = Scene::new();
    for zoom in [0.0, -0.5] {
        assert_invalid(&scene.render(json!({"zoom": zoom})).await, "zoom");
        assert!(!scene.argv.exists(), "non-positive zoom started the CLI");
    }
}

#[cfg(unix)]
#[tokio::test]
async fn board_render_keeps_existing_artifacts_and_cleans_its_private_output() {
    let scene = Scene::new();
    let sibling = scene.board.with_extension("render.png");
    std::fs::write(&sibling, b"owner's existing artifact").unwrap();
    assert!(!scene.render(json!({})).await.is_error);
    assert_eq!(
        std::fs::read(&sibling).unwrap(),
        b"owner's existing artifact"
    );
    let args = scene.args();
    let output = Path::new(flag(&args, "--output"));
    assert_ne!(output.parent().unwrap(), scene.dir.path());
    assert!(!output.exists(), "temporary output was not cleaned");
    assert!(
        !output.parent().unwrap().exists(),
        "temporary directory was not cleaned"
    );
}

#[test]
fn board_render_schema_exposes_the_camera_options_it_accepts() {
    let tool = tools()
        .into_iter()
        .find(|tool| tool.name == "get_board_2d_view")
        .unwrap();
    assert_eq!(
        tool.input_schema["properties"]["side"]["enum"],
        json!(["top", "bottom"])
    );
    assert_eq!(
        tool.input_schema["properties"]["zoom"]["exclusiveMinimum"],
        0
    );
}
