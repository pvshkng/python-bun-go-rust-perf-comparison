package main

import (
	"encoding/json"
	"fmt"
	"log"
	"net/http"
	"os"
	"strconv"
	"time"
)

func getenv(key, fallback string) string {
	if v := os.Getenv(key); v != "" {
		return v
	}
	return fallback
}

func main() {
	tokenCount, _ := strconv.Atoi(getenv("TOKEN_COUNT", "20"))
	tokenDelayMs, _ := strconv.Atoi(getenv("TOKEN_DELAY_MS", "50"))

	http.HandleFunc("/v1/chat/completions", func(w http.ResponseWriter, r *http.Request) {
		flusher, ok := w.(http.Flusher)
		if !ok {
			http.Error(w, "", http.StatusInternalServerError)
			return
		}

		w.Header().Set("Content-Type", "text/event-stream")
		w.Header().Set("Cache-Control", "no-cache")
		w.Header().Set("Connection", "keep-alive")

		for i := 0; i < tokenCount; i++ {
			chunk := map[string]any{
				"choices": []map[string]any{
					{
						"delta": map[string]string{"content": fmt.Sprintf("token%d ", i)},
						"index": 0,
					},
				},
			}
			data, _ := json.Marshal(chunk)
			fmt.Fprintf(w, "data: %s\n\n", data)
			flusher.Flush()
			time.Sleep(time.Duration(tokenDelayMs) * time.Millisecond)
		}

		fmt.Fprint(w, "data: [DONE]\n\n")
		flusher.Flush()
	})

	port := getenv("PORT", "9090")
	log.Printf("stub listening on :%s", port)
	if err := http.ListenAndServe(":"+port, nil); err != nil {
		panic(err)
	}
}
