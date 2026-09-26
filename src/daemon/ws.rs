//! 桥接 WS 端口：接受 WebSocket 握手，首条文本消息（hello 占位）回 `hello-ack`，
//! 之后保持连接直到对端关闭。消息协议随 change `init-app-bridge` 定稿（design.md 决策 1）。

use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Notify;
use tokio_tungstenite::accept_async;
use tokio_tungstenite::tungstenite::Message;

/// 在已绑定的 listener 上接受 WS 连接；`shutdown` 被通知后停止接受新连接。
pub async fn serve(listener: TcpListener, shutdown: Arc<Notify>) -> std::io::Result<()> {
    loop {
        tokio::select! {
            _ = shutdown.notified() => return Ok(()),
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                tokio::spawn(handle_connection(stream));
            }
        }
    }
}

/// 单连接处理：握手 → 首条文本回 hello-ack → 保持直到关闭。
async fn handle_connection(stream: TcpStream) {
    let mut ws = match accept_async(stream).await {
        Ok(ws) => ws,
        Err(_) => return, // 握手失败直接断开
    };
    let mut greeted = false;
    while let Some(msg) = ws.next().await {
        match msg {
            Ok(Message::Text(_)) if !greeted => {
                greeted = true;
                let ack = serde_json::json!({ "ok": true, "type": "hello-ack" }).to_string();
                if ws.send(Message::Text(ack)).await.is_err() {
                    return;
                }
            }
            Ok(Message::Close(_)) => return,
            Ok(_) => {} // 其余消息占位忽略（协议随 init-app-bridge 定稿）
            Err(_) => return,
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use tokio_tungstenite::connect_async;

    #[tokio::test]
    async fn handshake_hello_ack_and_keepalive() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("绑定随机端口");
        let port = listener.local_addr().expect("读取本地地址").port();
        let shutdown = Arc::new(Notify::new());
        let server = tokio::spawn(serve(listener, Arc::clone(&shutdown)));

        let url = format!("ws://127.0.0.1:{port}");
        let (mut client, _) = connect_async(&url).await.expect("WS 握手");
        client
            .send(Message::Text(r#"{"type":"hello"}"#.to_string()))
            .await
            .expect("发送 hello");
        let msg = client.next().await.expect("收取 ack").expect("ack 有效");
        let Message::Text(text) = msg else {
            panic!("应收到文本消息: {msg:?}")
        };
        let v: Value = serde_json::from_str(&text).expect("解析 hello-ack");
        assert_eq!(v["ok"], true);
        assert_eq!(v["type"], "hello-ack");

        // 连接保持：关闭客户端后服务端仍能接受下一条连接
        drop(client);
        let (mut client2, _) = connect_async(&url).await.expect("第二次 WS 握手");
        client2
            .send(Message::Text(r#"{"type":"hello"}"#.to_string()))
            .await
            .expect("发送第二条 hello");
        let msg2 = client2
            .next()
            .await
            .expect("收取第二个 ack")
            .expect("ack 有效");
        assert!(matches!(msg2, Message::Text(_)), "第二条连接也应收到 ack");
        drop(client2);

        shutdown.notify_one();
        server.await.expect("join ws 服务").expect("ws 服务退出");
    }
}
