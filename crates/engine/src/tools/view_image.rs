// Modified from OpenAI Codex (codex-rs/core/src/tools/handlers/view_image.rs and view_image_spec.rs @ 806d973): handler on ether's `ToolHandler`; reads from `LOCAL_FS`; the image is resized and re-encoded here with codex-utils-image (Codex defers that to history insertion); always `high` detail, so no `detail`, `environment_id` or output schema; emits `ViewImageToolCall` instead of turn items; no image-modality check (ether's `ModelProfile` has no modality list).
//! `view_image`: show the model a local image file.

use crate::tools::ToolError;
use crate::tools::ToolHandler;
use crate::tools::ToolInvocation;
use crate::tools::ToolResult;
use crate::tools::parse_arguments;
use codex_file_system::GetMetadataOptions;
use codex_file_system::LOCAL_FS;
use codex_file_system::ReadFileOptions;
use codex_protocol::models::DEFAULT_IMAGE_DETAIL;
use codex_protocol::models::FunctionCallOutputBody;
use codex_protocol::models::FunctionCallOutputContentItem;
use codex_protocol::models::FunctionCallOutputPayload;
use codex_protocol::models::ImageReference;
use codex_protocol::models::VIEW_IMAGE_TOOL_NAME;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::ViewImageToolCallEvent;
use codex_tools::JsonSchema;
use codex_tools::ResponsesApiTool;
use codex_tools::ToolSpec;
use codex_utils_image::PromptImageMode;
use codex_utils_image::load_for_prompt_bytes;
use codex_utils_path_uri::PathUri;
use futures::future::BoxFuture;
use serde::Deserialize;
use std::collections::BTreeMap;

const VIEW_IMAGE_INVALID_MESSAGE: &str =
    "unable to process image: invalid or unsupported image data";

pub(crate) struct ViewImageHandler;

#[derive(Deserialize)]
struct ViewImageArgs {
    path: String,
}

impl ToolHandler for ViewImageHandler {
    fn spec(&self) -> ToolSpec {
        create_view_image_tool()
    }

    fn supports_parallel(&self) -> bool {
        true
    }

    fn handle(&self, invocation: ToolInvocation) -> BoxFuture<'_, ToolResult> {
        Box::pin(async move {
            let ViewImageArgs { path } = parse_arguments(&invocation.payload)?;
            let cwd = PathUri::from_abs_path(&invocation.sess.config.cwd);
            let path_uri = cwd.join(&path).map_err(|err| {
                ToolError::RespondToModel(format!(
                    "unable to resolve image path `{path}` against cwd `{cwd}`: {err}"
                ))
            })?;
            let model_visible_path = path_uri.inferred_native_path_string();
            let fs = LOCAL_FS.as_ref();

            let metadata = fs
                .get_metadata(&path_uri, GetMetadataOptions::default(), None)
                .await
                .map_err(|error| {
                    ToolError::RespondToModel(format!(
                        "unable to locate image at `{model_visible_path}`: {error}"
                    ))
                })?;
            if !metadata.is_file {
                return Err(ToolError::RespondToModel(format!(
                    "image path `{model_visible_path}` is not a file"
                )));
            }
            let file_bytes = fs
                .read_file(&path_uri, ReadFileOptions::default(), None)
                .await
                .map_err(|error| {
                    ToolError::RespondToModel(format!(
                        "unable to read image at `{model_visible_path}`: {error}"
                    ))
                })?;
            let image = load_for_prompt_bytes(
                &path_uri.to_path_buf(),
                file_bytes,
                PromptImageMode::HIGH_DETAIL,
            )
            .map_err(|error| {
                if error.is_invalid_image() {
                    ToolError::RespondToModel(VIEW_IMAGE_INVALID_MESSAGE.to_string())
                } else {
                    ToolError::RespondToModel(format!(
                        "unable to process image at `{model_visible_path}`: {error}"
                    ))
                }
            })?;

            invocation
                .sess
                .send_event(
                    &invocation.turn_id,
                    EventMsg::ViewImageToolCall(ViewImageToolCallEvent {
                        call_id: invocation.call_id.clone(),
                        path: path_uri,
                    }),
                )
                .await;

            Ok(FunctionCallOutputPayload {
                body: FunctionCallOutputBody::ContentItems(vec![
                    FunctionCallOutputContentItem::InputImage {
                        image: ImageReference::Inline {
                            image_url: image.into_data_url(),
                        },
                        detail: Some(DEFAULT_IMAGE_DETAIL),
                    },
                ]),
                success: Some(true),
            })
        })
    }
}

fn create_view_image_tool() -> ToolSpec {
    let properties = BTreeMap::from([(
        "path".to_string(),
        JsonSchema::string(Some("Local filesystem path to an image file.".to_string())),
    )]);

    ToolSpec::Function(ResponsesApiTool {
        name: VIEW_IMAGE_TOOL_NAME.to_string(),
        description: "View a local image file from the filesystem when visual inspection is needed. Use this for images already available on disk."
            .to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(properties, Some(vec!["path".to_string()]), Some(false.into())),
        output_schema: None,
    })
}
