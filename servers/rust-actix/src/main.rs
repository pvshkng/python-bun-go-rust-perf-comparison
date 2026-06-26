use actix_web::{web, App, HttpResponse, HttpServer, Responder};
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::json;
use sqlx::{postgres::PgPoolOptions, PgPool, Row};
use std::env;

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

async fn chat(st: web::Data<AppState>, body: web::Bytes) -> impl Responder {
    let req: ChatReq = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(_) => return HttpResponse::BadRequest().finish(),
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
        _ => return HttpResponse::BadGateway().finish(),
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
                    yield Ok::<bytes::Bytes, actix_web::Error>(chunk);
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

    let mut resp = HttpResponse::Ok();
    resp.content_type("text/event-stream");
    resp.insert_header(("Cache-Control", "no-cache"));
    if let Some(t) = &thread_id {
        resp.insert_header(("X-Thread-Id", t.clone()));
    }
    resp.streaming(body_stream)
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
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

    println!("rust-actix server listening on :8080");
    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(AppState {
                client: reqwest::Client::new(),
                stub: stub.clone(),
                db: db.clone(),
            }))
            .route("/chat", web::post().to(chat))
    })
    .tcp_nodelay(true)
    .bind(("0.0.0.0", 8080))?
    .run()
    .await
}
