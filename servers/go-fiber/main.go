package main

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"flag"
	"io"
	"log"
	"net/http"
	"os"

	"github.com/gofiber/fiber/v2"
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

	app := fiber.New(fiber.Config{DisableStartupMessage: true})
	app.Post("/chat", s.chat)

	log.Print("go-fiber server listening on :8080")
	log.Printf("targetting STUB_URL at :%s", s.stub)
	if err := app.Listen(":8080"); err != nil {
		panic(err)
	}
}

func (s *server) chat(c *fiber.Ctx) error {
	var req chatReq
	if err := c.BodyParser(&req); err != nil {
		return c.SendStatus(http.StatusBadRequest)
	}

	ctx := context.Background()

	var msgs []message
	var threadID string

	if s.db != nil {
		if req.ThreadID == nil {
			row := s.db.QueryRow(ctx, "INSERT INTO threads DEFAULT VALUES RETURNING id::text")
			if err := row.Scan(&threadID); err != nil {
				log.Printf("insert thread failed: %v", err)
				return c.SendStatus(http.StatusInternalServerError)
			}
		} else {
			threadID = *req.ThreadID
		}

		if _, err := s.db.Exec(ctx,
			"INSERT INTO messages (thread_id, role, content) VALUES ($1::uuid, 'user', $2)",
			threadID, req.Message,
		); err != nil {
			log.Printf("insert user message failed: %v", err)
			return c.SendStatus(http.StatusInternalServerError)
		}

		rows, err := s.db.Query(ctx,
			"SELECT role, content FROM messages WHERE thread_id = $1::uuid ORDER BY created_at",
			threadID,
		)
		if err != nil {
			log.Printf("query messages failed: %v", err)
			return c.SendStatus(http.StatusInternalServerError)
		}
		for rows.Next() {
			var m message
			if err := rows.Scan(&m.Role, &m.Content); err != nil {
				log.Printf("scan message failed: %v", err)
				rows.Close()
				return c.SendStatus(http.StatusInternalServerError)
			}
			msgs = append(msgs, m)
		}
		rows.Close()
	} else {
		msgs = []message{{Role: "user", Content: req.Message}}
	}

	body, _ := json.Marshal(map[string]any{"messages": msgs, "stream": true})
	upReq, err := http.NewRequest(http.MethodPost, s.stub, bytes.NewReader(body))
	if err != nil {
		log.Printf("stub request build failed: %v", err)
		return c.SendStatus(http.StatusInternalServerError)
	}
	upReq.Header.Set("Content-Type", "application/json")

	resp, err := s.client.Do(upReq)
	if err != nil {
		log.Printf("stub request to %q failed: %v", s.stub, err)
		return c.SendStatus(http.StatusBadGateway)
	}

	if resp.StatusCode != http.StatusOK {
		resp.Body.Close()
		return c.SendStatus(resp.StatusCode)
	}

	c.Set("Content-Type", "text/event-stream")
	c.Set("Cache-Control", "no-cache")
	if threadID != "" {
		c.Set("X-Thread-Id", threadID)
	}

	c.Context().SetBodyStreamWriter(func(w *bufio.Writer) {
		defer resp.Body.Close()
		var captured bytes.Buffer
		tee := io.TeeReader(resp.Body, &captured)
		buf := make([]byte, 4096)
		for {
			n, err := tee.Read(buf)
			if n > 0 {
				w.Write(buf[:n])
				w.Flush()
			}
			if err != nil {
				break
			}
		}

		if s.db != nil {
			if _, err := s.db.Exec(context.Background(),
				"INSERT INTO messages (thread_id, role, content) VALUES ($1::uuid, 'assistant', $2)",
				threadID, captured.String(),
			); err != nil {
				log.Printf("insert assistant message failed: %v", err)
			}
		}
	})

	return nil
}
