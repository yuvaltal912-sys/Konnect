//! MCP (Model Context Protocol) JSON-RPC 2.0 message types.
//!
//! Reference: https://spec.modelcontextprotocol.io/specification/

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ─── JSON-RPC Error Codes ────────────────────────────────────────────────────

pub const PARSE_ERROR: i32 = -32700;
pub const INVALID_REQUEST: i32 = -32600;
pub const INTERNAL_ERROR: i32 = -32603;

// ─── JSON-RPC 2.0 Base Types ─────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcNotification {
    pub jsonrpc: String,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

impl JsonRpcResponse {
    pub fn success(id: Value, result: Value) -> Self {
        JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn error(id: Value, err: JsonRpcError) -> Self {
        JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(err),
        }
    }
}

impl JsonRpcNotification {
    pub fn new(method: impl Into<String>, params: Option<Value>) -> Self {
        JsonRpcNotification {
            jsonrpc: "2.0".to_string(),
            method: method.into(),
            params,
        }
    }
}

// ─── MCP Initialize ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResult {
    pub protocol_version: String,
    pub capabilities: ServerCapabilities,
    pub server_info: ServerInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ServerCapabilities {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<ToolsCapability>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resources: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompts: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logging: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolsCapability {
    pub list_changed: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerInfo {
    pub name: String,
    pub version: String,
}

// ─── MCP Tools ───────────────────────────────────────────────────────────────

/// tools/list response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListToolsResult {
    pub tools: Vec<McpToolDescription>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// One tool as seen by the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpToolDescription {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

/// tools/call params
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallToolParams {
    pub name: String,
    #[serde(default)]
    pub arguments: Option<Value>,
}

/// tools/call response
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallToolResult {
    pub content: Vec<ToolContent>,
    #[serde(default)]
    pub is_error: bool,
}

/// MCP content block. `rename_all` covers the variant tags; the fields need
/// `rename_all_fields` of their own, or `mime_type` ships instead of the
/// spec's `mimeType` and clients reject the whole result.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "lowercase",
    rename_all_fields = "camelCase"
)]
pub enum ToolContent {
    Text { text: String },
    Image { data: String, mime_type: String },
}

impl CallToolResult {
    pub fn text(s: impl Into<String>) -> Self {
        CallToolResult {
            content: vec![ToolContent::Text { text: s.into() }],
            is_error: false,
        }
    }

    pub fn error(s: impl Into<String>) -> Self {
        CallToolResult {
            content: vec![ToolContent::Text { text: s.into() }],
            is_error: true,
        }
    }

    pub fn json(v: &impl Serialize) -> Self {
        let text = serde_json::to_string(v).unwrap_or_default();
        CallToolResult::text(text)
    }

    pub fn image(data: impl Into<String>, mime_type: impl Into<String>) -> Self {
        CallToolResult {
            content: vec![ToolContent::Image {
                data: data.into(),
                mime_type: mime_type.into(),
            }],
            is_error: false,
        }
    }

    /// Keep machine-readable facts in text and the PNG in an MCP image block,
    /// where image-capable clients can inspect it without parsing base64 text.
    /// Callers check [`inline_over_budget`] first.
    pub fn json_with_png(metadata: &impl Serialize, png: &[u8]) -> Self {
        use base64::Engine as _;
        let mut result = Self::json(metadata);
        result.content.push(ToolContent::Image {
            data: base64::engine::general_purpose::STANDARD.encode(png),
            mime_type: "image/png".into(),
        });
        result
    }
}

// ─── Inline image budget ─────────────────────────────────────────────────────

/// The largest side an inline image block may have: the Claude API's
/// documented per-image limit of 8000 px.
pub const INLINE_IMAGE_MAX_SIDE_PX: u32 = 8000;

/// The largest PNG an inline image block may carry. The Claude API's
/// documented per-image limit is 5 MB, and 3,750,000 bytes are 5,000,000
/// once base64-encoded.
pub const INLINE_IMAGE_MAX_PNG_BYTES: usize = 3_750_000;

/// An image too large for an inline block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineOverBudget {
    /// Which limit the image breaks, with its numbers.
    pub reason: String,
    /// The widest `width_px` at the same aspect ratio that should fit.
    pub fit_width_px: u32,
}

impl InlineOverBudget {
    /// The refusal a tool returns instead of the image: the limit and the
    /// remedies, where `narrow` adds the tool's own (such as a region), and
    /// a PNG file already written is named rather than hidden.
    pub fn refusal(&self, narrow: &str, written: Option<&std::path::Path>) -> CallToolResult {
        let written = written
            .map(|path| format!(" The PNG was still written to {}.", path.display()))
            .unwrap_or_default();
        CallToolResult::error(format!(
            "the render is too large to return inline: {}. Lower width_px (about {} or less \
             fits){narrow}, or set 'inline' false with an 'output' path.{written}",
            self.reason, self.fit_width_px
        ))
    }
}

/// The one inline budget every image-returning tool checks before
/// [`CallToolResult::json_with_png`]. `None` when the image fits.
pub fn inline_over_budget(
    width_px: u32,
    height_px: u32,
    png_bytes: usize,
) -> Option<InlineOverBudget> {
    let reason = if width_px.max(height_px) > INLINE_IMAGE_MAX_SIDE_PX {
        format!(
            "the image is {width_px} x {height_px} px, over the inline limit of \
             {INLINE_IMAGE_MAX_SIDE_PX} px per side"
        )
    } else if png_bytes > INLINE_IMAGE_MAX_PNG_BYTES {
        format!(
            "the PNG is {png_bytes} bytes, over the inline limit of \
             {INLINE_IMAGE_MAX_PNG_BYTES} bytes (about 5 MB once base64-encoded)"
        )
    } else {
        return None;
    };
    Some(InlineOverBudget {
        reason,
        fit_width_px: inline_fit_width_px(width_px, height_px, Some(png_bytes)),
    })
}

/// The widest width at the same aspect ratio that keeps both sides within
/// [`INLINE_IMAGE_MAX_SIDE_PX`] and, when the PNG size is known, its bytes
/// within [`INLINE_IMAGE_MAX_PNG_BYTES`]. PNG size is assumed to scale
/// with pixel area, less a 10% margin, so that part is an estimate.
pub fn inline_fit_width_px(width_px: u32, height_px: u32, png_bytes: Option<usize>) -> u32 {
    let longest = f64::from(width_px.max(height_px).max(1));
    // One pixel of slack for the caller's rounding of the other side.
    let mut scale = (f64::from(INLINE_IMAGE_MAX_SIDE_PX - 1) / longest).min(1.0);
    if let Some(bytes) = png_bytes.filter(|bytes| *bytes > INLINE_IMAGE_MAX_PNG_BYTES) {
        scale = scale.min(0.9 * (INLINE_IMAGE_MAX_PNG_BYTES as f64 / bytes as f64).sqrt());
    }
    ((f64::from(width_px) * scale).floor() as u32).clamp(1, width_px.max(1))
}

// ─── MCP Notifications ───────────────────────────────────────────────────────

/// Sent by server when the tool list changes (after load_toolset / unload_toolset).
pub const TOOLS_LIST_CHANGED: &str = "notifications/tools/list_changed";

#[cfg(test)]
mod tests {
    use super::*;

    /// The exact keys MCP clients validate against — a rename here breaks them.
    #[test]
    fn image_content_serializes_with_spec_keys() {
        let v = serde_json::to_value(CallToolResult::image("QUJD", "image/png")).unwrap();
        assert_eq!(
            v,
            serde_json::json!({
                "content": [{ "type": "image", "data": "QUJD", "mimeType": "image/png" }],
                "isError": false,
            })
        );
    }

    #[test]
    fn json_with_png_keeps_metadata_and_image_in_separate_blocks() {
        let value = serde_json::to_value(CallToolResult::json_with_png(
            &serde_json::json!({"width_px": 23}),
            b"PNG bytes",
        ))
        .unwrap();
        assert_eq!(value["content"].as_array().unwrap().len(), 2);
        assert_eq!(
            value["content"][0],
            serde_json::json!({
                "type": "text", "text": "{\"width_px\":23}"
            })
        );
        assert_eq!(
            value["content"][1],
            serde_json::json!({
                "type": "image", "data": "UE5HIGJ5dGVz", "mimeType": "image/png"
            })
        );
        assert_eq!(value["isError"], false);
    }

    #[test]
    fn inline_budget_refuses_a_side_over_8000_px_or_a_png_over_3_75_mb() {
        assert_eq!(inline_over_budget(8000, 8000, 3_750_000), None);
        let tall = inline_over_budget(1600, 8160, 1000).unwrap();
        assert!(tall.reason.contains("1600 x 8160 px"), "{tall:?}");
        assert!(tall.reason.contains("8000 px per side"), "{tall:?}");
        // 1568 px wide keeps the region's height at 7997 px.
        assert_eq!(tall.fit_width_px, 1568);
        let heavy = inline_over_budget(4096, 4096, 7_500_000).unwrap();
        assert!(heavy.reason.contains("7500000 bytes"), "{heavy:?}");
        assert!(heavy.reason.contains("3750000 bytes"), "{heavy:?}");
        // Half the bytes needs about 1/sqrt(2) of the width, less the margin.
        assert_eq!(heavy.fit_width_px, 2606);
        assert_eq!(inline_fit_width_px(100, 50, None), 100);
    }

    #[test]
    fn text_content_serializes_with_spec_keys() {
        let v = serde_json::to_value(CallToolResult::error("boom")).unwrap();
        assert_eq!(
            v,
            serde_json::json!({
                "content": [{ "type": "text", "text": "boom" }],
                "isError": true,
            })
        );
    }

    #[test]
    fn tool_description_serializes_input_schema_as_camel_case() {
        let v = serde_json::to_value(McpToolDescription {
            name: "get_board_2d_view".into(),
            description: "Render the board.".into(),
            input_schema: serde_json::json!({ "type": "object" }),
        })
        .unwrap();
        assert!(v.get("inputSchema").is_some(), "got {v}");
    }
}
