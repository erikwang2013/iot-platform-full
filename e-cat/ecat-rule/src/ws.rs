use crate::push::{ConnectionGuard, PushHub};
use axum::{
    Json,
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::StatusCode,
    response::{IntoResponse, Response},
};
use tokio::sync::broadcast;

/// GET /ws —— 直连端点（网关不代理 WebSocket，P5 前端直连 8084）。
/// 浏览器 WS 无法携带 Authorization header，JWT 经 Sec-WebSocket-Protocol
/// 子协议传递（`bearer, <token>`），避免 token 进入 URL / 访问日志；
/// 响应只回选 `bearer`，绝不回显 token。
/// JWT 校验收编至框架 ecat_auth::verify_token（与网关同一 JWT_SECRET），sub 即租户；失败 401。
pub async fn ws_handler(State(hub): State<PushHub>, ws: WebSocketUpgrade) -> Response {
    // axum 以 BTreeSet 保存子协议（有序集合，非请求顺序），不能依赖 "bearer" 之后的位置；
    // 约定 bearer 之外的那个值即 token。
    let protocols: Vec<String> = ws
        .requested_protocols()
        .filter_map(|p| p.to_str().ok().map(str::to_owned))
        .collect();
    let token = protocols
        .iter()
        .find(|p| p.as_str() != "bearer")
        .filter(|_| protocols.iter().any(|p| p == "bearer"))
        .cloned();

    let tenant = match token
        .ok_or_else(|| "missing bearer subprotocol".to_string())
        .and_then(|token| {
            let secret = std::env::var("JWT_SECRET").map_err(|_| "server error".to_string())?;
            ecat_auth::verify_token(&token, &secret)
        }) {
        Ok(c) => c.sub.clone(),
        Err(e) => {
            // 细节只进日志，不回给客户端（防内部实现/签名细节泄露）
            tracing::warn!(error = %e, "ws jwt validation failed");
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({"error": "invalid token"})),
            )
                .into_response();
        }
    };
    // 连接名额（全局 + 单租户）：随连接存活，断开即归还。
    let Some(guard) = hub.try_connection(&tenant) else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "too many connections"})),
        )
            .into_response();
    };
    let rx = hub.subscribe(&tenant);
    ws.protocols(["bearer"])
        .on_upgrade(move |socket| async move {
            on_socket(socket, rx, guard).await;
            // rx 已随 on_socket 结束而 drop；此处清理无订阅者的 sender 条目。
            hub.release(&tenant);
        })
}

/// 双向读取：推送告警，同时消费客户端帧（Close/Ping），否则对端关闭无法感知。
async fn on_socket(
    mut socket: WebSocket,
    mut rx: broadcast::Receiver<crate::models::AlertMessage>,
    _connection: ConnectionGuard,
) {
    loop {
        tokio::select! {
            broadcast = rx.recv() => match broadcast {
                Ok(msg) => {
                    let text = serde_json::to_string(&msg).unwrap_or_default();
                    if socket.send(Message::Text(text.into())).await.is_err() {
                        break;
                    }
                }
                // 慢消费者丢最旧消息但仍保持连接（broadcast 语义）
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            },
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Ping(payload))) => {
                    if socket.send(Message::Pong(payload)).await.is_err() {
                        break;
                    }
                }
                // Close / 传输错误 / 对端关闭
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                // 本端点只推送；客户端 Text/Binary/Pong 一律忽略
                Some(Ok(_)) => {}
            },
        }
    }
}
