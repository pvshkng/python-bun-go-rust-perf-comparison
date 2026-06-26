use axum::{
    body::{Body, Bytes},
    extract::State,
    http::{header, StatusCode},
    response::Response,
    routing::post,
    Router,
};
use futures_util::StreamExt;
use hyper::server::conn::http1;
use hyper_util::rt::TokioIo;
use tower::Service;
use serde::Deserialize;
use serde_json::json;
use sqlx::{postgres::PgPoolOptions, PgPool, Row};
use std::env;

#[derive(Clone)]
struct AppState {
    client: reqwest::Client,
    stub: String,
    db: Option<PgPool>,
}

#[derive(Deserialize)]
struct ChatReq {
    message: String,
    thread_id: Option<String>,
}

#[tokio::main]
async fn main() {
    let use_db = env::args().any(|a| a == "--db");
    let stub = env::var("STUB_URL").expect("STUB_URL not set");
    let db = if use_db {
        Some(
            PgPoolOptions::new()
                .max_connections(20)
                .connect(&env::var("DATABASE_URL").expect("DATABASE_URL not set"))
                .await
                .expect("connect postgres"),
        )
    } else {
        None
    };

    let state = AppState {
        client: reqwest::Client::new(),
        stub,
        db,
    };

    let app = Router::new().route("/chat", post(chat)).with_state(state);
    println!("rust-axum server listening on :8080");
    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await.unwrap();

    loop {
        let (stream, _) = match listener.accept().await {
            Ok(v) => v,
            Err(_) => continue,
        };
        stream.set_nodelay(true).ok();
        let io = TokioIo::new(stream);
        let app = app.clone();
        tokio::spawn(async move {
            let service = hyper::service::service_fn(move |req| app.clone().call(req));
            let _ = http1::Builder::new()
                .serve_connection(io, service)
                .await;
        });
    }
}

fn status(code: StatusCode) -> Response {
    Response::builder().status(code).body(Body::empty()).unwrap()
}

async fn chat(State(st): State<AppState>, body: Bytes) -> Response {
    let req: ChatReq = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(_) => return status(StatusCode::BAD_REQUEST),
    };

    let mut thread_id: Option<String> = None;
    let messages: Vec<serde_json::Value>;

    if let Some(pool) = &st.db {
        let tid = match &req.thread_id {
            Some(t) => t.clone(),
            None => {
                let row = sqlx::query("INSERT INTO threads DEFAULT VALUES RETURNING id::text")
                    .fetch_one(pool)
                    .await
                    .unwrap();
                row.get::<String, _>(0)
            }
        };
        sqlx::query("INSERT INTO messages (thread_id, role, content) VALUES ($1::uuid, 'user', $2)")
            .bind(&tid)
            .bind(&req.message)
            .execute(pool)
            .await
            .unwrap();
        let rows = sqlx::query(
            "SELECT role, content FROM messages WHERE thread_id = $1::uuid ORDER BY created_at",
        )
        .bind(&tid)
        .fetch_all(pool)
        .await
        .unwrap();
        messages = rows
            .iter()
            .map(|r| json!({"role": r.get::<String, _>(0), "content": r.get::<String, _>(1)}))
            .collect();
        thread_id = Some(tid);
    } else {
        messages = vec![json!({"role": "user", "content": req.message})];
    }

    let upstream = st
        .client
        .post(&st.stub)
        .json(&json!({"messages": messages, "stream": true}))
        .send()
        .await;
    let upstream = match upstream {
        Ok(r) if r.status().is_success() => r,
        _ => return status(StatusCode::BAD_GATEWAY),
    };

    let pool = st.db.clone();
    let tid_for_db = thread_id.clone();
    let mut stream = upstream.bytes_stream();
    let body_stream = async_stream::stream! {
        let mut full = String::new();
        while let Some(item) = stream.next().await {
            match item {
                Ok(chunk) => {
                    full.push_str(&String::from_utf8_lossy(&chunk));
                    yield Ok::<bytes::Bytes, std::io::Error>(chunk);
                }
                Err(_) => break,
            }
        }
        if let (Some(pool), Some(tid)) = (pool, tid_for_db) {
            let _ = sqlx::query("INSERT INTO messages (thread_id, role, content) VALUES ($1::uuid, 'assistant', $2)")
                .bind(&tid)
                .bind(&full)
                .execute(&pool)
                .await;
        }
    };

    let mut builder = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache");
    if let Some(t) = &thread_id {
        builder = builder.header("X-Thread-Id", t);
    }
    builder.body(Body::from_stream(body_stream)).unwrap()
}
