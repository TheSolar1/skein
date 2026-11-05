//! Local HTTP gateway: open `http://NAME.skein.localhost:PORT/` in any browser to load a page from the Skein network.
//! Browsers resolve `*.localhost` to 127.0.0.1 on their own, so no proxy, DNS or admin rights are needed.
use crate::identity::parse_addr;
use crate::node::Node;
use axum::{
    extract::{Request, State},
    http::{header, StatusCode},
    response::{Html, IntoResponse, Response},
    Router,
};
use std::sync::Arc;

fn content_type(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css",
        "js" | "mjs" => "text/javascript",
        "json" => "application/json",
        "txt" | "md" => "text/plain; charset=utf-8",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        "mp4" => "video/mp4",
        "mp3" => "audio/mpeg",
        "pdf" => "application/pdf",
        "wasm" => "application/wasm",
        _ => "application/octet-stream",
    }
}

async fn handle(State(node): State<Arc<Node>>, req: Request) -> Response {
    let host_port = req.headers().get(header::HOST).and_then(|h| h.to_str().ok()).unwrap_or("").to_lowercase();
    let host = host_port.split(':').next().unwrap_or("").to_string();
        let path = req.uri().path().to_string();
    // "name.skein.localhost" -> the name; "5c1e:....skein.localhost" is not a valid host, so raw addresses use /_/ADDRESS/path.
    let target = if let Some(name) = host.strip_suffix(".skein.localhost") {
        node.resolve(name).map(|a| (a, path.clone())).ok_or(format!("{name}.skein is not known (yet)"))
    } else if let Some(rest) = path.strip_prefix("/_/") {
        let (a, p) = rest.split_once('/').unwrap_or((rest, ""));
        parse_addr(a).map(|a| (a, format!("/{p}"))).ok_or("bad address".to_string())
    } else {
        return panel(&node, &path);
    };
    match target {
        Err(e) => (StatusCode::NOT_FOUND, e).into_response(),
        Ok((addr, p)) => match node.fetch(addr, &p).await {
            Ok(body) => ([(header::CONTENT_TYPE, content_type(if p.ends_with('/') { "index.html" } else { &p }))], body).into_response(),
            Err(e) => (StatusCode::BAD_GATEWAY, e.to_string()).into_response(),
        },
    }
}

const PANEL: &str = include_str!("panel.html");

fn panel(node: &Node, path: &str) -> Response {
    if path == "/_status.json" {
        return ([(header::CONTENT_TYPE, "application/json"), (header::CACHE_CONTROL, "no-store")], node.status().to_string()).into_response();
    }
    Html(PANEL).into_response()
}

pub async fn serve(node: Arc<Node>, listen: std::net::SocketAddr) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind(listen).await?;
    axum::serve(listener, Router::new().fallback(handle).with_state(node)).await
}
