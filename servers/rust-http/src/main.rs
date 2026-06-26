use serde::Deserialize;
use serde_json::json;
use std::env;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;

type Pool = r2d2::Pool<r2d2_postgres::PostgresConnectionManager<r2d2_postgres::postgres::NoTls>>;

struct Upstream {
    host: String,
    port: u16,
    path: String,
}

struct App {
    stub: Upstream,
    db: Option<Pool>,
}

#[derive(Deserialize)]
struct ChatReq {
    message: String,
    thread_id: Option<String>,
}

fn parse_url(u: &str) -> Upstream {
    let s = u.strip_prefix("http://").unwrap_or(u);
    let (hostport, path) = match s.find('/') {
        Some(i) => (&s[..i], &s[i..]),
        None => (s, "/"),
    };
    let (host, port) = match hostport.rsplit_once(':') {
        Some((h, p)) => (h.to_string(), p.parse().unwrap_or(80)),
        None => (hostport.to_string(), 80),
    };
    Upstream {
        host,
        port,
        path: path.to_string(),
    }
}

fn main() {
    let use_db = env::args().any(|a| a == "--db");
    let stub = parse_url(&env::var("STUB_URL").expect("STUB_URL not set"));

    let db = if use_db {
        let cfg = env::var("DATABASE_URL")
            .expect("DATABASE_URL not set")
            .parse()
            .expect("parse DATABASE_URL");
        let manager =
            r2d2_postgres::PostgresConnectionManager::new(cfg, r2d2_postgres::postgres::NoTls);
        Some(r2d2::Pool::builder().max_size(20).build(manager).unwrap())
    } else {
        None
    };

    let app = Arc::new(App { stub, db });
    let listener = TcpListener::bind("0.0.0.0:8080").unwrap();
    println!("rust-http server listening on :8080");

    for stream in listener.incoming().flatten() {
        let app = app.clone();
        thread::spawn(move || {
            let _ = handle_conn(stream, app);
        });
    }
}

fn handle_conn(stream: TcpStream, app: Arc<App>) -> std::io::Result<()> {
    stream.set_nodelay(true).ok();
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut writer = stream;

    loop {
        let mut request_line = String::new();
        if reader.read_line(&mut request_line)? == 0 {
            return Ok(());
        }
        let mut parts = request_line.split_whitespace();
        let method = parts.next().unwrap_or("").to_string();
        let path = parts.next().unwrap_or("").to_string();

        let mut content_length = 0usize;
        let mut keep_alive = true;
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line)? == 0 {
                return Ok(());
            }
            if line == "\r\n" || line == "\n" {
                break;
            }
            let lower = line.to_ascii_lowercase();
            if let Some(v) = lower.strip_prefix("content-length:") {
                content_length = v.trim().parse().unwrap_or(0);
            } else if let Some(v) = lower.strip_prefix("connection:") {
                if v.trim() == "close" {
                    keep_alive = false;
                }
            }
        }

        let mut body = vec![0u8; content_length];
        reader.read_exact(&mut body)?;

        if method == "POST" && path == "/chat" {
            handle_chat(&mut writer, &app, &body)?;
        } else {
            write_simple(&mut writer, 404, "Not Found")?;
        }

        if !keep_alive {
            return Ok(());
        }
    }
}

fn handle_chat(writer: &mut TcpStream, app: &App, body: &[u8]) -> std::io::Result<()> {
    let req: ChatReq = match serde_json::from_slice(body) {
        Ok(r) => r,
        Err(_) => return write_simple(writer, 400, "Bad Request"),
    };

    let mut thread_id: Option<String> = None;
    let messages: Vec<serde_json::Value>;

    if let Some(pool) = &app.db {
        let mut client = pool.get().unwrap();
        let tid = match &req.thread_id {
            Some(t) => t.clone(),
            None => {
                let row = client
                    .query_one("INSERT INTO threads DEFAULT VALUES RETURNING id::text", &[])
                    .unwrap();
                row.get::<_, String>(0)
            }
        };
        client
            .execute(
                "INSERT INTO messages (thread_id, role, content) VALUES ($1::uuid, 'user', $2)",
                &[&tid, &req.message],
            )
            .unwrap();
        let rows = client
            .query(
                "SELECT role, content FROM messages WHERE thread_id = $1::uuid ORDER BY created_at",
                &[&tid],
            )
            .unwrap();
        messages = rows
            .iter()
            .map(|r| json!({"role": r.get::<_, String>(0), "content": r.get::<_, String>(1)}))
            .collect();
        thread_id = Some(tid);
    } else {
        messages = vec![json!({"role": "user", "content": req.message})];
    }

    let payload = serde_json::to_vec(&json!({"messages": messages, "stream": true})).unwrap();

    let up = TcpStream::connect((app.stub.host.as_str(), app.stub.port))?;
    up.set_nodelay(true).ok();
    let mut up_writer = up.try_clone()?;
    let head = format!(
        "POST {} HTTP/1.1\r\nHost: {}:{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        app.stub.path,
        app.stub.host,
        app.stub.port,
        payload.len()
    );
    up_writer.write_all(head.as_bytes())?;
    up_writer.write_all(&payload)?;
    up_writer.flush()?;

    let mut up_reader = BufReader::new(up);
    let mut status_line = String::new();
    up_reader.read_line(&mut status_line)?;
    let ok = status_line.contains(" 200 ");

    let mut chunked = false;
    loop {
        let mut line = String::new();
        if up_reader.read_line(&mut line)? == 0 {
            break;
        }
        if line == "\r\n" || line == "\n" {
            break;
        }
        let lower = line.to_ascii_lowercase();
        if lower.starts_with("transfer-encoding:") && lower.contains("chunked") {
            chunked = true;
        }
    }

    if !ok {
        return write_simple(writer, 502, "Bad Gateway");
    }

    let mut resp_head = String::from(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nTransfer-Encoding: chunked\r\n",
    );
    if let Some(t) = &thread_id {
        resp_head.push_str(&format!("X-Thread-Id: {}\r\n", t));
    }
    resp_head.push_str("\r\n");
    writer.write_all(resp_head.as_bytes())?;
    writer.flush()?;

    let mut full = String::new();
    if chunked {
        forward_chunked(&mut up_reader, writer, &mut full)?;
    } else {
        let mut buf = [0u8; 4096];
        loop {
            let n = up_reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            full.push_str(&String::from_utf8_lossy(&buf[..n]));
            write_chunk(writer, &buf[..n])?;
        }
    }
    writer.write_all(b"0\r\n\r\n")?;
    writer.flush()?;

    if let Some(pool) = &app.db {
        if let Some(tid) = &thread_id {
            if let Ok(mut client) = pool.get() {
                let _ = client.execute(
                    "INSERT INTO messages (thread_id, role, content) VALUES ($1::uuid, 'assistant', $2)",
                    &[tid, &full],
                );
            }
        }
    }
    Ok(())
}

fn forward_chunked(
    up: &mut BufReader<TcpStream>,
    writer: &mut TcpStream,
    full: &mut String,
) -> std::io::Result<()> {
    loop {
        let mut size_line = String::new();
        if up.read_line(&mut size_line)? == 0 {
            break;
        }
        let size = usize::from_str_radix(
            size_line.split(';').next().unwrap_or("0").trim(),
            16,
        )
        .unwrap_or(0);
        if size == 0 {
            let mut trailer = String::new();
            up.read_line(&mut trailer)?;
            break;
        }
        let mut data = vec![0u8; size];
        up.read_exact(&mut data)?;
        let mut crlf = [0u8; 2];
        up.read_exact(&mut crlf)?;
        full.push_str(&String::from_utf8_lossy(&data));
        write_chunk(writer, &data)?;
    }
    Ok(())
}

fn write_chunk(writer: &mut TcpStream, data: &[u8]) -> std::io::Result<()> {
    writer.write_all(format!("{:x}\r\n", data.len()).as_bytes())?;
    writer.write_all(data)?;
    writer.write_all(b"\r\n")?;
    writer.flush()
}

fn write_simple(writer: &mut TcpStream, code: u16, reason: &str) -> std::io::Result<()> {
    let head = format!(
        "HTTP/1.1 {} {}\r\nContent-Length: 0\r\n\r\n",
        code, reason
    );
    writer.write_all(head.as_bytes())?;
    writer.flush()
}
