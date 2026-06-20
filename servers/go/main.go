package main

import (
	"bytes"
	"context"
	"encoding/json"
	"flag"
	"io"
	"net/http"
	"os"

	"github.com/jackc/pgx/v5/pgxpool"
)

type server struct {
	db     *pgxpool.Pool
	stub   string
	client *http.Client
}

type chatReq struct {
	Message  string  `json:"message"`
	ThreadID *string `json:"thread_id"`
}

type message struct {
	Role    string `json:"role"`
	Content string `json:"content"`
}

func main() {
	useDB := flag.Bool("db", false, "connect to postgres")
	flag.Parse()

	s := &server{
		stub:   os.Getenv("STUB_URL"),
		client: &http.Client{},
	}

	if *useDB {
		pool, err := pgxpool.New(context.Background(), os.Getenv("DATABASE_URL"))
		if err != nil {
			panic(err)
		}
		s.db = pool
	}

	http.HandleFunc("/chat", s.chat)
	if err := http.ListenAndServe(":8080", nil); err != nil {
		panic(err)
	}
}

func (s *server) chat(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		w.WriteHeader(http.StatusMethodNotAllowed)
		return
	}

	flusher, ok := w.(http.Flusher)
	if !ok {
		w.WriteHeader(http.StatusInternalServerError)
		return
	}

	var req chatReq
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		w.WriteHeader(http.StatusBadRequest)
		return
	}

	ctx := r.Context()

	var msgs []message
	var threadID string

	if s.db != nil {
		if req.ThreadID == nil {
			row := s.db.QueryRow(ctx, "INSERT INTO threads DEFAULT VALUES RETURNING id::text")
			if err := row.Scan(&threadID); err != nil {
				w.WriteHeader(http.StatusInternalServerError)
				return
			}
		} else {
			threadID = *req.ThreadID
		}

		if _, err := s.db.Exec(ctx,
			"INSERT INTO messages (thread_id, role, content) VALUES ($1::uuid, 'user', $2)",
			threadID, req.Message,
		); err != nil {
			w.WriteHeader(http.StatusInternalServerError)
			return
		}

		rows, err := s.db.Query(ctx,
			"SELECT role, content FROM messages WHERE thread_id = $1::uuid ORDER BY created_at",
			threadID,
		)
		if err != nil {
			w.WriteHeader(http.StatusInternalServerError)
			return
		}
		for rows.Next() {
			var m message
			if err := rows.Scan(&m.Role, &m.Content); err != nil {
				w.WriteHeader(http.StatusInternalServerError)
				return
			}
			msgs = append(msgs, m)
		}
		rows.Close()
	} else {
		msgs = []message{{Role: "user", Content: req.Message}}
	}

	body, _ := json.Marshal(map[string]any{"messages": msgs})
	upReq, _ := http.NewRequestWithContext(ctx, http.MethodPost, s.stub, bytes.NewReader(body))
	upReq.Header.Set("Content-Type", "application/json")

	resp, err := s.client.Do(upReq)
	if err != nil {
		w.WriteHeader(http.StatusBadGateway)
		return
	}
	defer resp.Body.Close()

	w.Header().Set("Content-Type", "text/event-stream")
	w.Header().Set("Cache-Control", "no-cache")
	if threadID != "" {
		w.Header().Set("X-Thread-Id", threadID)
	}
	w.WriteHeader(http.StatusOK)

	var captured bytes.Buffer
	tee := io.TeeReader(resp.Body, &captured)
	buf := make([]byte, 4096)
	for {
		n, err := tee.Read(buf)
		if n > 0 {
			w.Write(buf[:n])
			flusher.Flush()
		}
		if err != nil {
			break
		}
	}

	if s.db != nil {
		s.db.Exec(context.Background(),
			"INSERT INTO messages (thread_id, role, content) VALUES ($1::uuid, 'assistant', $2)",
			threadID, captured.String(),
		)
	}
}
