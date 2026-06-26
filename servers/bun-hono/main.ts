import { Hono } from "hono";
import postgres from "postgres";

const useDB = process.argv.includes("--db");
const sql = useDB ? postgres(process.env.DATABASE_URL!) : null;
const STUB_URL = process.env.STUB_URL!;

const app = new Hono();

app.post("/chat", async (c) => {
  const { message, thread_id: inputThreadId } = await c.req.json();

  let threadId: string | null = null;
  let messages: { role: string; content: string }[];

  if (useDB && sql) {
    if (!inputThreadId) {
      const [row] = await sql`INSERT INTO threads DEFAULT VALUES RETURNING id::text`;
      threadId = row.id;
    } else {
      threadId = inputThreadId;
    }

    await sql`INSERT INTO messages (thread_id, role, content) VALUES (${threadId}::uuid, 'user', ${message})`;

    messages = await sql`
      SELECT role, content FROM messages
      WHERE thread_id = ${threadId}::uuid
      ORDER BY created_at
    `;
  } else {
    messages = [{ role: "user", content: message }];
  }

  const upstream = await fetch(STUB_URL, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ messages, stream: true }),
  });

  const reader = upstream.body!.getReader();
  const decoder = new TextDecoder();
  let fullText = "";

  const stream = new ReadableStream({
    async pull(controller) {
      const { done, value } = await reader.read();
      if (done) {
        if (useDB && sql && threadId) {
          sql`INSERT INTO messages (thread_id, role, content) VALUES (${threadId}::uuid, 'assistant', ${fullText})`.catch(
            () => {}
          );
        }
        controller.close();
        return;
      }
      fullText += decoder.decode(value, { stream: true });
      controller.enqueue(value);
    },
  });

  const headers: Record<string, string> = {
    "Content-Type": "text/event-stream",
    "Cache-Control": "no-cache",
  };
  if (threadId) headers["X-Thread-Id"] = threadId;

  return new Response(stream, { headers });
});

console.log("bun-hono server listening on :8080");
console.log(`targetting STUB_URL at :${STUB_URL}`);

export default { port: 8080, hostname: "0.0.0.0", fetch: app.fetch };
