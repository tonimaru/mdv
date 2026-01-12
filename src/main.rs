use askama::Template;
use axum::{
    extract::{Path, State},
    http::{header, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::get,
    Router,
};
use chrono::{DateTime, Local};
use clap::Parser;
use pulldown_cmark::{html, Options, Parser as MdParser};
use std::{
    fs,
    path::PathBuf,
    sync::Arc,
};

#[derive(Parser)]
#[command(name = "mdv")]
#[command(about = "Markdown Directory Viewer - A web-based markdown preview tool")]
struct Args {
    /// Root directory to serve
    #[arg(default_value = ".")]
    root: PathBuf,

    /// Port to listen on
    #[arg(short, long, default_value = "3000")]
    port: u16,

    /// Host to bind to
    #[arg(long, default_value = "127.0.0.1")]
    host: String,
}

#[derive(Clone)]
struct AppState {
    root_dir: Arc<PathBuf>,
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
}

#[derive(Template)]
#[template(path = "markdown.html")]
struct MarkdownTemplate {
    breadcrumbs: Vec<BreadcrumbItem>,
    content: String,
    filename: String,
    file_size: String,
    raw_path: String,
}

fn generate_breadcrumbs(path: &str) -> Vec<BreadcrumbItem> {
    let mut breadcrumbs = vec![BreadcrumbItem {
        name: "root".to_string(),
        path: "/".to_string(),
        is_last: path.is_empty(),
    }];

    if !path.is_empty() {
        let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        let mut current_path = String::new();

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

async fn handle_root(State(state): State<AppState>) -> Response {
    handle_path_internal(&state, "").await
}

async fn handle_path(
    State(state): State<AppState>,
    Path(path): Path<String>,
) -> Response {
    handle_path_internal(&state, &path).await
}

async fn handle_path_internal(state: &AppState, path: &str) -> Response {
    let Some(full_path) = validate_path(&state.root_dir, path) else {
        return (StatusCode::NOT_FOUND, Html("Not Found")).into_response();
    };

    if full_path.is_dir() {
        render_directory(state, &full_path, path).await
    } else if full_path.is_file() {
        let extension = full_path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if extension == "md" {
            render_markdown_file(state, &full_path, path).await
        } else {
            serve_static_file(&full_path).await
        }
    } else {
        (StatusCode::NOT_FOUND, Html("Not Found")).into_response()
    }
}

async fn render_directory(_state: &AppState, full_path: &PathBuf, url_path: &str) -> Response {
    let Ok(read_dir) = fs::read_dir(full_path) else {
        return (StatusCode::INTERNAL_SERVER_ERROR, Html("Failed to read directory")).into_response();
    };

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
                format!("/{}", name)
            } else {
                format!("/{}/{}", url_path.trim_start_matches('/'), name)
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

    entries.sort_by(|a, b| {
        match (a.is_dir, b.is_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        }
    });

    let breadcrumbs = generate_breadcrumbs(url_path);
    let has_parent = !url_path.is_empty();
    let parent_path = if has_parent {
        let parts: Vec<&str> = url_path.split('/').filter(|s| !s.is_empty()).collect();
        if parts.len() <= 1 {
            "/".to_string()
        } else {
            format!("/{}", parts[..parts.len() - 1].join("/"))
        }
    } else {
        "/".to_string()
    };

    let template = DirectoryTemplate {
        breadcrumbs,
        entries,
        has_parent,
        parent_path,
    };

    match template.render() {
        Ok(html) => Html(html).into_response(),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, Html("Template error")).into_response(),
    }
}

async fn render_markdown_file(_state: &AppState, full_path: &PathBuf, url_path: &str) -> Response {
    let Ok(content) = fs::read_to_string(full_path) else {
        return (StatusCode::INTERNAL_SERVER_ERROR, Html("Failed to read file")).into_response();
    };

    let html_content = render_markdown(&content);
    let breadcrumbs = generate_breadcrumbs(url_path);

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

    let raw_path = format!("/_raw{}", url_path);

    let template = MarkdownTemplate {
        breadcrumbs,
        content: html_content,
        filename,
        file_size,
        raw_path,
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

    let mime_type = mime_guess::from_path(full_path)
        .first_or_octet_stream()
        .to_string();

    ([(header::CONTENT_TYPE, mime_type)], content).into_response()
}

async fn handle_raw(
    State(state): State<AppState>,
    Path(path): Path<String>,
) -> Response {
    let Some(full_path) = validate_path(&state.root_dir, &path) else {
        return (StatusCode::NOT_FOUND, Html("Not Found")).into_response();
    };

    if full_path.is_file() {
        serve_static_file(&full_path).await
    } else {
        (StatusCode::NOT_FOUND, Html("Not Found")).into_response()
    }
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    let root_dir = args.root.canonicalize().unwrap_or_else(|e| {
        eprintln!("Error: Cannot resolve root directory: {}", e);
        std::process::exit(1);
    });

    if !root_dir.is_dir() {
        eprintln!("Error: {} is not a directory", root_dir.display());
        std::process::exit(1);
    }

    let state = AppState {
        root_dir: Arc::new(root_dir.clone()),
    };

    let app = Router::new()
        .route("/", get(handle_root))
        .route("/_raw/{*path}", get(handle_raw))
        .route("/{*path}", get(handle_path))
        .with_state(state);

    let addr = format!("{}:{}", args.host, args.port);
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap_or_else(|e| {
        eprintln!("Error: Cannot bind to {}: {}", addr, e);
        std::process::exit(1);
    });

    println!("Serving {} at http://{}", root_dir.display(), addr);

    axum::serve(listener, app).await.unwrap();
}
