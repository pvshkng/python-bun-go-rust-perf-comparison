import http from "k6/http";
import { check } from "k6";

export const options = {
  vus: 50,
  duration: "10m",
};

let threadId = null;

export default function () {
  const res = http.post(
    "http://localhost:8080/chat",
    JSON.stringify({ thread_id: threadId, message: "hello" }),
    { headers: { "Content-Type": "application/json" }, timeout: "30s" }
  );

  check(res, {
    "status 200": (r) => r.status === 200,
    "has DONE": (r) => r.body.includes("[DONE]"),
  });

  threadId = res.headers["X-Thread-Id"] ?? threadId;
}
