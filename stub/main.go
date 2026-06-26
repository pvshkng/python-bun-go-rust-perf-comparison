package main

import (
	"encoding/json"
	"fmt"
	"log"
	"net/http"
	"os"
	"regexp"
	"strconv"
	"strings"
	"time"
)

type chatReq struct {
	Messages []message `json:"messages"`
	Stream   *bool     `json:"stream"`
}

type message struct {
	Role    string `json:"role"`
	Content string `json:"content"`
}

var paragraphs []string

var countWords = map[string]int{
	"a": 1, "an": 1, "one": 1, "two": 2, "three": 3, "four": 4, "five": 5,
	"six": 6, "seven": 7, "eight": 8, "nine": 9, "ten": 10, "eleven": 11,
	"twelve": 12, "thirteen": 13, "fourteen": 14, "fifteen": 15, "sixteen": 16,
	"seventeen": 17, "eighteen": 18, "nineteen": 19, "twenty": 20,
}

var digitRe = regexp.MustCompile(`(\d+)\s+paragraph`)
var wordRe = regexp.MustCompile(`([a-z]+)\s+paragraph`)

func getenv(key, fallback string) string {
	if v := os.Getenv(key); v != "" {
		return v
	}
	return fallback
}

func loadParagraphs(path string) {
	raw, err := os.ReadFile(path)
	if err != nil {
		log.Fatalf("read lorem data %q: %v", path, err)
	}
	var doc struct {
		Data []string `json:"data"`
	}
	if err := json.Unmarshal(raw, &doc); err != nil {
		log.Fatalf("parse lorem data: %v", err)
	}
	if len(doc.Data) == 0 {
		log.Fatal("lorem data is empty")
	}
	paragraphs = doc.Data
}

func lastUserMessage(msgs []message) string {
	for i := len(msgs) - 1; i >= 0; i-- {
		if msgs[i].Role == "user" {
			return msgs[i].Content
		}
	}
	if len(msgs) > 0 {
		return msgs[len(msgs)-1].Content
	}
	return ""
}

func paragraphCount(text string) int {
	t := strings.ToLower(text)
	if !strings.Contains(t, "paragraph") {
		return 1
	}
	if m := digitRe.FindStringSubmatch(t); m != nil {
		if n, err := strconv.Atoi(m[1]); err == nil && n > 0 {
			return n
		}
	}
	if m := wordRe.FindStringSubmatch(t); m != nil {
		if n, ok := countWords[m[1]]; ok {
			return n
		}
	}
	return 1
}

func selected(text string) []string {
	n := paragraphCount(text)
	if n > len(paragraphs) {
		n = len(paragraphs)
	}
	if n < 1 {
		n = 1
	}
	return paragraphs[:n]
}

func main() {
	tokenDelayMs, _ := strconv.Atoi(getenv("TOKEN_DELAY_MS", "0"))
	loadParagraphs(getenv("LOREM_PATH", "data/lorem_ipsum.json"))

	http.HandleFunc("/v1/chat/completions", func(w http.ResponseWriter, r *http.Request) {
		var req chatReq
		if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
			http.Error(w, "bad request", http.StatusBadRequest)
			return
		}

		paras := selected(lastUserMessage(req.Messages))
		stream := req.Stream == nil || *req.Stream

		if !stream {
			content := strings.Join(paras, "\n\n")
			w.Header().Set("Content-Type", "application/json")
			json.NewEncoder(w).Encode(map[string]any{
				"choices": []map[string]any{{
					"index":         0,
					"message":       map[string]string{"role": "assistant", "content": content},
					"finish_reason": "stop",
				}},
			})
			return
		}

		flusher, ok := w.(http.Flusher)
		if !ok {
			http.Error(w, "", http.StatusInternalServerError)
			return
		}
		w.Header().Set("Content-Type", "text/event-stream")
		w.Header().Set("Cache-Control", "no-cache")
		w.Header().Set("Connection", "keep-alive")

		delay := time.Duration(tokenDelayMs) * time.Millisecond
		emit := func(s string) {
			chunk := map[string]any{
				"choices": []map[string]any{{
					"index": 0,
					"delta": map[string]string{"content": s},
				}},
			}
			data, _ := json.Marshal(chunk)
			fmt.Fprintf(w, "data: %s\n\n", data)
			flusher.Flush()
			if delay > 0 {
				time.Sleep(delay)
			}
		}

		for pi, para := range paras {
			if pi > 0 {
				emit("\n\n")
			}
			for _, word := range strings.Fields(para) {
				emit(word + " ")
			}
		}
		fmt.Fprint(w, "data: [DONE]\n\n")
		flusher.Flush()
	})

	port := getenv("PORT", "9090")
	log.Printf("stub listening on :%s (%d paragraphs loaded)", port, len(paragraphs))
	if err := http.ListenAndServe(":"+port, nil); err != nil {
		panic(err)
	}
}
