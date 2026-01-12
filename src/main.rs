use askama::Template;
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path, Query, State,
    },
    http::{header, StatusCode},
    response::{
        sse::{Event, KeepAlive, Sse},
        Html, IntoResponse, Response,
    },
    routing::{delete, get, post},
    Json, Router,
};
use chrono::{DateTime, Local};
use clap::Parser;
use futures::{stream::Stream, SinkExt, StreamExt};
use notify::{Config, PollWatcher, RecursiveMode, Watcher};
use pulldown_cmark::{html, Options, Parser as MdParser};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    convert::Infallible,
    fs,
    path::PathBuf,
    sync::Arc,
};
use tokio::sync::{broadcast, RwLock};

#[derive(Parser)]
#[command(name = "mdv")]
#[command(about = "Markdown Directory Viewer - A multi-workspace markdown preview server")]
struct Args {
    /// Port to listen on
    #[arg(short, long, default_value = "3000")]
    port: u16,

    /// Host to bind to
    #[arg(long, default_value = "127.0.0.1")]
    host: String,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
enum WsCommand {
    Navigate { url: String },
    Scroll { percent: u32 },
    Focus { workspace_id: String, file_path: String },
}

struct Workspace {
    id: String,
    root_dir: PathBuf,
    name: String,
    #[allow(dead_code)]
    watcher_handle: Option<std::thread::JoinHandle<()>>,
}

struct AppStateInner {
    workspaces: HashMap<String, Workspace>,
}

#[derive(Clone)]
struct AppState {
    inner: Arc<RwLock<AppStateInner>>,
    reload_tx: broadcast::Sender<String>,
    ws_tx: broadcast::Sender<WsCommand>,
}

#[derive(Deserialize)]
struct RegisterRequest {
    path: String,
}

#[derive(Serialize)]
struct RegisterResponse {
    id: String,
    name: String,
    url: String,
}

#[derive(Deserialize)]
struct ActiveQuery {
    path: String,
}

#[derive(Serialize)]
struct StatusResponse {
    status: String,
    workspaces: Vec<WorkspaceInfo>,
}

#[derive(Serialize)]
struct WorkspaceInfo {
    id: String,
    name: String,
    path: String,
}

#[derive(Deserialize)]
struct ScrollQuery {
    percent: u32,
}

#[derive(Clone)]
struct BreadcrumbItem {
    name: String,
    path: String,
    is_last: bool,
}

#[derive(Clone)]
struct FileEntry {
    name: String,
    path: String,
    is_dir: bool,
    size: String,
    modified: String,
}

#[derive(Template)]
#[template(path = "directory.html")]
struct DirectoryTemplate {
    breadcrumbs: Vec<BreadcrumbItem>,
    entries: Vec<FileEntry>,
    has_parent: bool,
    parent_path: String,
    workspace_id: String,
    workspace_name: String,
}

#[derive(Template)]
#[template(path = "markdown.html")]
struct MarkdownTemplate {
    breadcrumbs: Vec<BreadcrumbItem>,
    content: String,
    filename: String,
    file_size: String,
    raw_path: String,
    workspace_id: String,
    workspace_name: String,
}

fn generate_workspace_id(path: &PathBuf) -> String {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("workspace");

    let hash = {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        path.hash(&mut hasher);
        hasher.finish()
    };

    format!("{}-{:x}", name, hash & 0xFFFF)
}

fn generate_breadcrumbs(workspace_id: &str, workspace_name: &str, path: &str) -> Vec<BreadcrumbItem> {
    let base_path = format!("/view/{}", workspace_id);
    let mut breadcrumbs = vec![
        BreadcrumbItem {
            name: "root".to_string(),
            path: "/".to_string(),
            is_last: false,
        },
        BreadcrumbItem {
            name: workspace_name.to_string(),
            path: base_path.clone(),
            is_last: path.is_empty(),
        },
    ];

    if !path.is_empty() {
        let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        let mut current_path = base_path;

        for (i, part) in parts.iter().enumerate() {
            current_path.push('/');
            current_path.push_str(part);
            breadcrumbs.push(BreadcrumbItem {
                name: part.to_string(),
                path: current_path.clone(),
                is_last: i == parts.len() - 1,
            });
        }
    }

    breadcrumbs
}

fn format_file_size(size: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if size >= GB {
        format!("{:.1} GB", size as f64 / GB as f64)
    } else if size >= MB {
        format!("{:.1} MB", size as f64 / MB as f64)
    } else if size >= KB {
        format!("{:.1} KB", size as f64 / KB as f64)
    } else {
        format!("{} B", size)
    }
}

fn format_datetime(time: std::time::SystemTime) -> String {
    let datetime: DateTime<Local> = time.into();
    datetime.format("%Y-%m-%d %H:%M").to_string()
}

fn render_markdown(content: &str) -> String {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_FOOTNOTES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);

    let parser = MdParser::new_ext(content, options);
    let mut html_output = String::new();
    html::push_html(&mut html_output, parser);
    html_output
}

fn contains_markdown(path: &PathBuf) -> bool {
    if path.is_file() {
        return path.extension().and_then(|e| e.to_str()) == Some("md");
    }

    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries.filter_map(|e| e.ok()) {
            let entry_path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            if contains_markdown(&entry_path) {
                return true;
            }
        }
    }
    false
}

fn validate_path(root: &PathBuf, requested_path: &str) -> Option<PathBuf> {
    let cleaned_path = requested_path.trim_start_matches('/');
    let full_path = root.join(cleaned_path);

    match full_path.canonicalize() {
        Ok(canonical) => {
            let root_canonical = root.canonicalize().ok()?;
            if canonical.starts_with(&root_canonical) {
                Some(canonical)
            } else {
                None
            }
        }
        Err(_) => None,
    }
}

// API: Register workspace
async fn api_register(
    State(state): State<AppState>,
    Json(req): Json<RegisterRequest>,
) -> Response {
    let path = PathBuf::from(&req.path);
    let Ok(canonical_path) = path.canonicalize() else {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "Invalid path"}))).into_response();
    };

    if !canonical_path.is_dir() {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "Path is not a directory"}))).into_response();
    }

    let workspace_id = generate_workspace_id(&canonical_path);
    let workspace_name = canonical_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("workspace")
        .to_string();

    let mut inner = state.inner.write().await;

    if !inner.workspaces.contains_key(&workspace_id) {
        let reload_tx = state.reload_tx.clone();
        let watch_id = workspace_id.clone();
        let watch_dir = canonical_path.clone();

        let watcher_handle = std::thread::spawn(move || {
            let (tx, rx) = std::sync::mpsc::channel();
            let config = Config::default().with_poll_interval(std::time::Duration::from_millis(500));
            let Ok(mut watcher) = PollWatcher::new(tx, config) else { return };
            if watcher.watch(&watch_dir, RecursiveMode::Recursive).is_err() {
                return;
            }

            loop {
                match rx.recv() {
                    Ok(Ok(event)) => {
                        let is_md = event.paths.iter().any(|p| {
                            p.extension().and_then(|e| e.to_str()) == Some("md")
                        });
                        if is_md {
                            let _ = reload_tx.send(watch_id.clone());
                        }
                    }
                    Ok(Err(_)) => {}
                    Err(_) => break,
                }
            }
        });

        inner.workspaces.insert(
            workspace_id.clone(),
            Workspace {
                id: workspace_id.clone(),
                root_dir: canonical_path.clone(),
                name: workspace_name.clone(),
                watcher_handle: Some(watcher_handle),
            },
        );
    }

    let response = RegisterResponse {
        id: workspace_id.clone(),
        name: workspace_name,
        url: format!("/view/{}", workspace_id),
    };

    Json(response).into_response()
}

// API: Remove workspace
async fn api_unregister(
    State(state): State<AppState>,
    Path(workspace_id): Path<String>,
) -> Response {
    let mut inner = state.inner.write().await;

    if let Some(workspace) = inner.workspaces.remove(&workspace_id) {
        // Drop the watcher handle to stop the file watcher thread
        drop(workspace.watcher_handle);
        Json(serde_json::json!({"status": "ok", "id": workspace_id})).into_response()
    } else {
        (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Workspace not found"}))).into_response()
    }
}

// API: Get active file URL and notify browser
async fn api_active(
    State(state): State<AppState>,
    Query(query): Query<ActiveQuery>,
) -> Response {
    let abs_path = PathBuf::from(&query.path);
    let Ok(canonical_path) = abs_path.canonicalize() else {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "Invalid path"}))).into_response();
    };

    let inner = state.inner.read().await;

    for (id, workspace) in &inner.workspaces {
        if let Ok(workspace_canonical) = workspace.root_dir.canonicalize() {
            if canonical_path.starts_with(&workspace_canonical) {
                let relative = canonical_path
                    .strip_prefix(&workspace_canonical)
                    .unwrap_or(&canonical_path);
                let relative_str = relative.to_string_lossy();
                let url = format!("/view/{}/{}", id, relative_str);

                let _ = state.ws_tx.send(WsCommand::Focus {
                    workspace_id: id.clone(),
                    file_path: relative_str.to_string(),
                });
                let _ = state.ws_tx.send(WsCommand::Navigate { url: url.clone() });

                return Json(serde_json::json!({
                    "url": url,
                    "workspace_id": id,
                })).into_response();
            }
        }
    }

    (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "File not in any registered workspace"}))).into_response()
}

// API: Status check
async fn api_status(State(state): State<AppState>) -> Json<StatusResponse> {
    let inner = state.inner.read().await;
    let workspaces: Vec<WorkspaceInfo> = inner
        .workspaces
        .values()
        .map(|w| WorkspaceInfo {
            id: w.id.clone(),
            name: w.name.clone(),
            path: w.root_dir.to_string_lossy().to_string(),
        })
        .collect();

    Json(StatusResponse {
        status: "ok".to_string(),
        workspaces,
    })
}

// API: Scroll sync
async fn api_scroll(
    State(state): State<AppState>,
    Query(query): Query<ScrollQuery>,
) -> &'static str {
    let _ = state.ws_tx.send(WsCommand::Scroll { percent: query.percent });
    "ok"
}

// View workspace root
async fn handle_view_root(
    State(state): State<AppState>,
    Path(workspace_id): Path<String>,
) -> Response {
    handle_view_path_internal(&state, &workspace_id, "").await
}

// View workspace path
async fn handle_view_path(
    State(state): State<AppState>,
    Path((workspace_id, path)): Path<(String, String)>,
) -> Response {
    handle_view_path_internal(&state, &workspace_id, &path).await
}

async fn handle_view_path_internal(state: &AppState, workspace_id: &str, path: &str) -> Response {
    let inner = state.inner.read().await;
    let Some(workspace) = inner.workspaces.get(workspace_id) else {
        return (StatusCode::NOT_FOUND, Html("Workspace not found")).into_response();
    };

    let Some(full_path) = validate_path(&workspace.root_dir, path) else {
        return (StatusCode::NOT_FOUND, Html("Not Found")).into_response();
    };

    let workspace_name = workspace.name.clone();
    drop(inner);

    if full_path.is_dir() {
        render_directory(workspace_id, &workspace_name, &full_path, path).await
    } else if full_path.is_file() {
        let extension = full_path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if extension == "md" {
            render_markdown_file(workspace_id, &workspace_name, &full_path, path).await
        } else {
            serve_static_file(&full_path).await
        }
    } else {
        (StatusCode::NOT_FOUND, Html("Not Found")).into_response()
    }
}

async fn render_directory(
    workspace_id: &str,
    workspace_name: &str,
    full_path: &PathBuf,
    url_path: &str,
) -> Response {
    let Ok(read_dir) = fs::read_dir(full_path) else {
        return (StatusCode::INTERNAL_SERVER_ERROR, Html("Failed to read directory")).into_response();
    };

    let base_url = format!("/view/{}", workspace_id);

    let mut entries: Vec<FileEntry> = read_dir
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                return None;
            }

            let entry_full_path = entry.path();
            let metadata = entry.metadata().ok()?;
            let is_dir = metadata.is_dir();

            if is_dir {
                if !contains_markdown(&entry_full_path) {
                    return None;
                }
            } else {
                let is_md = entry_full_path.extension().and_then(|e| e.to_str()) == Some("md");
                if !is_md {
                    return None;
                }
            }

            let size = if is_dir {
                "-".to_string()
            } else {
                format_file_size(metadata.len())
            };
            let modified = metadata
                .modified()
                .ok()
                .map(format_datetime)
                .unwrap_or_else(|| "-".to_string());

            let entry_path = if url_path.is_empty() {
                format!("{}/{}", base_url, name)
            } else {
                format!("{}/{}/{}", base_url, url_path.trim_start_matches('/'), name)
            };

            Some(FileEntry {
                name,
                path: entry_path,
                is_dir,
                size,
                modified,
            })
        })
        .collect();

    entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });

    let breadcrumbs = generate_breadcrumbs(workspace_id, workspace_name, url_path);
    let has_parent = !url_path.is_empty();
    let parent_path = if has_parent {
        let parts: Vec<&str> = url_path.split('/').filter(|s| !s.is_empty()).collect();
        if parts.len() <= 1 {
            base_url
        } else {
            format!("{}/{}", base_url, parts[..parts.len() - 1].join("/"))
        }
    } else {
        base_url
    };

    let template = DirectoryTemplate {
        breadcrumbs,
        entries,
        has_parent,
        parent_path,
        workspace_id: workspace_id.to_string(),
        workspace_name: workspace_name.to_string(),
    };

    match template.render() {
        Ok(html) => Html(html).into_response(),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, Html("Template error")).into_response(),
    }
}

async fn render_markdown_file(
    workspace_id: &str,
    workspace_name: &str,
    full_path: &PathBuf,
    url_path: &str,
) -> Response {
    let Ok(content) = fs::read_to_string(full_path) else {
        return (StatusCode::INTERNAL_SERVER_ERROR, Html("Failed to read file")).into_response();
    };

    let html_content = render_markdown(&content);
    let breadcrumbs = generate_breadcrumbs(workspace_id, workspace_name, url_path);

    let metadata = fs::metadata(full_path).ok();
    let file_size = metadata
        .as_ref()
        .map(|m| format_file_size(m.len()))
        .unwrap_or_else(|| "-".to_string());

    let filename = full_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string();

    let raw_path = format!("/_raw/{}/{}", workspace_id, url_path.trim_start_matches('/'));

    let template = MarkdownTemplate {
        breadcrumbs,
        content: html_content,
        filename,
        file_size,
        raw_path,
        workspace_id: workspace_id.to_string(),
        workspace_name: workspace_name.to_string(),
    };

    match template.render() {
        Ok(html) => Html(html).into_response(),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, Html("Template error")).into_response(),
    }
}

async fn serve_static_file(full_path: &PathBuf) -> Response {
    let Ok(content) = fs::read(full_path) else {
        return (StatusCode::INTERNAL_SERVER_ERROR, Html("Failed to read file")).into_response();
    };

    let mime = mime_guess::from_path(full_path).first_or_octet_stream();
    let content_type = if mime.type_() == "text" {
        format!("{}; charset=utf-8", mime)
    } else {
        mime.to_string()
    };

    ([(header::CONTENT_TYPE, content_type)], content).into_response()
}

async fn handle_raw(
    State(state): State<AppState>,
    Path((workspace_id, path)): Path<(String, String)>,
) -> Response {
    let inner = state.inner.read().await;
    let Some(workspace) = inner.workspaces.get(&workspace_id) else {
        return (StatusCode::NOT_FOUND, Html("Workspace not found")).into_response();
    };

    let Some(full_path) = validate_path(&workspace.root_dir, &path) else {
        return (StatusCode::NOT_FOUND, Html("Not Found")).into_response();
    };
    drop(inner);

    if full_path.is_file() {
        serve_static_file(&full_path).await
    } else {
        (StatusCode::NOT_FOUND, Html("Not Found")).into_response()
    }
}

async fn handle_reload(
    State(state): State<AppState>,
    Path(workspace_id): Path<String>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let mut rx = state.reload_tx.subscribe();
    let ws_id = workspace_id.clone();

    let stream = async_stream::stream! {
        loop {
            match rx.recv().await {
                Ok(id) => {
                    if id == ws_id {
                        yield Ok(Event::default().event("reload").data("reload"));
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    };

    Sse::new(stream).keep_alive(KeepAlive::default())
}

async fn handle_ws(ws: WebSocketUpgrade, State(state): State<AppState>) -> Response {
    ws.on_upgrade(|socket| handle_ws_connection(socket, state))
}

async fn handle_ws_connection(socket: WebSocket, state: AppState) {
    let (mut sender, mut receiver) = socket.split();
    let mut ws_rx = state.ws_tx.subscribe();

    let send_task = tokio::spawn(async move {
        loop {
            match ws_rx.recv().await {
                Ok(cmd) => {
                    if let Ok(json) = serde_json::to_string(&cmd) {
                        if sender.send(Message::Text(json.into())).await.is_err() {
                            break;
                        }
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    let recv_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = receiver.next().await {
            if let Message::Close(_) = msg {
                break;
            }
        }
    });

    tokio::select! {
        _ = send_task => {}
        _ = recv_task => {}
    }
}

// Root page with workspace list
async fn handle_root(State(state): State<AppState>) -> Response {
    let inner = state.inner.read().await;
    let workspaces: Vec<_> = inner.workspaces.values().collect();

    let workspace_list = if workspaces.is_empty() {
        "<p style=\"color:#8b949e;\">No workspaces registered yet.</p>".to_string()
    } else {
        let items: Vec<String> = workspaces
            .iter()
            .map(|ws| {
                format!(
                    r#"<li><a href="/view/{}" style="color:#58a6ff;">{}</a> <span style="color:#8b949e;">- {}</span></li>"#,
                    ws.id, ws.name, ws.root_dir.display()
                )
            })
            .collect();
        format!("<ul>{}</ul>", items.join("\n"))
    };

    let html = format!(
        r#"<!DOCTYPE html>
<html>
<head><title>mdv server</title></head>
<body style="background:#0d1117;color:#c9d1d9;font-family:sans-serif;padding:2rem;">
<h1>mdv server is running</h1>
<p>Use your editor plugin to register workspaces and open files.</p>
<h2>Workspaces</h2>
{}
<h2>API endpoints</h2>
<ul>
<li>POST /api/workspace/register - Register a workspace</li>
<li>DELETE /api/workspace/{{id}} - Remove a workspace</li>
<li>GET /api/active?path=... - Navigate to a file</li>
<li>GET /api/status - Server status</li>
</ul>
</body>
</html>"#,
        workspace_list
    );

    (StatusCode::OK, Html(html)).into_response()
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    let (reload_tx, _) = broadcast::channel::<String>(16);
    let (ws_tx, _) = broadcast::channel::<WsCommand>(16);

    let state = AppState {
        inner: Arc::new(RwLock::new(AppStateInner {
            workspaces: HashMap::new(),
        })),
        reload_tx,
        ws_tx,
    };

    let app = Router::new()
        .route("/", get(handle_root))
        .route("/api/workspace/register", post(api_register))
        .route("/api/workspace/{id}", delete(api_unregister))
        .route("/api/active", get(api_active))
        .route("/api/status", get(api_status))
        .route("/api/remote/scroll", get(api_scroll))
        .route("/ws", get(handle_ws))
        .route("/view/{workspace_id}", get(handle_view_root))
        .route("/view/{workspace_id}/{*path}", get(handle_view_path))
        .route("/_reload/{workspace_id}", get(handle_reload))
        .route("/_raw/{workspace_id}/{*path}", get(handle_raw))
        .with_state(state);

    let addr = format!("{}:{}", args.host, args.port);
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap_or_else(|e| {
        eprintln!("Error: Cannot bind to {}: {}", addr, e);
        std::process::exit(1);
    });

    println!("mdv server listening at http://{}", addr);

    axum::serve(listener, app).await.unwrap();
}
