use crate::{
    service::{ExternalGrant, ExternalService},
    tokens::{TokenAuthError, TokenStore},
};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
};
use pkms_core::{
    ExternalError, ExternalErrorCode, ExternalRequest, ExternalResponse, ExternalScope,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    net::{Ipv4Addr, SocketAddr},
    sync::Arc,
};
use tokio::{net::TcpListener, sync::Mutex};

#[derive(Clone)]
struct ApiState {
    service: Arc<Mutex<ExternalService>>,
    tokens: Arc<TokenStore>,
}

pub async fn serve_http(
    port: u16,
    service: ExternalService,
    tokens: TokenStore,
) -> std::io::Result<()> {
    let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, port))).await?;
    let address = listener.local_addr()?;
    eprintln!("spanseek external HTTP listening on http://{address}");
    let state = ApiState {
        service: Arc::new(Mutex::new(service)),
        tokens: Arc::new(tokens),
    };
    let app = Router::new()
        .route("/v1/request", post(request))
        .route("/v1/capabilities", post(capabilities))
        .layer(DefaultBodyLimit::max(1024 * 1024))
        .with_state(state);
    axum::serve(listener, app).await
}

async fn capabilities(State(state): State<ApiState>, headers: HeaderMap) -> Response {
    let request = ExternalRequest {
        protocol: pkms_core::EXTERNAL_PROTOCOL.into(),
        request_id: format!("http-{}", uuid::Uuid::new_v4()),
        command: pkms_core::ExternalCommand::Capabilities,
    };
    handle_request(state, headers, request).await
}

async fn request(State(state): State<ApiState>, headers: HeaderMap, body: Bytes) -> Response {
    let request = match ExternalRequest::decode(std::str::from_utf8(&body).unwrap_or_default()) {
        Ok(request) => request,
        Err(error) => {
            let request_id = serde_json::from_slice::<Value>(&body)
                .ok()
                .and_then(|value| {
                    value
                        .get("request_id")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                })
                .unwrap_or_else(|| "unknown".into());
            return response_for(
                StatusCode::BAD_REQUEST,
                ExternalResponse::failure(request_id, error),
            );
        }
    };
    handle_request(state, headers, request).await
}

async fn handle_request(state: ApiState, headers: HeaderMap, request: ExternalRequest) -> Response {
    let raw_token = match bearer(&headers) {
        Some(token) => token,
        None => {
            if state
                .service
                .lock()
                .await
                .record_auth_failure(&request, "unauthenticated", ExternalErrorCode::Unauthorized)
                .is_err()
            {
                return audit_unavailable(&request);
            }
            return response_for(
                StatusCode::UNAUTHORIZED,
                ExternalResponse::failure(
                    request.request_id.clone(),
                    ExternalError {
                        code: ExternalErrorCode::Unauthorized,
                        message: "a bearer token is required".into(),
                        required_scope: None,
                        retryable: false,
                    },
                ),
            );
        }
    };
    let required = request.command.required_scope();
    let summary = match state.tokens.authenticate_current(raw_token, required) {
        Ok(summary) => summary,
        Err(error) => {
            let (code, required_scope) = crate::tokens::auth_status(&error);
            let status = match error {
                TokenAuthError::Unauthorized => StatusCode::UNAUTHORIZED,
                TokenAuthError::Forbidden { .. } => StatusCode::FORBIDDEN,
                TokenAuthError::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
            };
            let principal = match &error {
                TokenAuthError::Unauthorized | TokenAuthError::Unavailable => "unauthenticated",
                TokenAuthError::Forbidden { id, .. } => id.as_str(),
            };
            if state
                .service
                .lock()
                .await
                .record_auth_failure(&request, principal, code)
                .is_err()
            {
                return audit_unavailable(&request);
            }
            return response_for(
                status,
                ExternalResponse::failure(
                    request.request_id.clone(),
                    ExternalError {
                        code,
                        message: if code == ExternalErrorCode::Unauthorized {
                            "token is missing, revoked, or invalid".into()
                        } else if code == ExternalErrorCode::Unavailable {
                            "token store is unavailable".into()
                        } else {
                            "token does not grant the required scope".into()
                        },
                        required_scope,
                        retryable: false,
                    },
                ),
            );
        }
    };
    let scopes: BTreeSet<ExternalScope> = summary.scopes.iter().copied().collect();
    let response = state
        .service
        .lock()
        .await
        .dispatch_as(&request, &ExternalGrant::http(summary.id, scopes));
    let status = status_for(&response);
    response_for(status, response)
}

fn audit_unavailable(request: &ExternalRequest) -> Response {
    response_for(
        StatusCode::SERVICE_UNAVAILABLE,
        ExternalResponse::failure(
            request.request_id.clone(),
            ExternalError {
                code: ExternalErrorCode::Unavailable,
                message: "external audit is unavailable".into(),
                required_scope: None,
                retryable: false,
            },
        ),
    )
}
fn bearer(headers: &HeaderMap) -> Option<&str> {
    let value = headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?;
    let (scheme, token) = value.split_once(' ')?;
    (scheme.eq_ignore_ascii_case("bearer") && !token.is_empty()).then_some(token)
}

fn status_for(response: &ExternalResponse) -> StatusCode {
    if response.ok {
        return StatusCode::OK;
    }
    match response.error.as_ref().map(|error| error.code) {
        Some(ExternalErrorCode::Unauthorized) => StatusCode::UNAUTHORIZED,
        Some(ExternalErrorCode::Forbidden | ExternalErrorCode::WriteProposalOnly) => {
            StatusCode::FORBIDDEN
        }
        Some(ExternalErrorCode::NotFound) => StatusCode::NOT_FOUND,
        Some(ExternalErrorCode::Conflict) => StatusCode::CONFLICT,
        Some(ExternalErrorCode::Unavailable) => StatusCode::SERVICE_UNAVAILABLE,
        Some(ExternalErrorCode::InvalidRequest | ExternalErrorCode::UnsupportedProtocol) => {
            StatusCode::BAD_REQUEST
        }
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

fn response_for(status: StatusCode, response: ExternalResponse) -> Response {
    let body = serde_json::to_value(response).unwrap_or_else(|_| json!({"ok":false}));
    (status, Json(body)).into_response()
}
