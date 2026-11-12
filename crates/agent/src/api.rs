//! Local HTTP and WebSocket API. Binds to 127.0.0.1 and requires the agent token.

use crate::supervisor::{SupError, Supervisor};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, Request, State};
use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use keeprun_core::model::{
    ApiError, AppView, Event, Health, IssueReport, NewApp, RunTask, UpdateApp,
};
use keeprun_core::{Env, Settings};
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::broadcast::error::RecvError;
use tower_http::cors::{AllowOrigin, CorsLayer};

#[derive(Clone)]
pub struct AppState {
    pub sup: Arc<Supervisor>,
    pub settings: Arc<Settings>,
    pub token: Arc<str>,
}

pub fn router(state: AppState) -> Router {
    let protected = Router::new()
        .route("/apps", get(list_apps).post(add_app))
        .route(
            "/apps/{id}",
            get(get_app).patch(update_app).delete(remove_app),
        )
        .route("/apps/{id}/events", get(app_events))
        .route("/events", get(all_events))
        .route("/apps/{id}/start", post(start_app))
        .route("/apps/{id}/stop", post(stop_app))
        .route("/apps/{id}/restart", post(restart_app))
        .route("/apps/{id}/outside/end", post(end_outside))
        .route("/apps/{id}/tasks/run", post(run_task))
        .route("/apps/{id}/tasks/stop", post(stop_task))
        .route("/apps/{id}/logs", get(logs_ws))
        .route("/apps/{id}/logs/recent", get(recent_logs))
        .route("/apps/{id}/issue", get(app_issue).delete(clear_issue))
        .route_layer(middleware::from_fn_with_state(state.clone(), auth));

    Router::new()
        .route("/health", get(health))
        .merge(protected)
        .layer(cors(state.settings.env))
        .with_state(state)
}

/// The desktop UI calls the API from its webview. Allow only the Tauri origins, plus the
/// Vite dev server in the dev environment.
fn cors(env: Env) -> CorsLayer {
    let mut origins = vec![
        "tauri://localhost",
        "http://tauri.localhost",
        "https://tauri.localhost",
    ];
    if env == Env::Dev {
        origins.extend(["http://localhost:1420", "http://127.0.0.1:1420"]);
    }
    let origins: Vec<HeaderValue> = origins.into_iter().map(HeaderValue::from_static).collect();
    CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_methods([Method::GET, Method::POST, Method::PATCH, Method::DELETE])
        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE])
}

#[derive(Deserialize)]
struct TokenQuery {
    token: Option<String>,
}

async fn auth(
    State(st): State<AppState>,
    Query(q): Query<TokenQuery>,
    req: Request,
    next: Next,
) -> Response {
    // Browsers cannot set headers on WebSocket requests, so `?token=` is accepted too.
    let given = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::to_string)
        .or(q.token);
    match given {
        Some(t) if constant_eq(t.as_bytes(), st.token.as_bytes()) => next.run(req).await,
        _ => err(
            StatusCode::UNAUTHORIZED,
            "Missing or wrong agent token. Restart the UI or CLI to reload it.",
        ),
    }
}

fn constant_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn err(code: StatusCode, msg: impl Into<String>) -> Response {
    (code, Json(ApiError { error: msg.into() })).into_response()
}

impl IntoResponse for SupError {
    fn into_response(self) -> Response {
        let code = match &self {
            SupError::NotFound(_) => StatusCode::NOT_FOUND,
            SupError::Conflict(_) => StatusCode::CONFLICT,
            SupError::BadRequest(_) => StatusCode::BAD_REQUEST,
            SupError::Internal(e) => {
                tracing::error!("{e:#}");
                StatusCode::INTERNAL_SERVER_ERROR
            }
        };
        err(code, self.to_string())
    }
}

type ApiResult<T> = Result<Json<T>, SupError>;

async fn health(State(st): State<AppState>) -> Json<Health> {
    Json(Health {
        ok: true,
        env: st.settings.env.to_string(),
        version: env!("CARGO_PKG_VERSION").into(),
    })
}

async fn list_apps(State(st): State<AppState>) -> Json<Vec<keeprun_core::model::AppView>> {
    Json(st.sup.list())
}

async fn add_app(
    State(st): State<AppState>,
    Json(body): Json<NewApp>,
) -> Result<Response, SupError> {
    let v = st.sup.add(body)?;
    Ok((StatusCode::CREATED, Json(v)).into_response())
}

async fn get_app(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<keeprun_core::model::AppView> {
    st.sup.view(&id).map(Json)
}

async fn remove_app(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, SupError> {
    st.sup.remove(&id)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn start_app(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<keeprun_core::model::AppView> {
    st.sup.start(&id).map(Json)
}

async fn stop_app(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<keeprun_core::model::AppView> {
    st.sup.stop(&id).await.map(Json)
}

/// End copies of the app running outside Keeprun.
async fn end_outside(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<keeprun_core::model::AppView> {
    st.sup.end_outside(&id).await.map(Json)
}

async fn restart_app(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<keeprun_core::model::AppView> {
    st.sup.restart(&id).await.map(Json)
}

async fn run_task(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<RunTask>,
) -> ApiResult<AppView> {
    st.sup.run_task(&id, body).map(Json)
}

async fn stop_task(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<AppView> {
    st.sup.stop_task(&id).map(Json)
}

async fn app_issue(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<IssueReport> {
    st.sup.issue(&id).map(Json)
}

async fn clear_issue(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<AppView> {
    st.sup.clear_issue(&id).map(Json)
}

#[derive(Deserialize)]
struct TailQuery {
    tail: Option<usize>,
}

async fn recent_logs(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<TailQuery>,
) -> ApiResult<Vec<keeprun_core::model::LogLine>> {
    st.sup.recent_logs(&id, q.tail.unwrap_or(200)).map(Json)
}

async fn logs_ws(
    State(st): State<AppState>,
    Path(id): Path<String>,
    ws: WebSocketUpgrade,
) -> Result<Response, SupError> {
    let (history, rx) = st.sup.subscribe_logs(&id)?;
    Ok(ws.on_upgrade(move |socket| stream_logs(socket, history, rx)))
}

async fn stream_logs(
    mut socket: WebSocket,
    history: Vec<keeprun_core::model::LogLine>,
    mut rx: tokio::sync::broadcast::Receiver<keeprun_core::model::LogLine>,
) {
    for line in history {
        if send(&mut socket, &line).await.is_err() {
            return;
        }
    }
    loop {
        tokio::select! {
            msg = rx.recv() => match msg {
                Ok(line) => if send(&mut socket, &line).await.is_err() { return },
                Err(RecvError::Lagged(n)) => {
                    let note = keeprun_core::model::LogLine {
                        ts: crate::supervisor::now_ms(),
                        stream: keeprun_core::model::Stream::System,
                        text: format!("{n} lines skipped because the client was too slow."),
                        task: false,
                    };
                    if send(&mut socket, &note).await.is_err() { return }
                }
                Err(RecvError::Closed) => return,
            },
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return,
                _ => {}
            },
        }
    }
}

async fn send(
    socket: &mut WebSocket,
    line: &keeprun_core::model::LogLine,
) -> Result<(), axum::Error> {
    let json = serde_json::to_string(line).unwrap_or_default();
    socket.send(Message::Text(json.into())).await
}

async fn update_app(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<UpdateApp>,
) -> ApiResult<keeprun_core::model::AppView> {
    st.sup.update(&id, body).map(Json)
}

#[derive(Deserialize)]
struct LimitQuery {
    limit: Option<u32>,
}

async fn app_events(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<LimitQuery>,
) -> ApiResult<Vec<Event>> {
    st.sup.events(Some(&id), q.limit.unwrap_or(100)).map(Json)
}

async fn all_events(
    State(st): State<AppState>,
    Query(q): Query<LimitQuery>,
) -> ApiResult<Vec<Event>> {
    st.sup.events(None, q.limit.unwrap_or(100)).map(Json)
}
