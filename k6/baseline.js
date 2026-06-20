import http from "k6/http";
import { check } from "k6";

export const options = {
  vus: 10,
  duration: "60s",
};

let threadId = null;

export default function () {
  const res = http.post(
    "http://localhost:8080/chat",
    JSON.stringify({ thread_id: threadId, message: "hello" }),
    { headers: { "Content-Type": "application/json" } }
  );

  check(res, {
    "status 200": (r) => r.status === 200,
    "has DONE": (r) => r.body.includes("[DONE]"),
  });

  threadId = res.headers["X-Thread-Id"] ?? threadId;
}
