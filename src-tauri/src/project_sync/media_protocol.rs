//! The native media transport; all path validation and bounded reads belong
//! to the application/infrastructure layers, shared with fixture tests.

use super::service::{
    read_project_media_range_with_repository, repository,
};
use mallard_infrastructure::session_detail::SessionDetailErrorCode;
use tauri::http::{Request, Response};

pub fn respond(request: Request<Vec<u8>>) -> Response<Vec<u8>> {
    let mut builder = Response::builder()
        .header("Accept-Ranges", "bytes")
        .header("Cache-Control", "no-store")
        .header("X-Content-Type-Options", "nosniff");
    if request.method() != "GET" && request.method() != "HEAD" {
        return builder
            .status(405)
            .header("Allow", "GET, HEAD")
            .body(Vec::new())
            .unwrap();
    }
    let Ok(repository) = repository() else {
        return builder.status(500).body(Vec::new()).unwrap();
    };
    let range = match request.headers().get("Range") {
        Some(header) => match header.to_str() {
            Ok(value) => Some(value),
            Err(_) => return builder.status(400).body(Vec::new()).unwrap(),
        },
        None => None,
    };
    match read_project_media_range_with_repository(
        &repository,
        request.uri().path(),
        range,
        request.method() == "HEAD",
    ) {
        Ok((metadata, response)) => {
            builder = builder
                .status(response.status)
                .header("Content-Type", metadata.mime_type)
                .header("Content-Length", response.content_length);
            if let Some(range) = response.content_range {
                builder = builder.header("Content-Range", range);
            }
            builder.body(response.bytes).unwrap()
        }
        Err(error) => builder
            .status(match error.code {
                SessionDetailErrorCode::NotFound => 404,
                SessionDetailErrorCode::ResponseTooLarge => 413,
                SessionDetailErrorCode::InvalidRequest
                | SessionDetailErrorCode::OwnershipChanged => 403,
                _ => 500,
            })
            .body(Vec::new())
            .unwrap(),
    }
}
